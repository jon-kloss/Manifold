//! `PlanStore` — the persistence seam `Session` drives (SDD §10).
//!
//! `Session` used to hold a concrete SQLite `SqlitePlanStore` (in `crate::plan_file`,
//! compiled only under the `sqlite` feature).
//! This trait factors out the exact method set it calls so a future web build
//! can swap SQLite for an IndexedDB-backed store without touching `Session`.
//! The trait is object-safe: `Session` holds a `Box<dyn PlanStore>` (dynamic
//! dispatch — persistence is I/O-bound, so the vtable cost is nil, and it avoids
//! threading a generic `<S>` param through the whole `Session` struct and every
//! constructor/test).
//!
//! Desktop-file specifics (WAL mode, rolling `.bak`, the `:memory:` variant,
//! and the `open`/`in_memory` constructors) stay OFF the trait and ON the
//! concrete SQLite impl — trait objects can't carry constructors, and those
//! concerns don't generalize to a browser store.

use planner_core::patch::PatchBatch;
use planner_core::state::{PlanMeta, PlanState};
use planner_core::undo::UndoEntry;

use crate::plan_file::PersistError;

/// The persistence surface `Session` requires. Every method mirrors the
/// behavior the SQLite impl has today; getters that return `Option<String>`
/// keep that shape (a missing KV row reads `None`), everything else returns
/// `Result<_, PersistError>`.
pub trait PlanStore {
    /// Hydrate canonical state + the applied undo journal + the undo cursor.
    fn load(&self) -> Result<(PlanState, Vec<UndoEntry>, usize), PersistError>;

    /// Persist one committed command: entity rows + undo entry + cursor,
    /// atomically. `applied` is how many undo entries are applied after this
    /// commit; the redo tail is truncated to `applied - 1` prior entries.
    fn commit(
        &mut self,
        entry: &UndoEntry,
        meta: &PlanMeta,
        applied: usize,
    ) -> Result<(), PersistError>;

    /// Persist an undo/redo move: entity rows + cursor, atomically.
    fn checkpoint(
        &mut self,
        batch: &PatchBatch,
        meta: &PlanMeta,
        applied: usize,
    ) -> Result<(), PersistError>;

    // --- KV / singleton accessors (meta store, outside the undo journal) ---

    fn set_view_state(&self, json: &str) -> Result<(), PersistError>;
    fn view_state(&self) -> Option<String>;

    fn set_last_import(&self, json: &str) -> Result<(), PersistError>;
    fn last_import(&self) -> Option<String>;

    fn set_unlocked(&self, json: &str) -> Result<(), PersistError>;
    fn unlocked(&self) -> Option<String>;

    fn set_purchased_schematics(&self, json: &str) -> Result<(), PersistError>;
    fn purchased_schematics(&self) -> Option<String>;

    fn save_advisor_gate(&self, json: &str) -> Result<(), PersistError>;
    fn advisor_gate(&self) -> Option<String>;

    /// Persist the plan meta blob directly (a preference toggle is not an
    /// undoable command, so it writes the `plan_meta` KV row on its own).
    fn save_meta(&self, meta: &PlanMeta) -> Result<(), PersistError>;

    // --- list accessors (advisor cards + mutes, outside the undo journal) ---

    fn save_advisor_card(&self, id: &str, json: &str) -> Result<(), PersistError>;
    fn load_advisor_cards(&self) -> Result<Vec<String>, PersistError>;

    fn add_mute(&self, rule: &str, at: &str) -> Result<(), PersistError>;
    fn remove_mute(&self, rule: &str) -> Result<(), PersistError>;
    fn load_mutes(&self) -> Result<Vec<String>, PersistError>;

    /// Deterministic fault seam (`fault-injection` feature only): mutable access
    /// to the count-down guards that fail the next N `commit`/`checkpoint` calls.
    /// A trait method (not a concrete field) so tests can reach it through the
    /// `Box<dyn PlanStore>` `Session` holds. Object-safe: `&mut self` in,
    /// `&mut FaultPlan` out.
    #[cfg(feature = "fault-injection")]
    fn faults_mut(&mut self) -> &mut crate::plan_file::FaultPlan;
}
