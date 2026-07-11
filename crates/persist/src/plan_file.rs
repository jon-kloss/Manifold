//! `world.ficsit` — SQLite plan file (SDD §10). Tables double as the undo
//! journal. WAL mode; every committed command writes in one transaction, so
//! there is never unsaved state; a rolling `.bak` is taken on open.

use std::path::{Path, PathBuf};

use planner_core::patch::{PatchBatch, PatchOp};
use planner_core::state::{PlanMeta, PlanState};
use planner_core::undo::UndoEntry;
use rusqlite::Connection;

#[derive(Debug, thiserror::Error)]
pub enum PersistError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("plan file corrupt: {0}")]
    Corrupt(String),
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS entities (id TEXT PRIMARY KEY, collection TEXT NOT NULL, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS routes (id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS proposals (id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS proposal_items (proposal_id TEXT NOT NULL, idx INTEGER NOT NULL, json TEXT NOT NULL, PRIMARY KEY (proposal_id, idx));
CREATE TABLE IF NOT EXISTS undo_log (seq INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL, forward TEXT NOT NULL, inverse TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS advisor_cards (id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS mutes (rule TEXT PRIMARY KEY, muted_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS style_guides (id TEXT PRIMARY KEY, json TEXT NOT NULL);
";

pub struct PlanFile {
    conn: Connection,
    pub path: PathBuf,
}

impl PlanFile {
    /// Open (or create) a plan file. Takes the rolling `.bak` before touching
    /// an existing file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PersistError> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            std::fs::copy(&path, path.with_extension("ficsit.bak"))?;
        }
        let conn = Connection::open(&path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn, path })
    }

    pub fn in_memory() -> Result<Self, PersistError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn,
            path: PathBuf::from(":memory:"),
        })
    }

    /// Hydrate canonical state + the applied undo journal.
    pub fn load(&self) -> Result<(PlanState, Vec<UndoEntry>, usize), PersistError> {
        let mut state = PlanState::default();
        if let Ok(json) = self.get_meta("plan_meta") {
            state.meta = serde_json::from_str::<PlanMeta>(&json)?;
        }
        {
            let mut stmt = self
                .conn
                .prepare("SELECT collection, id, json FROM entities")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (collection, id, json) = row?;
                let value: serde_json::Value = serde_json::from_str(&json)?;
                let batch = vec![PatchOp::Add {
                    path: format!("/{collection}/{id}"),
                    value,
                }];
                state.apply_batch(&batch).map_err(PersistError::Corrupt)?;
            }
        }
        let mut entries = Vec::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT label, forward, inverse FROM undo_log ORDER BY seq")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (label, forward, inverse) = row?;
                entries.push(UndoEntry {
                    label,
                    forward: serde_json::from_str(&forward)?,
                    inverse: serde_json::from_str(&inverse)?,
                });
            }
        }
        let cursor: usize = self
            .get_meta("undo_cursor")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(entries.len())
            .min(entries.len());
        Ok((state, entries, cursor))
    }

    fn get_meta(&self, key: &str) -> Result<String, rusqlite::Error> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            (key, value),
        )?;
        Ok(())
    }

    /// Mirror a batch of entity-level ops into rows.
    fn apply_rows(&self, batch: &PatchBatch) -> Result<(), PersistError> {
        for op in batch {
            let path = op.path().trim_start_matches('/');
            let Some((collection, id)) = path.split_once('/') else {
                return Err(PersistError::Corrupt(format!("bad path {path}")));
            };
            if collection == "meta" {
                continue; // plan meta is rewritten wholesale below
            }
            match op {
                PatchOp::Add { value, .. } | PatchOp::Replace { value, .. } => {
                    self.conn.execute(
                        "INSERT OR REPLACE INTO entities (id, collection, json) VALUES (?1, ?2, ?3)",
                        (id, collection, serde_json::to_string(value)?),
                    )?;
                }
                PatchOp::Remove { .. } => {
                    self.conn
                        .execute("DELETE FROM entities WHERE id = ?1", [id])?;
                }
            }
        }
        Ok(())
    }

    /// Persist one committed command: entity rows + undo entry + cursor, atomically.
    /// `applied` is how many undo entries are applied after this commit.
    pub fn commit(
        &mut self,
        entry: &UndoEntry,
        meta: &PlanMeta,
        applied: usize,
    ) -> Result<(), PersistError> {
        let tx_guard = self.conn.unchecked_transaction()?;
        // A new commit truncates any redo tail: keep only the entries that were
        // applied before this one (applied - 1), drop the rest.
        self.conn.execute(
            "DELETE FROM undo_log WHERE seq NOT IN (SELECT seq FROM undo_log ORDER BY seq LIMIT ?1)",
            [applied.saturating_sub(1)],
        )?;
        self.conn.execute(
            "INSERT INTO undo_log (label, forward, inverse) VALUES (?1, ?2, ?3)",
            (
                &entry.label,
                serde_json::to_string(&entry.forward)?,
                serde_json::to_string(&entry.inverse)?,
            ),
        )?;
        self.apply_rows(&entry.forward)?;
        self.set_meta("plan_meta", &serde_json::to_string(meta)?)?;
        self.set_meta("undo_cursor", &applied.to_string())?;
        tx_guard.commit()?;
        Ok(())
    }

    /// Persist an undo/redo move: entity rows + cursor, atomically.
    pub fn checkpoint(
        &mut self,
        batch: &PatchBatch,
        meta: &PlanMeta,
        applied: usize,
    ) -> Result<(), PersistError> {
        let tx_guard = self.conn.unchecked_transaction()?;
        self.apply_rows(batch)?;
        self.set_meta("plan_meta", &serde_json::to_string(meta)?)?;
        self.set_meta("undo_cursor", &applied.to_string())?;
        tx_guard.commit()?;
        Ok(())
    }

    /// Persist window/zoom state (UI restores position on reopen — Principle 1).
    /// Advisor feed persistence — outside the undo journal by design: cards
    /// record what the advisor SAW; undoing a plan edit must not eat them.
    pub fn save_advisor_card(&self, id: &str, json: &str) -> Result<(), PersistError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO advisor_cards (id, json) VALUES (?1, ?2)",
            (id, json),
        )?;
        Ok(())
    }

    pub fn load_advisor_cards(&self) -> Result<Vec<String>, PersistError> {
        let mut stmt = self.conn.prepare("SELECT json FROM advisor_cards")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn add_mute(&self, rule: &str, at: &str) -> Result<(), PersistError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO mutes (rule, muted_at) VALUES (?1, ?2)",
            (rule, at),
        )?;
        Ok(())
    }

    pub fn remove_mute(&self, rule: &str) -> Result<(), PersistError> {
        self.conn
            .execute("DELETE FROM mutes WHERE rule = ?1", [rule])?;
        Ok(())
    }

    pub fn load_mutes(&self) -> Result<Vec<String>, PersistError> {
        let mut stmt = self.conn.prepare("SELECT rule FROM mutes")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn set_view_state(&self, json: &str) -> Result<(), PersistError> {
        self.set_meta("view_state", json)?;
        Ok(())
    }

    pub fn view_state(&self) -> Option<String> {
        self.get_meta("view_state").ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use planner_core::commands::{apply, Command};
    use planner_core::entities::MapPos;
    use planner_core::undo::UndoLog;

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

    #[test]
    fn reopen_restores_state_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("world.ficsit");

        let fid;
        {
            let mut file = PlanFile::open(&path).unwrap();
            let mut state = PlanState::default();
            let mut log = UndoLog::new();
            let tx = apply(&mut state, &cmd_create("NORTHERN FORGE")).unwrap();
            fid = tx.created[0].clone();
            let entry = log.commit(tx);
            file.commit(&entry, &state.meta, log.entries().len())
                .unwrap();
            let tx = apply(
                &mut state,
                &Command::RenameFactory {
                    id: fid.clone(),
                    name: "IRON WORKS".into(),
                },
            )
            .unwrap();
            let entry = log.commit(tx);
            file.commit(&entry, &state.meta, log.entries().len())
                .unwrap();
        }

        // Reopen: state, undo depth, and undo behavior must survive.
        let file2 = PlanFile::open(&path).unwrap();
        let (mut state, entries, cursor) = file2.load().unwrap();
        assert_eq!(cursor, 2);
        assert_eq!(state.factories[&fid].name, "IRON WORKS");
        let mut log = UndoLog::hydrate(entries);
        let batch = log.undo(&mut state).unwrap();
        assert_eq!(state.factories[&fid].name, "NORTHERN FORGE");
        let mut file2 = file2;
        file2.checkpoint(&batch, &state.meta, 1).unwrap();

        // Reopen again: the undo cursor must have persisted.
        let file3 = PlanFile::open(&path).unwrap();
        let (state3, entries3, cursor3) = file3.load().unwrap();
        assert_eq!(cursor3, 1);
        assert_eq!(entries3.len(), 2, "redo tail preserved");
        assert_eq!(state3.factories[&fid].name, "NORTHERN FORGE");
        // .bak exists after reopening an existing file.
        assert!(path.with_extension("ficsit.bak").exists());
    }
}
