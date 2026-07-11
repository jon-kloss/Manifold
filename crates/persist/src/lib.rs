//! persist — one `.ficsit` SQLite plan file per world (SDD §10).
//! WAL mode; autosave on transaction commit; rolling .bak on open.

pub mod plan_file;

pub use plan_file::PlanFile;
