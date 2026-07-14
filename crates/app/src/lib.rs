//! app — the command/session layer shared by the Tauri shell and the dev bridge.
//! Both frontends call the exact same `Session` methods; Rust owns canonical
//! state in every mode (SDD §4 — no state forking).

pub mod advisor;
pub mod altopt;
pub mod buildqueue;
pub mod chat;
pub mod cutover;
pub mod import;
pub mod jobs;
pub mod session;
pub mod wizard;

pub use session::Session;
pub mod tokens;
