//! solver — T0 ratio propagation (wasm-able, no LP deps) and T1 local LP (feature "lp").
//! Budget contract per Addendum A4: T0 5ms sync, T1 50ms async; misses are reported, never hidden.

pub mod model;
pub mod t0;
#[cfg(feature = "lp")]
pub mod t1;

pub use model::*;
