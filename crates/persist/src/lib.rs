//! persist — one `.ficsit` SQLite plan file per world (SDD §10).
//! WAL mode; autosave on transaction commit; rolling .bak on open.
//!
//! `Session` drives persistence through the [`PlanStore`] trait, not the
//! concrete file type, so a future web build can swap SQLite for an
//! IndexedDB-backed store. [`SqlitePlanStore`] is the desktop impl (the
//! `PlanFile` alias keeps its historical name); [`MemoryPlanStore`] is a
//! pure-Rust impl that proves the seam is SQLite-independent.

pub mod memory;
pub mod plan_file;
pub mod store;

pub use memory::MemoryPlanStore;
pub use plan_file::PersistError;
#[cfg(feature = "sqlite")]
pub use plan_file::{PlanFile, SqlitePlanStore};
pub use store::PlanStore;

#[cfg(feature = "fault-injection")]
pub use plan_file::FaultPlan;
