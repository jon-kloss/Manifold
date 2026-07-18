//! `MemoryPlanStore` parity with `SqlitePlanStore` — the Phase-1 de-risk proof.
//!
//! Every test builds ONE canonical command script (ids are ULIDs, so replaying
//! commands twice would mint different ids and diverge) and replays the SAME
//! `UndoEntry`/`PatchBatch` inputs into both stores, asserting identical
//! observable output from `load()` and every KV/list accessor. That is the
//! bar: a non-SQLite `PlanStore` behaves indistinguishably, so `Session` is
//! genuinely decoupled from rusqlite (the wasm precondition).

use persist::{MemoryPlanStore, PlanStore, SqlitePlanStore};
use planner_core::commands::{apply, Command};
use planner_core::entities::{Id, MapPos};
use planner_core::patch::PatchBatch;
use planner_core::state::{PlanMeta, PlanState};
use planner_core::undo::{UndoEntry, UndoLog};

fn cmd_create(name: &str) -> Command {
    Command::CreateFactory {
        name: name.into(),
        position: MapPos {
            x: 1.0,
            y: 2.0,
            z: 0.0,
        },
        region: "GRASS FIELDS".into(),
    }
}

fn cmd_rename(id: &Id, name: &str) -> Command {
    Command::RenameFactory {
        id: id.clone(),
        name: name.into(),
    }
}

/// One durable operation, captured so it can be replayed byte-identically into
/// any store — mirrors exactly what `Session` hands the store.
enum Step {
    Commit {
        entry: UndoEntry,
        meta: PlanMeta,
        applied: usize,
    },
    Checkpoint {
        batch: PatchBatch,
        meta: PlanMeta,
        applied: usize,
    },
}

fn replay(store: &mut dyn PlanStore, steps: &[Step]) {
    for step in steps {
        match step {
            Step::Commit {
                entry,
                meta,
                applied,
            } => store.commit(entry, meta, *applied).unwrap(),
            Step::Checkpoint {
                batch,
                meta,
                applied,
            } => store.checkpoint(batch, meta, *applied).unwrap(),
        }
    }
}

/// The observable result of `load()`: canonical projection + journal labels +
/// cursor. Two stores fed identical steps must produce identical observations.
fn observe(store: &dyn PlanStore) -> (serde_json::Value, Vec<String>, usize) {
    let (state, entries, cursor) = store.load().unwrap();
    (
        state.project(),
        entries.iter().map(|e| e.label.clone()).collect(),
        cursor,
    )
}

/// A script that exercises commit, checkpoint (undo + redo), and — critically —
/// redo-tail truncation: create A → rename B → rename C, undo twice back to A,
/// then commit rename D, which must drop the [B, C] redo tail. Returns the
/// steps plus the factory id for content assertions.
fn truncation_script() -> (Vec<Step>, Id) {
    // Mirror Session::commit_mutation: stage, record with applied = current
    // depth + this entry, then advance the in-memory log.
    fn commit_step(
        state: &mut PlanState,
        log: &mut UndoLog,
        cmd: &Command,
        steps: &mut Vec<Step>,
    ) -> Vec<Id> {
        let tx = apply(state, cmd).unwrap();
        let created = tx.created.clone();
        let entry = UndoLog::stage(tx);
        steps.push(Step::Commit {
            entry: entry.clone(),
            meta: state.meta.clone(),
            applied: log.entries().len() + 1,
        });
        log.push(entry);
        created
    }

    let mut state = PlanState::default();
    let mut log = UndoLog::new();
    let mut steps = Vec::new();

    let fid = commit_step(&mut state, &mut log, &cmd_create("A"), &mut steps)[0].clone();
    commit_step(&mut state, &mut log, &cmd_rename(&fid, "B"), &mut steps);
    commit_step(&mut state, &mut log, &cmd_rename(&fid, "C"), &mut steps);

    // Undo twice: checkpoint with the post-undo applied depth (Session's
    // `applied_count()` == `undo.entries().len()`).
    for _ in 0..2 {
        let batch = log.undo(&mut state).unwrap().unwrap();
        steps.push(Step::Checkpoint {
            batch,
            meta: state.meta.clone(),
            applied: log.entries().len(),
        });
    }

    // Commit a new edit off the rewound cursor — truncates the [B, C] tail.
    commit_step(&mut state, &mut log, &cmd_rename(&fid, "D"), &mut steps);

    (steps, fid)
}

#[test]
fn commit_checkpoint_and_truncation_parity() {
    let (steps, fid) = truncation_script();

    let mut sqlite = SqlitePlanStore::in_memory().unwrap();
    let mut memory = MemoryPlanStore::new();
    replay(&mut sqlite, &steps);
    replay(&mut memory, &steps);

    // The two stores agree on every observable: projection, journal, cursor.
    assert_eq!(
        observe(&sqlite),
        observe(&memory),
        "MemoryPlanStore must be observationally identical to SqlitePlanStore"
    );

    // And the truncation actually happened: journal is [A, D] (len 2), cursor
    // at 2 (no redo tail), and the live name is D.
    let (state, entries, cursor) = memory.load().unwrap();
    assert_eq!(entries.len(), 2, "redo tail [B, C] truncated");
    assert_eq!(cursor, 2, "cursor == depth ⇒ nothing to redo");
    assert_eq!(state.factories[&fid].name, "D");
}

/// A script that exercises the entity DELETE path: create A and a SURVIVOR,
/// then commit `DeleteFactory { A }`, whose forward batch carries a
/// `PatchOp::Remove` for A. A store whose Remove arm is broken leaves a ghost
/// A row that projects back on `load()`. Returns the steps plus (deleted id,
/// surviving id) for content assertions.
fn delete_script() -> (Vec<Step>, Id, Id) {
    fn commit_step(
        state: &mut PlanState,
        log: &mut UndoLog,
        cmd: &Command,
        steps: &mut Vec<Step>,
    ) -> Vec<Id> {
        let tx = apply(state, cmd).unwrap();
        let created = tx.created.clone();
        let entry = UndoLog::stage(tx);
        steps.push(Step::Commit {
            entry: entry.clone(),
            meta: state.meta.clone(),
            applied: log.entries().len() + 1,
        });
        log.push(entry);
        created
    }

    let mut state = PlanState::default();
    let mut log = UndoLog::new();
    let mut steps = Vec::new();

    let deleted = commit_step(&mut state, &mut log, &cmd_create("A"), &mut steps)[0].clone();
    let survivor =
        commit_step(&mut state, &mut log, &cmd_create("SURVIVOR"), &mut steps)[0].clone();
    // The forward batch of this commit contains a `PatchOp::Remove` for A.
    commit_step(
        &mut state,
        &mut log,
        &Command::DeleteFactory {
            id: deleted.clone(),
        },
        &mut steps,
    );

    (steps, deleted, survivor)
}

#[test]
fn delete_path_parity() {
    let (steps, deleted, survivor) = delete_script();

    // Sanity: the DeleteFactory forward batch really does carry a Remove — this
    // is the arm the parity comparison below pins.
    let has_remove = matches!(&steps[2], Step::Commit { entry, .. }
        if entry.forward.iter().any(|op| matches!(op, planner_core::patch::PatchOp::Remove { .. })));
    assert!(has_remove, "delete script must exercise PatchOp::Remove");

    let mut sqlite = SqlitePlanStore::in_memory().unwrap();
    let mut memory = MemoryPlanStore::new();
    replay(&mut sqlite, &steps);
    replay(&mut memory, &steps);

    // Both stores must agree on every observable. A ghost A left in Memory's
    // `entities` map (a no-op Remove arm) projects back on load() and diverges
    // from SQLite right here.
    assert_eq!(
        observe(&sqlite),
        observe(&memory),
        "MemoryPlanStore delete arm must match SqlitePlanStore"
    );

    // And the delete actually took: A is gone from both, SURVIVOR remains.
    for store in [&sqlite as &dyn PlanStore, &memory as &dyn PlanStore] {
        let (state, _, _) = store.load().unwrap();
        assert!(!state.factories.contains_key(&deleted), "A must be deleted");
        assert!(
            state.factories.contains_key(&survivor),
            "SURVIVOR must remain"
        );
    }
}

#[test]
fn load_roundtrip_parity_on_fresh_store() {
    // An empty store hydrates to default state, no journal, cursor 0 — same on
    // both impls.
    let sqlite = SqlitePlanStore::in_memory().unwrap();
    let memory = MemoryPlanStore::new();
    assert_eq!(observe(&sqlite), observe(&memory));
    let (_, entries, cursor) = memory.load().unwrap();
    assert!(entries.is_empty());
    assert_eq!(cursor, 0);
}

#[test]
fn journal_payload_parity_and_undo_replay() {
    // `observe()` compares journal entries by `label` only. This test pins the
    // full `forward`/`inverse` `PatchBatch` payloads between the two stores, and
    // then replays one undo through an `UndoLog` hydrated from each loaded
    // journal (mirroring plan_file.rs's `reopen_restores_state_and_undo`). A
    // corrupted or forward/inverse-swapped Memory journal must fail here.
    let (steps, fid) = truncation_script();
    let mut sqlite = SqlitePlanStore::in_memory().unwrap();
    let mut memory = MemoryPlanStore::new();
    replay(&mut sqlite, &steps);
    replay(&mut memory, &steps);

    let (sq_state, sq_entries, sq_cursor) = sqlite.load().unwrap();
    let (mem_state, mem_entries, mem_cursor) = memory.load().unwrap();

    // Deep journal equality: label AND both patch payloads, entry by entry.
    assert_eq!(sq_entries.len(), mem_entries.len());
    assert_eq!(sq_cursor, mem_cursor);
    for (a, b) in sq_entries.iter().zip(&mem_entries) {
        assert_eq!(a.label, b.label, "journal label diverged");
        assert_eq!(a.forward, b.forward, "journal forward payload diverged");
        assert_eq!(a.inverse, b.inverse, "journal inverse payload diverged");
    }

    // Replay one undo through a hydrated log on each store — resulting canonical
    // state must be identical (a swapped forward/inverse would apply the wrong
    // batch and diverge).
    let mut st_sq = sq_state;
    let mut log_sq = UndoLog::hydrate_with_cursor(sq_entries, sq_cursor);
    log_sq.undo(&mut st_sq).unwrap().unwrap();

    let mut st_mem = mem_state;
    let mut log_mem = UndoLog::hydrate_with_cursor(mem_entries, mem_cursor);
    log_mem.undo(&mut st_mem).unwrap().unwrap();

    assert_eq!(
        st_sq.project(),
        st_mem.project(),
        "post-undo projection diverged between stores"
    );
    // The truncation script's live name is D; undoing the D rename returns A.
    assert_eq!(st_mem.factories[&fid].name, "A", "undo rewound D -> A");
}

#[test]
fn kv_accessor_parity() {
    let sqlite = SqlitePlanStore::in_memory().unwrap();
    let memory = MemoryPlanStore::new();
    let stores: [&dyn PlanStore; 2] = [&sqlite, &memory];

    // Absent keys read None on both impls.
    for s in stores {
        assert_eq!(s.view_state(), None);
        assert_eq!(s.last_import(), None);
        assert_eq!(s.unlocked(), None);
        assert_eq!(s.purchased_schematics(), None);
        assert_eq!(s.advisor_gate(), None);
    }

    // Each setter round-trips, and both impls return the same value.
    for s in stores {
        s.set_view_state("{\"zoom\":2}").unwrap();
        s.set_last_import("{\"saveName\":\"world\"}").unwrap();
        s.set_unlocked("[\"Recipe_A_C\"]").unwrap();
        s.set_purchased_schematics("[\"Schematic_3-1_C\"]").unwrap();
        s.save_advisor_gate("{\"armed\":[\"k\"]}").unwrap();
    }
    assert_eq!(sqlite.view_state(), memory.view_state());
    assert_eq!(sqlite.view_state().as_deref(), Some("{\"zoom\":2}"));
    assert_eq!(sqlite.last_import(), memory.last_import());
    assert_eq!(sqlite.unlocked(), memory.unlocked());
    assert_eq!(sqlite.purchased_schematics(), memory.purchased_schematics());
    assert_eq!(sqlite.advisor_gate(), memory.advisor_gate());

    // Per-key overwrite: set the same key a SECOND time with a new value and
    // re-read — last write must win on both impls. A per-key insert-if-absent
    // bug would keep the original "{\"zoom\":2}" and diverge here.
    for s in stores {
        s.set_view_state("{\"zoom\":9}").unwrap();
    }
    assert_eq!(sqlite.view_state(), memory.view_state());
    assert_eq!(
        sqlite.view_state().as_deref(),
        Some("{\"zoom\":9}"),
        "second set_view_state must overwrite the first (last-wins)"
    );

    // save_meta writes the plan_meta row that load() reads back into state.meta.
    let mut meta = PlanMeta::default();
    meta.preferences.no_trains = true;
    sqlite.save_meta(&meta).unwrap();
    memory.save_meta(&meta).unwrap();
    assert_eq!(sqlite.load().unwrap().0.meta, memory.load().unwrap().0.meta,);
    assert_eq!(sqlite.load().unwrap().0.meta, meta);
}

#[test]
fn list_accessor_parity() {
    let sqlite = SqlitePlanStore::in_memory().unwrap();
    let memory = MemoryPlanStore::new();
    let stores: [&dyn PlanStore; 2] = [&sqlite, &memory];

    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };

    for s in stores {
        assert!(s.load_advisor_cards().unwrap().is_empty());
        assert!(s.load_mutes().unwrap().is_empty());
        s.save_advisor_card("c1", "{\"id\":\"c1\"}").unwrap();
        s.save_advisor_card("c2", "{\"id\":\"c2\"}").unwrap();
        // Upsert on the same id replaces, not appends.
        s.save_advisor_card("c1", "{\"id\":\"c1\",\"v\":2}")
            .unwrap();
        s.add_mute("rule_a", "2026-01-01T00:00:00Z").unwrap();
        s.add_mute("rule_b", "2026-01-02T00:00:00Z").unwrap();
        s.remove_mute("rule_a").unwrap();
    }

    assert_eq!(
        sorted(sqlite.load_advisor_cards().unwrap()),
        sorted(memory.load_advisor_cards().unwrap()),
    );
    assert_eq!(sqlite.load_advisor_cards().unwrap().len(), 2);
    assert_eq!(
        sorted(sqlite.load_mutes().unwrap()),
        sorted(memory.load_mutes().unwrap()),
    );
    assert_eq!(memory.load_mutes().unwrap(), vec!["rule_b".to_string()]);
}

#[test]
fn trait_object_commit_load_cycle() {
    // Pin the dyn-safe surface: drive a `Box<dyn PlanStore>` — the exact type
    // `Session` holds — through commit + load, for BOTH impls.
    let (steps, fid) = truncation_script();
    for mut store in [
        Box::new(SqlitePlanStore::in_memory().unwrap()) as Box<dyn PlanStore>,
        Box::new(MemoryPlanStore::new()) as Box<dyn PlanStore>,
    ] {
        replay(store.as_mut(), &steps);
        let (state, entries, cursor) = store.load().unwrap();
        assert_eq!(state.factories[&fid].name, "D");
        assert_eq!(entries.len(), 2);
        assert_eq!(cursor, 2);
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn commit_fault_parity() {
    // Arm `fail_commits = 1` on BOTH stores, attempt the first commit, and
    // assert identical `Err` plus identical unchanged (no-trace) state — the
    // Memory fault seam must be observationally identical to SQLite's.
    let (steps, _fid) = truncation_script();
    let (entry, meta, applied) = match &steps[0] {
        Step::Commit {
            entry,
            meta,
            applied,
        } => (entry, meta, *applied),
        Step::Checkpoint { .. } => unreachable!("first step is a commit"),
    };

    let mut sqlite = SqlitePlanStore::in_memory().unwrap();
    let mut memory = MemoryPlanStore::new();
    sqlite.faults_mut().fail_commits = 1;
    memory.faults_mut().fail_commits = 1;

    let err_sq = sqlite.commit(entry, meta, applied).unwrap_err();
    let err_mem = memory.commit(entry, meta, applied).unwrap_err();
    // Identical error surface (both inject `io: injected persist fault (commit)`).
    assert_eq!(
        err_sq.to_string(),
        err_mem.to_string(),
        "injected commit fault must present identically"
    );

    // No trace: both stores are still empty (the failed commit rolled back /
    // never touched state), and remain observationally identical.
    assert_eq!(observe(&sqlite), observe(&memory));
    for store in [&sqlite as &dyn PlanStore, &memory as &dyn PlanStore] {
        let (state, entries, cursor) = store.load().unwrap();
        assert!(entries.is_empty(), "failed commit left no journal entry");
        assert_eq!(cursor, 0, "cursor unmoved");
        assert!(state.factories.is_empty(), "no entity row written");
    }
}

/// Web snapshot round-trip (Phase 3): a `MemoryPlanStore` driven through the
/// full script — including redo-tail truncation AND KV/list writes — exports
/// to bytes, reconstructs, and reads back OBSERVATIONALLY IDENTICAL. This is
/// the durability contract the web worker relies on: `export_snapshot` →
/// IndexedDB → `from_snapshot_bytes` reconstitutes the exact plan (canonical
/// state, undo journal + cursor, and every meta/card/mute the store held).
#[test]
fn snapshot_export_import_round_trips() {
    let (steps, fid) = truncation_script();
    let mut memory = MemoryPlanStore::new();
    replay(&mut memory, &steps);
    // Exercise the KV + list surfaces too, so the round-trip covers everything
    // Session hydrates, not just the entity rows + journal.
    memory.set_view_state(r#"{"openFactory":null}"#).unwrap();
    memory.set_last_import(r#"{"saveName":"world"}"#).unwrap();
    memory.set_unlocked(r#"["Recipe_Alt_C"]"#).unwrap();
    memory
        .set_purchased_schematics(r#"["Schematic_3-1_C"]"#)
        .unwrap();
    memory.save_advisor_gate(r#"{"armed":true}"#).unwrap();
    memory
        .save_advisor_card("card-1", r#"{"id":"card-1"}"#)
        .unwrap();
    memory
        .add_mute("power_swing", "2026-07-15T00:00:00Z")
        .unwrap();

    let before = observe(&memory);
    let bytes = memory
        .export_snapshot()
        .expect("MemoryPlanStore exports a blob");
    let restored = MemoryPlanStore::from_snapshot_bytes(&bytes).unwrap();

    // Canonical state, journal, and cursor survive byte-identical.
    assert_eq!(before, observe(&restored), "load() round-trips");
    // The rewound-then-recommitted cursor and the surviving factory content.
    let (state, entries, cursor) = restored.load().unwrap();
    assert_eq!(
        state.factories.get(&fid).map(|f| f.name.as_str()),
        Some("D")
    );
    assert_eq!(
        cursor,
        entries.len(),
        "cursor at the tip after the final commit"
    );
    // Every KV/list accessor round-trips.
    assert_eq!(
        restored.view_state().as_deref(),
        Some(r#"{"openFactory":null}"#)
    );
    assert_eq!(
        restored.last_import().as_deref(),
        Some(r#"{"saveName":"world"}"#),
        "last_import survives the snapshot round-trip"
    );
    assert_eq!(restored.unlocked().as_deref(), Some(r#"["Recipe_Alt_C"]"#));
    assert_eq!(
        restored.purchased_schematics().as_deref(),
        Some(r#"["Schematic_3-1_C"]"#)
    );
    assert_eq!(
        restored.advisor_gate().as_deref(),
        Some(r#"{"armed":true}"#)
    );
    assert_eq!(
        restored.load_advisor_cards().unwrap(),
        vec![r#"{"id":"card-1"}"#.to_string()]
    );
    assert_eq!(
        restored.load_mutes().unwrap(),
        vec!["power_swing".to_string()]
    );
}

/// A snapshot with an unknown `version` is rejected, not silently mis-loaded.
#[test]
fn snapshot_rejects_unknown_version() {
    let mut memory = MemoryPlanStore::new();
    let (steps, _fid) = truncation_script();
    replay(&mut memory, &steps);
    let bytes = memory.export_snapshot().unwrap();
    // Bump the version field in the raw JSON and confirm reconstruction refuses.
    let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    v["version"] = serde_json::json!(9999);
    let tampered = serde_json::to_vec(&v).unwrap();
    assert!(MemoryPlanStore::from_snapshot_bytes(&tampered).is_err());
}

/// `MemoryPlanStore::reset` (the web build's store) must empty EVERY map, and
/// the emptiness must survive the export_snapshot → from_snapshot_bytes
/// round-trip the web worker writes to IndexedDB — otherwise a "new empire"
/// would resurrect stale rows (cards/mutes/meta) on reload.
#[test]
fn reset_empties_every_map_and_the_snapshot_round_trip() {
    let (steps, _fid) = truncation_script();
    let mut memory = MemoryPlanStore::new();
    replay(&mut memory, &steps);
    memory.set_view_state(r#"{"z":3}"#).unwrap();
    memory.set_unlocked(r#"["Recipe_Alt_C"]"#).unwrap();
    memory
        .save_advisor_card("card-1", r#"{"id":"card-1"}"#)
        .unwrap();
    memory
        .add_mute("power_swing", "2026-07-15T00:00:00Z")
        .unwrap();

    memory.reset().unwrap();

    // In-memory: everything empty.
    let (state, entries, cursor) = memory.load().unwrap();
    assert!(state.factories.is_empty(), "entities cleared");
    assert!(
        entries.is_empty() && cursor == 0,
        "journal + cursor cleared"
    );
    assert!(
        memory.view_state().is_none() && memory.unlocked().is_none(),
        "meta cleared"
    );
    assert!(
        memory.load_advisor_cards().unwrap().is_empty(),
        "cards cleared"
    );
    assert!(memory.load_mutes().unwrap().is_empty(), "mutes cleared");

    // Durable: the exported blob (what the web worker writes to IndexedDB) is
    // empty too — a reload does not resurrect any stale row.
    let bytes = memory.export_snapshot().expect("exports a blob");
    let restored = MemoryPlanStore::from_snapshot_bytes(&bytes).unwrap();
    let (rstate, rentries, _) = restored.load().unwrap();
    assert!(rstate.factories.is_empty(), "restored entities empty");
    assert!(rentries.is_empty(), "restored journal empty");
    assert!(
        restored.load_advisor_cards().unwrap().is_empty(),
        "restored cards empty"
    );
    assert!(
        restored.load_mutes().unwrap().is_empty(),
        "restored mutes empty"
    );
    assert!(restored.view_state().is_none(), "restored meta empty");
}
