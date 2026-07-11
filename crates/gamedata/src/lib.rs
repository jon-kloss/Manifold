//! gamedata — Docs.json → normalized gamedata.sqlite, keyed by game build (SDD §7).

pub mod assets;
pub mod db;
pub mod docs;
pub mod worldnodes;

pub use docs::{parse_docs, GameData};
