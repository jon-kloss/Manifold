//! planner-core — domain model, canonical state, command layer, undo log.
//! Rust owns canonical state (SDD §4); the renderer is a projection patched by events.

pub mod commands;
pub mod entities;
pub mod layout;
pub mod patch;
pub mod proposals;
pub mod state;
pub mod transport;
pub mod undo;

pub use entities::*;
pub use patch::{PatchBatch, PatchOp};
pub use state::PlanState;
