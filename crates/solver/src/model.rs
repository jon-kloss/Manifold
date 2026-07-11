//! Solver-facing snapshot types. The app layer composes a `FactorySnapshot`
//! from canonical state + gamedata; the solver never queries anything — this
//! keeps T0 a pure function (SDD §5.1) and identical between native and WASM.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type ItemId = String;

/// Satisfactory power law: production scales linearly with clock,
/// power scales with clock^1.321928.
pub const POWER_EXPONENT: f64 = 1.321928;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSpec {
    pub id: String,
    pub machine: String,
    pub duration_s: f64,
    /// (item, amount per cycle)
    pub inputs: Vec<(ItemId, f64)>,
    pub outputs: Vec<(ItemId, f64)>,
    pub power_mw: f64,
}

impl RecipeSpec {
    /// Items/min of `item` produced by one machine at 100% clock.
    pub fn out_rate(&self, item: &str) -> f64 {
        self.outputs
            .iter()
            .find(|(i, _)| i == item)
            .map(|(_, amt)| amt * 60.0 / self.duration_s)
            .unwrap_or(0.0)
    }

    pub fn in_rate(&self, item: &str) -> f64 {
        self.inputs
            .iter()
            .find(|(i, _)| i == item)
            .map(|(_, amt)| amt * 60.0 / self.duration_s)
            .unwrap_or(0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSpec {
    pub id: String,
    pub recipe: RecipeSpec,
    pub count: u32,
    pub clock: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum NodeRef {
    Group(String),
    /// Boundary input port (items enter the factory here).
    Input(String),
    /// Boundary output port (items leave here; targets live here).
    Output(String),
    /// Belt junction (splitter/merger/storage): pure conservation, no transform.
    Junction(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeSpec {
    pub id: String,
    pub from: NodeRef,
    pub to: NodeRef,
    pub item: ItemId,
    /// Belt capacity in items/min (from tier).
    pub capacity: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputPortSpec {
    pub id: String,
    pub item: ItemId,
    /// Hard ceiling (node extraction rate, bound route capacity). None = open.
    pub ceiling: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputPortSpec {
    pub id: String,
    pub item: ItemId,
    /// Target rate in items/min.
    pub rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorySnapshot {
    pub groups: Vec<GroupSpec>,
    pub edges: Vec<EdgeSpec>,
    pub inputs: Vec<InputPortSpec>,
    pub outputs: Vec<OutputPortSpec>,
    /// Junction node ids (splitters/mergers/storage) — conservation only.
    #[serde(default)]
    pub junctions: Vec<String>,
}

/// The edit being previewed (T0) or committed (T1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum T0Edit {
    /// Drag/commit of an output target slider.
    SetTarget { port: String, rate: f64 },
    /// Manual clock change on one group (count re-derives).
    SetClock { group: String, clock: f64 },
    /// Recompute as-is (recipe/tier/structure change already in snapshot).
    Recompute,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupResult {
    pub count: u32,
    pub clock: f64,
    pub power_mw: f64,
    /// items/min by item.
    pub in_rates: BTreeMap<ItemId, f64>,
    pub out_rates: BTreeMap<ItemId, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeResult {
    pub flow: f64,
    /// flow / capacity, 0..∞ (≥1 = over capacity).
    pub saturation: f64,
}

/// What binds the target ceiling — named so the UI can say it (no dead ends).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Constraint {
    BeltCapacity {
        edge: String,
        item: ItemId,
        capacity: f64,
    },
    InputCeiling {
        port: String,
        item: ItemId,
        ceiling: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetCeiling {
    /// Maximum feasible rate for the edited target given all constraints.
    pub max_rate: f64,
    pub binding: Constraint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveResult {
    pub groups: BTreeMap<String, GroupResult>,
    pub edges: BTreeMap<String, EdgeResult>,
    /// Realized rate per port (inputs and outputs).
    pub ports: BTreeMap<String, f64>,
    pub total_power_mw: f64,
    /// Present when the edit targets an output slider: where it hard-stops.
    pub target_ceiling: Option<TargetCeiling>,
    /// True when the requested rate exceeded the ceiling and was clamped.
    pub clamped: bool,
    pub solve_us: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SolveError {
    #[error("factory graph has a cycle — T0 requires a DAG")]
    Cyclic,
    #[error("unknown reference: {id}")]
    UnknownRef { id: String },
    #[error("solver internal error: {message}")]
    Internal { message: String },
}
