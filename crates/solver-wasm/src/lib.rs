//! Thin wasm-bindgen wrapper over solver::t0 for the renderer's optimistic drag path.

use wasm_bindgen::prelude::*;

/// Solve a factory snapshot with T0 ratio propagation.
/// `snapshot_js` is a `FactorySnapshot`, `edit_js` a `T0Edit`; returns a `T0Result`.
#[wasm_bindgen]
pub fn t0_solve(snapshot_js: JsValue, edit_js: JsValue) -> Result<JsValue, JsValue> {
    let snapshot: solver::model::FactorySnapshot = serde_wasm_bindgen::from_value(snapshot_js)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let edit: solver::model::T0Edit =
        serde_wasm_bindgen::from_value(edit_js).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let result = solver::t0::solve(&snapshot, &edit);
    serde_wasm_bindgen::to_value(&result).map_err(|e| JsValue::from_str(&e.to_string()))
}
