//! Entity-granular JSON patches. A `PatchBatch` is what every command returns and
//! what `state://patch` events carry (SDD §4). Paths address the projected-state
//! tree the renderer holds: `/factories/<id>`, `/groups/<id>`, `/ports/<id>`,
//! `/edges/<id>`, `/nodeClaims/<id>`, `/meta/...`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PatchOp {
    Add { path: String, value: Value },
    Replace { path: String, value: Value },
    Remove { path: String },
}

impl PatchOp {
    pub fn path(&self) -> &str {
        match self {
            PatchOp::Add { path, .. }
            | PatchOp::Replace { path, .. }
            | PatchOp::Remove { path } => path,
        }
    }
}

pub type PatchBatch = Vec<PatchOp>;

/// Apply a batch to a JSON tree (two-segment paths: `/collection/id`).
/// Used by tests and by state reconstruction; the renderer has a TS twin.
pub fn apply(tree: &mut Value, batch: &[PatchOp]) -> Result<(), String> {
    for op in batch {
        let (coll, key) = split_path(op.path())?;
        let obj = tree
            .get_mut(coll)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("unknown collection {coll}"))?;
        match op {
            PatchOp::Add { value, .. } | PatchOp::Replace { value, .. } => {
                obj.insert(key.to_string(), value.clone());
            }
            PatchOp::Remove { .. } => {
                obj.remove(key);
            }
        }
    }
    Ok(())
}

fn split_path(path: &str) -> Result<(&str, &str), String> {
    let mut it = path.trim_start_matches('/').splitn(2, '/');
    match (it.next(), it.next()) {
        (Some(c), Some(k)) if !c.is_empty() && !k.is_empty() => Ok((c, k)),
        _ => Err(format!("bad patch path {path}")),
    }
}
