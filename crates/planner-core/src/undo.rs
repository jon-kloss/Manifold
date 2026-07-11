//! Command-sourced undo (SDD §4). Each committed transaction appends one
//! `(inverse_patch, forward_patch)` entry; undo applies the inverse in reverse
//! order, redo re-applies the forward. Solve-induced writes live inside the
//! same entry as the edit that caused them.

use serde::{Deserialize, Serialize};

use crate::commands::Transaction;
use crate::patch::PatchBatch;
use crate::state::PlanState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoEntry {
    pub label: String,
    pub forward: PatchBatch,
    pub inverse: PatchBatch,
}

#[derive(Debug, Default)]
pub struct UndoLog {
    entries: Vec<UndoEntry>,
    /// Number of entries currently applied (everything past this is redo tail).
    cursor: usize,
}

impl UndoLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restore from persisted entries (all considered applied).
    pub fn hydrate(entries: Vec<UndoEntry>) -> Self {
        let cursor = entries.len();
        Self { entries, cursor }
    }

    /// Restore from persisted entries with an explicit cursor (persisted state
    /// already reflects the cursor position — entries past it are the redo tail).
    pub fn hydrate_with_cursor(entries: Vec<UndoEntry>, cursor: usize) -> Self {
        let cursor = cursor.min(entries.len());
        Self { entries, cursor }
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.entries.len()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.cursor
            .checked_sub(1)
            .map(|i| self.entries[i].label.as_str())
    }

    pub fn entries(&self) -> &[UndoEntry] {
        &self.entries[..self.cursor]
    }

    /// Commit an open transaction: truncate the redo tail and append.
    /// Inverse ops are stored in reverse application order, ready to apply.
    pub fn commit(&mut self, tx: Transaction) -> UndoEntry {
        let mut inverse = tx.inverse;
        inverse.reverse();
        let entry = UndoEntry {
            label: tx.label,
            forward: tx.forward,
            inverse,
        };
        self.entries.truncate(self.cursor);
        self.entries.push(entry.clone());
        self.cursor = self.entries.len();
        entry
    }

    /// Apply the inverse of the newest applied entry. Returns the batch the
    /// renderer needs (already applied to canonical state).
    pub fn undo(&mut self, state: &mut PlanState) -> Option<PatchBatch> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        let batch = self.entries[self.cursor].inverse.clone();
        state
            .apply_batch(&batch)
            .expect("inverse patch must apply cleanly — undo log corrupt otherwise");
        Some(batch)
    }

    /// Re-apply the next redo entry.
    pub fn redo(&mut self, state: &mut PlanState) -> Option<PatchBatch> {
        if self.cursor >= self.entries.len() {
            return None;
        }
        let batch = self.entries[self.cursor].forward.clone();
        self.cursor += 1;
        state
            .apply_batch(&batch)
            .expect("forward patch must apply cleanly — undo log corrupt otherwise");
        Some(batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{apply, Command};
    use crate::entities::*;

    fn create_factory(state: &mut PlanState, log: &mut UndoLog) -> Id {
        let tx = apply(
            state,
            &Command::CreateFactory {
                name: "NORTHERN FORGE".into(),
                position: MapPos {
                    x: 100.0,
                    y: 200.0,
                    z: 0.0,
                },
                region: "GRASS FIELDS".into(),
            },
        )
        .unwrap();
        let id = tx.created[0].clone();
        log.commit(tx);
        id
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut state = PlanState::default();
        let mut log = UndoLog::new();
        let fid = create_factory(&mut state, &mut log);
        assert!(state.factories.contains_key(&fid));

        let tx = apply(
            &mut state,
            &Command::RenameFactory {
                id: fid.clone(),
                name: "IRON WORKS".into(),
            },
        )
        .unwrap();
        log.commit(tx);
        assert_eq!(state.factories[&fid].name, "IRON WORKS");

        log.undo(&mut state).unwrap();
        assert_eq!(state.factories[&fid].name, "NORTHERN FORGE");
        log.undo(&mut state).unwrap();
        assert!(!state.factories.contains_key(&fid));
        assert!(!log.can_undo());

        log.redo(&mut state).unwrap();
        assert!(state.factories.contains_key(&fid));
        log.redo(&mut state).unwrap();
        assert_eq!(state.factories[&fid].name, "IRON WORKS");
        assert!(!log.can_redo());
    }

    #[test]
    fn new_commit_truncates_redo_tail() {
        let mut state = PlanState::default();
        let mut log = UndoLog::new();
        let fid = create_factory(&mut state, &mut log);
        let tx = apply(
            &mut state,
            &Command::RenameFactory {
                id: fid.clone(),
                name: "A".into(),
            },
        )
        .unwrap();
        log.commit(tx);
        log.undo(&mut state).unwrap();
        let tx = apply(
            &mut state,
            &Command::RenameFactory {
                id: fid.clone(),
                name: "B".into(),
            },
        )
        .unwrap();
        log.commit(tx);
        assert!(!log.can_redo());
        assert_eq!(state.factories[&fid].name, "B");
    }

    #[test]
    fn cascade_delete_restores_children_on_undo() {
        let mut state = PlanState::default();
        let mut log = UndoLog::new();
        let fid = create_factory(&mut state, &mut log);
        let tx = apply(
            &mut state,
            &Command::AddGroup {
                factory: fid.clone(),
                machine: "Build_ConstructorMk1_C".into(),
                recipe: "Recipe_IronRod_C".into(),
                count: 4,
                clock: 1.0,
                graph_pos: GraphPos { x: 0.0, y: 0.0 },
                floor: 0,
            },
        )
        .unwrap();
        let gid = tx.created[0].clone();
        log.commit(tx);

        let tx = apply(&mut state, &Command::DeleteFactory { id: fid.clone() }).unwrap();
        log.commit(tx);
        assert!(state.factories.is_empty());
        assert!(state.groups.is_empty());

        log.undo(&mut state).unwrap();
        assert!(state.factories.contains_key(&fid));
        assert!(state.groups.contains_key(&gid));
        assert_eq!(state.factories[&fid].groups, vec![gid]);
    }

    #[test]
    fn built_entities_are_immutable() {
        let mut state = PlanState::default();
        let mut log = UndoLog::new();
        let fid = create_factory(&mut state, &mut log);
        state.factories.get_mut(&fid).unwrap().status = Status::Built;
        let err = apply(
            &mut state,
            &Command::MoveFactoryPin {
                id: fid.clone(),
                position: MapPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
            },
        );
        assert!(matches!(
            err,
            Err(crate::commands::DomainError::BuiltImmutable { .. })
        ));
        let err = apply(&mut state, &Command::DeleteFactory { id: fid });
        assert!(matches!(
            err,
            Err(crate::commands::DomainError::BuiltImmutable { .. })
        ));
    }

    #[test]
    fn projection_matches_patch_application() {
        // Renderer invariant: hydrate-then-patch equals re-hydrate.
        let mut state = PlanState::default();
        let mut log = UndoLog::new();
        let mut projected = state.project();
        let tx = apply(
            &mut state,
            &Command::CreateFactory {
                name: "X".into(),
                position: MapPos {
                    x: 1.0,
                    y: 2.0,
                    z: 0.0,
                },
                region: "DUNE DESERT".into(),
            },
        )
        .unwrap();
        let entry = log.commit(tx);
        crate::patch::apply(&mut projected, &entry.forward).unwrap();
        assert_eq!(projected, state.project());
        let batch = log.undo(&mut state).unwrap();
        crate::patch::apply(&mut projected, &batch).unwrap();
        assert_eq!(projected, state.project());
    }
}
