//! `MemoryPlanStore` — a pure-Rust (NO rusqlite) [`PlanStore`] over in-process
//! maps + a `Vec<UndoEntry>` journal.
//!
//! This is the Phase-1 de-risk deliverable: proof that the [`PlanStore`]
//! abstraction genuinely decouples `Session` from SQLite (the precondition for
//! the coming wasm build, where rusqlite's native C can't compile). It mirrors
//! the SQLite impl's observable semantics exactly — redo-tail truncation on
//! commit, cursor tracking, entity-row upsert/delete keyed by id, and KV/list
//! round-trips — so `Session` behaves identically on top of it. It is NOT the
//! web store (Phase 3's IndexedDB store is JS-bridged); it doubles as a fast,
//! dependency-free test store.

use std::cell::RefCell;
use std::collections::BTreeMap;

use planner_core::patch::{PatchBatch, PatchOp};
use planner_core::state::{PlanMeta, PlanState};
use planner_core::undo::UndoEntry;
use serde::{Deserialize, Serialize};

use crate::plan_file::PersistError;
use crate::store::PlanStore;

/// An entity row: its `collection` plus the stored JSON value (SQLite keys the
/// `entities` table by `id` alone and stores `collection` alongside — mirrored).
type EntityRow = (String, serde_json::Value);

/// A complete, self-describing snapshot of a [`MemoryPlanStore`] — every field
/// the store holds, so a round-trip through [`MemoryPlanStore::from_snapshot`]
/// reconstructs a byte-identical store. This is the durable unit the WEB build
/// writes to IndexedDB after every mutation (see [`PlanStore::export_snapshot`]):
/// because the undo `cursor` lives inside `meta` under `undo_cursor` and the
/// plan meta lives under `plan_meta`, capturing these five maps captures
/// EVERYTHING `Session` hydrates (canonical state, the undo journal + cursor,
/// view state, unlocked/purchased sets, advisor cards, gate, and mutes) — no
/// separate cursor/state fields are needed. `version` guards a future format
/// change (reject an unknown one rather than silently mis-deserialize a plan).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshot {
    /// Snapshot format version. Bumped only on an incompatible layout change.
    pub version: u32,
    /// id -> (collection, json value) — the entity rows.
    pub entities: BTreeMap<String, EntityRow>,
    /// The undo journal in application order.
    pub journal: Vec<UndoEntry>,
    /// The meta KV store (`plan_meta`, `undo_cursor`, `view_state`, …).
    pub meta: BTreeMap<String, String>,
    /// Advisor cards, id -> json.
    pub cards: BTreeMap<String, String>,
    /// Muted rules, rule -> muted_at.
    pub mutes: BTreeMap<String, String>,
}

/// Current [`MemorySnapshot`] format version.
const SNAPSHOT_VERSION: u32 = 1;

#[derive(Default)]
pub struct MemoryPlanStore {
    /// id -> (collection, json value). Keyed by id to match the SQLite PK.
    entities: RefCell<BTreeMap<String, EntityRow>>,
    /// The undo journal, in application order (SQLite's `undo_log` by `seq`).
    journal: RefCell<Vec<UndoEntry>>,
    /// The meta KV store (`plan_meta`, `undo_cursor`, `view_state`, …).
    meta: RefCell<BTreeMap<String, String>>,
    /// Advisor cards, id -> json.
    cards: RefCell<BTreeMap<String, String>>,
    /// Muted rules, rule -> muted_at.
    mutes: RefCell<BTreeMap<String, String>>,
    /// Injected-failure counters (tests only) — honored identically to SQLite.
    #[cfg(feature = "fault-injection")]
    faults: crate::plan_file::FaultPlan,
}

impl MemoryPlanStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture the whole store as a [`MemorySnapshot`] (the web durability
    /// unit). A deep clone of the five maps — cheap relative to the IndexedDB
    /// write it feeds, and it keeps the store live for the next mutation.
    pub fn snapshot(&self) -> MemorySnapshot {
        MemorySnapshot {
            version: SNAPSHOT_VERSION,
            entities: self.entities.borrow().clone(),
            journal: self.journal.borrow().clone(),
            meta: self.meta.borrow().clone(),
            cards: self.cards.borrow().clone(),
            mutes: self.mutes.borrow().clone(),
        }
    }

    /// Reconstruct a store from a [`MemorySnapshot`] — the inverse of
    /// [`Self::snapshot`]. A version mismatch is rejected (rather than
    /// silently mis-loading) so a forward-incompatible blob surfaces cleanly.
    pub fn from_snapshot(snapshot: MemorySnapshot) -> Result<Self, PersistError> {
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(PersistError::Corrupt(format!(
                "unsupported plan snapshot version {} (expected {SNAPSHOT_VERSION})",
                snapshot.version
            )));
        }
        Ok(Self {
            entities: RefCell::new(snapshot.entities),
            journal: RefCell::new(snapshot.journal),
            meta: RefCell::new(snapshot.meta),
            cards: RefCell::new(snapshot.cards),
            mutes: RefCell::new(snapshot.mutes),
            #[cfg(feature = "fault-injection")]
            faults: crate::plan_file::FaultPlan::default(),
        })
    }

    /// Reconstruct from the raw JSON bytes [`PlanStore::export_snapshot`]
    /// produced — the shape the web worker reads back out of IndexedDB.
    pub fn from_snapshot_bytes(bytes: &[u8]) -> Result<Self, PersistError> {
        let snapshot: MemorySnapshot = serde_json::from_slice(bytes)?;
        Self::from_snapshot(snapshot)
    }

    fn get_meta(&self, key: &str) -> Option<String> {
        self.meta.borrow().get(key).cloned()
    }

    fn set_meta(&self, key: &str, value: &str) {
        self.meta
            .borrow_mut()
            .insert(key.to_string(), value.to_string());
    }

    /// Mirror a batch of entity-level ops into rows — byte-for-byte the SQLite
    /// impl's `apply_rows`: `meta` paths are skipped (plan meta is rewritten
    /// wholesale via the KV store), a pathless op is `Corrupt`.
    fn apply_rows(&self, batch: &PatchBatch) -> Result<(), PersistError> {
        let mut entities = self.entities.borrow_mut();
        for op in batch {
            let path = op.path().trim_start_matches('/');
            let Some((collection, id)) = path.split_once('/') else {
                return Err(PersistError::Corrupt(format!("bad path {path}")));
            };
            if collection == "meta" {
                continue; // plan meta is rewritten wholesale in the KV store
            }
            match op {
                PatchOp::Add { value, .. } | PatchOp::Replace { value, .. } => {
                    entities.insert(id.to_string(), (collection.to_string(), value.clone()));
                }
                PatchOp::Remove { .. } => {
                    entities.remove(id);
                }
            }
        }
        Ok(())
    }
}

impl PlanStore for MemoryPlanStore {
    fn reset(&mut self) -> Result<(), PersistError> {
        self.entities.borrow_mut().clear();
        self.journal.borrow_mut().clear();
        self.meta.borrow_mut().clear();
        self.cards.borrow_mut().clear();
        self.mutes.borrow_mut().clear();
        Ok(())
    }

    fn load(&self) -> Result<(PlanState, Vec<UndoEntry>, usize), PersistError> {
        let mut state = PlanState::default();
        if let Some(json) = self.get_meta("plan_meta") {
            state.meta = serde_json::from_str::<PlanMeta>(&json)?;
        }
        for (id, (collection, value)) in self.entities.borrow().iter() {
            let batch = vec![PatchOp::Add {
                path: format!("/{collection}/{id}"),
                value: value.clone(),
            }];
            state.apply_batch(&batch).map_err(PersistError::Corrupt)?;
        }
        let entries: Vec<UndoEntry> = self.journal.borrow().clone();
        let cursor: usize = self
            .get_meta("undo_cursor")
            .and_then(|v| v.parse().ok())
            .unwrap_or(entries.len())
            .min(entries.len());
        Ok((state, entries, cursor))
    }

    fn commit(
        &mut self,
        entry: &UndoEntry,
        meta: &PlanMeta,
        applied: usize,
    ) -> Result<(), PersistError> {
        #[cfg(feature = "fault-injection")]
        if self.faults.fail_commits > 0 {
            self.faults.fail_commits -= 1;
            return Err(PersistError::Io(std::io::Error::other(
                "injected persist fault (commit)",
            )));
        }
        // A new commit truncates any redo tail: keep only the entries applied
        // before this one (`applied - 1`), drop the rest — same as the SQLite
        // `DELETE … WHERE seq NOT IN (… LIMIT applied - 1)`.
        self.journal
            .borrow_mut()
            .truncate(applied.saturating_sub(1));
        self.journal.borrow_mut().push(entry.clone());
        self.apply_rows(&entry.forward)?;
        self.set_meta("plan_meta", &serde_json::to_string(meta)?);
        self.set_meta("undo_cursor", &applied.to_string());
        Ok(())
    }

    fn checkpoint(
        &mut self,
        batch: &PatchBatch,
        meta: &PlanMeta,
        applied: usize,
    ) -> Result<(), PersistError> {
        #[cfg(feature = "fault-injection")]
        if self.faults.fail_checkpoints > 0 {
            self.faults.fail_checkpoints -= 1;
            return Err(PersistError::Io(std::io::Error::other(
                "injected persist fault (checkpoint)",
            )));
        }
        self.apply_rows(batch)?;
        self.set_meta("plan_meta", &serde_json::to_string(meta)?);
        self.set_meta("undo_cursor", &applied.to_string());
        Ok(())
    }

    fn set_view_state(&self, json: &str) -> Result<(), PersistError> {
        self.set_meta("view_state", json);
        Ok(())
    }

    fn view_state(&self) -> Option<String> {
        self.get_meta("view_state")
    }

    fn set_last_import(&self, json: &str) -> Result<(), PersistError> {
        self.set_meta("last_import", json);
        Ok(())
    }

    fn last_import(&self) -> Option<String> {
        self.get_meta("last_import")
    }

    fn set_unlocked(&self, json: &str) -> Result<(), PersistError> {
        self.set_meta("unlocked", json);
        Ok(())
    }

    fn unlocked(&self) -> Option<String> {
        self.get_meta("unlocked")
    }

    fn set_purchased_schematics(&self, json: &str) -> Result<(), PersistError> {
        self.set_meta("purchased_schematics", json);
        Ok(())
    }

    fn purchased_schematics(&self) -> Option<String> {
        self.get_meta("purchased_schematics")
    }

    fn save_advisor_gate(&self, json: &str) -> Result<(), PersistError> {
        self.set_meta("advisor_gate", json);
        Ok(())
    }

    fn advisor_gate(&self) -> Option<String> {
        self.get_meta("advisor_gate")
    }

    fn save_meta(&self, meta: &PlanMeta) -> Result<(), PersistError> {
        self.set_meta("plan_meta", &serde_json::to_string(meta)?);
        Ok(())
    }

    fn save_advisor_card(&self, id: &str, json: &str) -> Result<(), PersistError> {
        self.cards
            .borrow_mut()
            .insert(id.to_string(), json.to_string());
        Ok(())
    }

    fn load_advisor_cards(&self) -> Result<Vec<String>, PersistError> {
        Ok(self.cards.borrow().values().cloned().collect())
    }

    fn add_mute(&self, rule: &str, at: &str) -> Result<(), PersistError> {
        self.mutes
            .borrow_mut()
            .insert(rule.to_string(), at.to_string());
        Ok(())
    }

    fn remove_mute(&self, rule: &str) -> Result<(), PersistError> {
        self.mutes.borrow_mut().remove(rule);
        Ok(())
    }

    fn load_mutes(&self) -> Result<Vec<String>, PersistError> {
        Ok(self.mutes.borrow().keys().cloned().collect())
    }

    #[cfg(feature = "fault-injection")]
    fn faults_mut(&mut self) -> &mut crate::plan_file::FaultPlan {
        &mut self.faults
    }

    /// Web durability hook: serialize the full store to JSON bytes. JSON (not
    /// bincode) keeps the blob debuggable and adds no dependency — the store is
    /// already all serde values. The IndexedDB write it feeds dwarfs the encode
    /// cost, so a deep-clone snapshot is fine.
    fn export_snapshot(&self) -> Option<Vec<u8>> {
        serde_json::to_vec(&self.snapshot()).ok()
    }
}
