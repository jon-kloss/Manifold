//! Docs.json parser (SDD §7). The community-documented file ships with the game
//! install (UTF-16LE); the bundled test fixture is UTF-8. Both are accepted —
//! encoding is detected by BOM.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum DocsError {
    #[error("Docs.json is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Docs.json has unexpected shape: {0}")]
    Shape(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub class_name: String,
    pub display_name: String,
    /// RF_SOLID | RF_LIQUID | RF_GAS
    pub form: String,
    pub stack_size: String,
    /// MJ per item — drives generator fuel-burn synthesis.
    #[serde(default)]
    pub energy_mj: f64,
    /// FGResourceDescriptor — a world-sourced raw (ores, water, oil, nitrogen).
    /// Raws can still appear as recipe products (Unpackage Water), but a
    /// planner must source them from extractors, never from those recipes.
    #[serde(default)]
    pub is_resource: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub class_name: String,
    pub display_name: String,
    pub duration_s: f64,
    /// (item class, amount per cycle) — liquids already normalized to m³.
    pub ingredients: Vec<(String, f64)>,
    pub products: Vec<(String, f64)>,
    pub produced_in: Vec<String>,
    /// True for alternate recipes (unlocked via hard drives).
    pub alternate: bool,
    /// Average sustained draw override for recipes run in variable-power
    /// machines (Particle Accelerator etc.): constant + factor/2, the
    /// midpoint of the per-cycle power ramp. None for fixed-power recipes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable_power_mw: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum MachineKind {
    Manufacturer,
    Extractor {
        items_per_cycle: f64,
        cycle_time_s: f64,
    },
    /// Fuel generator: produces MW (as the `POWER_ITEM` pseudo-item) by
    /// burning fuel through synthesized recipes.
    Generator {
        power_production_mw: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    pub class_name: String,
    pub display_name: String,
    pub power_mw: f64,
    /// Top-down build footprint (width, depth) in meters, ~1 decimal —
    /// derived from Docs.json `mClearanceData`: the union over CT_Hard
    /// clearance boxes with each box's `RelativeTransform` applied
    /// (centimeters ÷ 100). None when the catalog carries no hard clearance
    /// data (older trimmed fixtures) — honest absence, never 0×0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint_m: Option<(f64, f64)>,
    #[serde(flatten)]
    pub kind: MachineKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Belt {
    pub class_name: String,
    pub display_name: String,
    /// items/min (Docs mSpeed is items/min × 2).
    pub capacity_per_min: f64,
    pub tier: u8,
}

/// A tier-progression milestone (an `EST_Milestone` FGSchematic) — what the
/// player buys at the HUB terminal to advance. `cost` is the build cost in
/// (item class, quantity) pairs, all SOLID parts (no fluid m³ scaling), used
/// by the opportunity engine's `milestone_gap` family (PR 4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Milestone {
    pub display_name: String,
    pub tier: u32,
    /// (item class, quantity) — solid parts only, no cm³/1000 fluid scaling.
    pub cost: Vec<(String, f64)>,
}

/// Any buildable in the game — the full catalog for display/search. The
/// specialized tables (machines/belts) carry solver-relevant detail on top.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Buildable {
    pub class_name: String,
    pub display_name: String,
    /// FG native class, e.g. `FGBuildableAttachmentSplitter`.
    pub native_class: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameData {
    pub build_version: String,
    pub items: BTreeMap<String, Item>,
    pub recipes: BTreeMap<String, Recipe>,
    pub machines: BTreeMap<String, Machine>,
    pub belts: BTreeMap<String, Belt>,
    #[serde(default)]
    pub buildables: BTreeMap<String, Buildable>,
    /// Schematic class → recipe classes it unlocks (W2b unlocked-alt awareness).
    /// Empty when Docs.json ships no FGSchematic section (the trimmed fixture),
    /// so old catalogs load unchanged.
    #[serde(default)]
    pub schematics: BTreeMap<String, Vec<String>>,
    /// Milestone schematic class → cost/tier/name (PR 4 `milestone_gap`).
    /// Populated ONLY for `EST_Milestone` schematics; empty when Docs.json ships
    /// no milestones (the trimmed fixture), so old catalogs load unchanged.
    #[serde(default)]
    pub milestones: BTreeMap<String, Milestone>,
}

/// Decode raw Docs.json bytes: UTF-16LE when BOM'd (real installs), UTF-8 otherwise.
pub fn decode(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// Pull `Desc_X_C`-style class references + Amounts out of an FG item-amount
/// string: `((ItemClass="…'/…/Desc_OreIron.Desc_OreIron_C'",Amount=1),(…))`.
fn parse_item_amounts(raw: &str) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for chunk in raw.split("ItemClass=").skip(1) {
        // The class path ends right before `",Amount` (or `,Amount` in unquoted
        // variants); the class name is the token after the last '.' or '/'.
        let Some(end) = chunk.find(",Amount") else {
            continue;
        };
        let path = chunk[..end].trim_matches(['"', '\'', ')', '(']);
        let class = path
            .trim_end_matches(['\'', '"'])
            .rsplit(['.', '/'])
            .next()
            .unwrap_or_default()
            .trim_matches(['\'', '"']);
        let amount = chunk
            .split("Amount=")
            .nth(1)
            .and_then(|a| {
                a.chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                    .collect::<String>()
                    .parse::<f64>()
                    .ok()
            })
            .unwrap_or(0.0);
        if !class.is_empty() {
            out.push((class.to_string(), amount));
        }
    }
    out
}

/// `("/Game/.../Build_SmelterMk1.Build_SmelterMk1_C")` → `[Build_SmelterMk1_C]`
fn parse_class_list(raw: &str) -> Vec<String> {
    raw.split(['"', '\''])
        .filter(|s| s.contains('/'))
        .filter_map(|path| path.rsplit(['.', '/']).next())
        .filter(|c| !c.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Walk a schematic's `mUnlocks` value (JSON array of unlock objects, or a
/// flat FG string, depending on the Docs.json exporter) and collect every
/// `Recipe_*` class it references, de-duplicated in first-seen order. The
/// `Recipe_` prefix isolates recipe unlocks from item/scanner/inventory
/// unlocks that share the same block. Tolerant of any nesting/shape.
fn collect_recipe_classes(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(raw) => {
            for c in parse_class_list(raw) {
                if c.starts_with("Recipe_") && !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|e| collect_recipe_classes(e, out)),
        Value::Object(o) => o.values().for_each(|e| collect_recipe_classes(e, out)),
        _ => {}
    }
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn f(v: &Value, key: &str) -> f64 {
    v.get(key)
        .and_then(Value::as_str)
        .and_then(|x| x.parse().ok())
        .or_else(|| v.get(key).and_then(Value::as_f64))
        .unwrap_or(0.0)
}

/// Top-down build footprint from an FG `mClearanceData` string: the union of
/// world-space X/Y extents over every CT_Hard `ClearanceBox`, with each box's
/// optional `RelativeTransform` (Translation + quaternion Rotation) applied
/// to all 8 corners — rotations about horizontal axes mix Z into X/Y, so Z
/// participates even though only X/Y extents are reported. CT_Soft boxes are
/// skipped: soft clearance is non-blocking in game, and pad sizing wants the
/// hard envelope. Returned as (width, depth) in meters rounded to one
/// decimal. None when the string holds no parseable hard box (absent key on
/// trimmed fixtures / decor / soft-only classes — honest absence).
fn parse_clearance_footprint(raw: &str) -> Option<(f64, f64)> {
    /// First number after `key` in `s` (`X=-800.000000,…` → -800.0).
    fn axis(s: &str, key: &str) -> Option<f64> {
        s.split(key)
            .nth(1)?
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .collect::<String>()
            .parse()
            .ok()
    }
    /// Innards of the first `key…)` group in `s`: `Min=(` → `X=…,Y=…,Z=…`.
    fn group<'a>(s: &'a str, key: &str) -> Option<&'a str> {
        s.split_once(key)
            .map(|(_, rest)| rest.split(')').next().unwrap_or(rest))
    }
    /// Rotate `v` by the UE quaternion `q` (Docs.json component order
    /// X, Y, Z, W): v' = v + 2·q_vec×(q_vec×v + w·v). Dependency-free —
    /// the only quaternion math in the workspace.
    fn quat_rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
        fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        }
        let qv = [q[0], q[1], q[2]];
        let w = q[3];
        let c = cross(qv, v);
        let t = cross(qv, [c[0] + w * v[0], c[1] + w * v[1], c[2] + w * v[2]]);
        [v[0] + 2.0 * t[0], v[1] + 2.0 * t[1], v[2] + 2.0 * t[2]]
    }
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut boxes = 0usize;
    // Per-entry tokenizer, split on `ClearanceBox=` boundaries. Entries are
    // separated by `,(` — a sequence that never occurs INSIDE one entry —
    // and an entry's `Type=` preamble sits BEFORE its `ClearanceBox=` while
    // its `RelativeTransform=` follows AFTER, so naive whole-string chunking
    // attaches both to the wrong box.
    let parts: Vec<&str> = raw.split("ClearanceBox=").collect();
    for i in 1..parts.len() {
        // `…),(Type=CT_Soft,ClearanceBox=…` — this entry's soft marker is the
        // tail of the PREVIOUS chunk, after the last entry separator.
        let preamble = parts[i - 1].rsplit(",(").next().unwrap_or(parts[i - 1]);
        if preamble.contains("Type=CT_Soft") {
            continue;
        }
        // This entry's own content ends at the next entry separator.
        let entry = parts[i].split(",(").next().unwrap_or(parts[i]);
        let (Some(mn), Some(mx)) = (group(entry, "Min=("), group(entry, "Max=(")) else {
            continue;
        };
        let coords = |g: &str| Some([axis(g, "X=")?, axis(g, "Y=")?, axis(g, "Z=")?]);
        let (Some(lo), Some(hi)) = (coords(mn), coords(mx)) else {
            continue;
        };
        // Optional per-box transform: zero translation / identity rotation
        // when absent (components individually default too — FG omits axes).
        let t = group(entry, "Translation=(");
        let tv = |k: &str| t.and_then(|g| axis(g, k)).unwrap_or(0.0);
        let (tx, ty) = (tv("X="), tv("Y="));
        let r = group(entry, "Rotation=(");
        let rv = |k: &str, d: f64| r.and_then(|g| axis(g, k)).unwrap_or(d);
        let q = [rv("X=", 0.0), rv("Y=", 0.0), rv("Z=", 0.0), rv("W=", 1.0)];
        for cx in [lo[0], hi[0]] {
            for cy in [lo[1], hi[1]] {
                for cz in [lo[2], hi[2]] {
                    let v = quat_rotate(q, [cx, cy, cz]);
                    min_x = min_x.min(v[0] + tx);
                    max_x = max_x.max(v[0] + tx);
                    min_y = min_y.min(v[1] + ty);
                    max_y = max_y.max(v[1] + ty);
                }
            }
        }
        boxes += 1;
    }
    if boxes == 0 {
        return None;
    }
    let meters = |cm: f64| (cm / 10.0).round() / 10.0; // cm → m at one decimal
    Some((meters(max_x - min_x), meters(max_y - min_y)))
}

/// Pseudo-item carried by generator outputs: 1 "item/min" = 1 MW.
/// Power is production (Addendum A2) — the ordinary solver handles it.
pub const POWER_ITEM: &str = "__PowerMW";

const BELT_TIERS: [(&str, u8); 6] = [
    ("Build_ConveyorBeltMk1_C", 1),
    ("Build_ConveyorBeltMk2_C", 2),
    ("Build_ConveyorBeltMk3_C", 3),
    ("Build_ConveyorBeltMk4_C", 4),
    ("Build_ConveyorBeltMk5_C", 5),
    ("Build_ConveyorBeltMk6_C", 6),
];

/// Parse decoded Docs.json text into normalized game data.
pub fn parse_docs(text: &str, build_version: &str) -> Result<GameData, DocsError> {
    let root: Value = serde_json::from_str(text)?;
    let sections = root
        .as_array()
        .ok_or_else(|| DocsError::Shape("top level is not an array".into()))?;

    let mut gd = GameData {
        build_version: build_version.to_string(),
        ..Default::default()
    };
    let mut generator_fuels: Vec<(String, f64, Vec<String>)> = Vec::new();
    // Machines whose draw varies by recipe, and every recipe's raw
    // (constant, factor) pair — joined in a post-pass below, so section
    // ordering in Docs.json never matters.
    let mut variable_power_machines: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    let mut recipe_variable_power: BTreeMap<String, (f64, f64)> = BTreeMap::new();

    for section in sections {
        let native = section
            .get("NativeClass")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let classes = section
            .get("Classes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Match on the FG class name at the end of the native-class path.
        let fg = native
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches('\'');
        // Every FGBuildable* class lands in the display catalog, whatever it
        // is — the app can name and show anything the game data knows about.
        if fg.starts_with("FGBuildable") {
            for c in &classes {
                let b = Buildable {
                    class_name: s(c, "ClassName"),
                    display_name: s(c, "mDisplayName"),
                    native_class: fg.to_string(),
                };
                if !b.class_name.is_empty() {
                    gd.buildables.insert(b.class_name.clone(), b);
                }
            }
        }
        match fg {
            "FGItemDescriptor"
            | "FGResourceDescriptor"
            | "FGItemDescriptorBiomass"
            | "FGItemDescriptorNuclearFuel"
            | "FGEquipmentDescriptor" => {
                for c in &classes {
                    let item = Item {
                        class_name: s(c, "ClassName"),
                        display_name: s(c, "mDisplayName"),
                        form: s(c, "mForm"),
                        stack_size: s(c, "mStackSize"),
                        energy_mj: f(c, "mEnergyValue"),
                        is_resource: fg == "FGResourceDescriptor",
                    };
                    if !item.class_name.is_empty() {
                        gd.items.insert(item.class_name.clone(), item);
                    }
                }
            }
            "FGRecipe" => {
                for c in &classes {
                    let class_name = s(c, "ClassName");
                    let produced_in = parse_class_list(&s(c, "mProducedIn"));
                    let recipe = Recipe {
                        alternate: class_name.starts_with("Recipe_Alternate_"),
                        class_name: class_name.clone(),
                        display_name: s(c, "mDisplayName"),
                        duration_s: f(c, "mManufactoringDuration"),
                        ingredients: parse_item_amounts(&s(c, "mIngredients")),
                        products: parse_item_amounts(&s(c, "mProduct")),
                        produced_in,
                        variable_power_mw: None, // filled by the post-pass below
                    };
                    if !recipe.class_name.is_empty() && !recipe.products.is_empty() {
                        recipe_variable_power.insert(
                            class_name.clone(),
                            (
                                f(c, "mVariablePowerConsumptionConstant"),
                                f(c, "mVariablePowerConsumptionFactor"),
                            ),
                        );
                        gd.recipes.insert(class_name, recipe);
                    }
                }
            }
            "FGBuildableManufacturer" | "FGBuildableManufacturerVariablePower" => {
                let variable = fg == "FGBuildableManufacturerVariablePower";
                for c in &classes {
                    // Variable-power machines (Particle Accelerator, Converter,
                    // Quantum Encoder) ship mPowerConsumption ≈ 0; the honest
                    // planning number is the average of the estimated min/max.
                    // "Mininum" is a genuine Docs.json typo — parse it as-is.
                    let power_mw = if variable {
                        let est = (f(c, "mEstimatedMininumPowerConsumption")
                            + f(c, "mEstimatedMaximumPowerConsumption"))
                            / 2.0;
                        if est > 0.0 {
                            est
                        } else {
                            f(c, "mPowerConsumption")
                        }
                    } else {
                        f(c, "mPowerConsumption")
                    };
                    let m = Machine {
                        class_name: s(c, "ClassName"),
                        display_name: s(c, "mDisplayName"),
                        power_mw,
                        footprint_m: parse_clearance_footprint(&s(c, "mClearanceData")),
                        kind: MachineKind::Manufacturer,
                    };
                    if !m.class_name.is_empty() {
                        if variable {
                            variable_power_machines.insert(m.class_name.clone());
                        }
                        gd.machines.insert(m.class_name.clone(), m);
                    }
                }
            }
            "FGBuildableResourceExtractor" | "FGBuildableWaterPump" => {
                for c in &classes {
                    let m = Machine {
                        class_name: s(c, "ClassName"),
                        display_name: s(c, "mDisplayName"),
                        power_mw: f(c, "mPowerConsumption"),
                        footprint_m: parse_clearance_footprint(&s(c, "mClearanceData")),
                        kind: MachineKind::Extractor {
                            items_per_cycle: f(c, "mItemsPerCycle"),
                            cycle_time_s: f(c, "mExtractCycleTime"),
                        },
                    };
                    if !m.class_name.is_empty() {
                        gd.machines.insert(m.class_name.clone(), m);
                    }
                }
            }
            "FGBuildableGeneratorFuel"
            | "FGBuildableGeneratorNuclear"
            | "FGBuildableGeneratorGeoThermal" => {
                for c in &classes {
                    let class_name = s(c, "ClassName");
                    if class_name.is_empty() {
                        continue;
                    }
                    // Geothermal is fuel-less variable power: mPowerProduction
                    // is 0 and mVariablePowerProductionFactor carries the
                    // normal-purity AVERAGE (200 MW; the game cycles ±50% and
                    // scales by geyser purity, which imports don't know) — an
                    // honest nameplate, and it keeps the 20-generator geo farm
                    // of a real save from reading 0 MW / "IMPORTED WORKS".
                    // Purity-scaled truth needs geyser nodes in the world
                    // snapshot — BACKLOG.
                    let mw = match f(c, "mPowerProduction") {
                        p if p > 0.0 => p,
                        _ => f(c, "mVariablePowerProductionFactor"),
                    };
                    // fuel classes: modern Docs nests them in mFuel[].mFuelClass
                    let mut fuels: Vec<String> = Vec::new();
                    if let Some(list) = c.get("mFuel").and_then(Value::as_array) {
                        for entry in list {
                            let fc = s(entry, "mFuelClass");
                            if !fc.is_empty() {
                                fuels.push(fc);
                            }
                        }
                    }
                    gd.machines.insert(
                        class_name.clone(),
                        Machine {
                            class_name: class_name.clone(),
                            display_name: s(c, "mDisplayName"),
                            power_mw: 0.0, // generators draw nothing; they produce
                            footprint_m: parse_clearance_footprint(&s(c, "mClearanceData")),
                            kind: MachineKind::Generator {
                                power_production_mw: mw,
                            },
                        },
                    );
                    generator_fuels.push((class_name, mw, fuels));
                }
            }
            "FGSchematic" => {
                for c in &classes {
                    let class_name = s(c, "ClassName");
                    if class_name.is_empty() {
                        continue;
                    }
                    let mut recipes: Vec<String> = Vec::new();
                    if let Some(unlocks) = c.get("mUnlocks") {
                        collect_recipe_classes(unlocks, &mut recipes);
                    }
                    gd.schematics.insert(class_name.clone(), recipes);
                    // Milestone metadata (PR 4): only EST_Milestone schematics
                    // carry a buyable tier/cost. mTechTier is a STRING → u32
                    // (skip the milestone if unparseable); mCost is the recipe-
                    // ingredient item-amount form. A milestone with no parseable
                    // cost is skipped (defensive; all real ones have costs).
                    // Cost entries whose ItemClass doesn't resolve to a known
                    // item are dropped in a post-pass below (never guess).
                    if s(c, "mType") == "EST_Milestone" {
                        let Ok(tier) = s(c, "mTechTier").parse::<u32>() else {
                            continue;
                        };
                        let cost = parse_item_amounts(&s(c, "mCost"));
                        if cost.is_empty() {
                            continue;
                        }
                        gd.milestones.insert(
                            class_name,
                            Milestone {
                                display_name: s(c, "mDisplayName"),
                                tier,
                                cost,
                            },
                        );
                    }
                }
            }
            "FGBuildableConveyorBelt" => {
                for c in &classes {
                    let class_name = s(c, "ClassName");
                    let tier = BELT_TIERS
                        .iter()
                        .find(|(n, _)| *n == class_name)
                        .map(|(_, t)| *t);
                    if let Some(tier) = tier {
                        gd.belts.insert(
                            class_name.clone(),
                            Belt {
                                class_name,
                                display_name: s(c, "mDisplayName"),
                                capacity_per_min: f(c, "mSpeed") / 2.0,
                                tier,
                            },
                        );
                    }
                }
            }
            _ => {}
        }
    }

    // Liquids/gases: Docs amounts are liters; normalize authored recipe
    // amounts to m³ (game rates are m³/min). Ordering invariant: this MUST
    // run before burn-recipe synthesis below — fluid mEnergyValue is MJ per
    // m³, so synthesized burn amounts are born in m³/min and must never be
    // divided by 1000.
    // Milestone costs reference item classes; drop any that don't resolve to a
    // known item (never guess). A post-pass so Docs.json section ordering
    // (items before or after schematics) never matters.
    {
        let items = &gd.items;
        for m in gd.milestones.values_mut() {
            m.cost.retain(|(item, _)| items.contains_key(item));
        }
    }

    let liquid_forms = ["RF_LIQUID", "RF_GAS"];
    let is_fluid: std::collections::BTreeSet<String> = gd
        .items
        .values()
        .filter(|i| liquid_forms.contains(&i.form.as_str()))
        .map(|i| i.class_name.clone())
        .collect();
    for r in gd.recipes.values_mut() {
        for (item, amount) in r.ingredients.iter_mut().chain(r.products.iter_mut()) {
            if is_fluid.contains(item) {
                *amount /= 1000.0;
            }
        }
    }

    // Variable-power recipes: draw varies by recipe, not machine, so store the
    // average sustained draw (constant + factor/2 — the ramp midpoint the
    // in-game UI reports) as a per-recipe override. The machine-class gate is
    // load-bearing: ordinary recipes also carry these keys with Docs.json
    // defaults (constant 0, factor 1) and must stay None.
    for r in gd.recipes.values_mut() {
        let Some(&(constant, factor)) = recipe_variable_power.get(&r.class_name) else {
            continue;
        };
        if constant + factor > 0.0
            && r.produced_in
                .iter()
                .any(|m| variable_power_machines.contains(m))
        {
            r.variable_power_mw = Some(constant + factor / 2.0);
        }
    }

    // Synthesize fuel-burn recipes: MW·60 MJ/min ÷ fuel MJ = fuel/min.
    // Runs after fluid normalization (see above) so these recipes keep their
    // already-correct m³/min amounts. Supplemental fluids (water) wait for
    // the pipe network model — noted in DECISIONS.md; the fuel math itself
    // is exact.
    for (gen_class, mw, fuels) in generator_fuels {
        for fuel in fuels {
            let Some(fuel_item) = gd.items.get(&fuel) else {
                continue;
            };
            if fuel_item.energy_mj <= 0.0 {
                continue;
            }
            let per_min = mw * 60.0 / fuel_item.energy_mj;
            let class_name = format!("Recipe_Power_{}_{}", gen_class.trim_end_matches("_C"), fuel);
            gd.recipes.insert(
                class_name.clone(),
                Recipe {
                    alternate: false,
                    class_name,
                    display_name: format!(
                        "{} — {}",
                        gd.machines
                            .get(&gen_class)
                            .map(|m| m.display_name.clone())
                            .unwrap_or_default(),
                        fuel_item.display_name
                    ),
                    duration_s: 60.0,
                    ingredients: vec![(fuel, per_min)],
                    products: vec![(POWER_ITEM.to_string(), mw)],
                    produced_in: vec![gen_class.clone()],
                    variable_power_mw: None,
                },
            );
        }
    }
    // The pseudo power item so names resolve everywhere.
    gd.items.insert(
        POWER_ITEM.to_string(),
        Item {
            class_name: POWER_ITEM.to_string(),
            display_name: "Power".to_string(),
            form: "RF_POWER".to_string(),
            stack_size: String::new(),
            energy_mj: 0.0,
            is_resource: false,
        },
    );

    Ok(gd)
}

/// Extraction ceiling for a claim: items/min for a miner class on a node purity.
pub fn extraction_rate(machine: &Machine, purity: &str, clock: f64) -> f64 {
    let MachineKind::Extractor {
        items_per_cycle,
        cycle_time_s,
    } = &machine.kind
    else {
        return 0.0;
    };
    let base = items_per_cycle / cycle_time_s * 60.0;
    let purity_factor = match purity {
        "impure" => 0.5,
        "pure" => 2.0,
        _ => 1.0,
    };
    base * purity_factor * clock
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_item_amount_strings() {
        let raw = r#"((ItemClass="/Script/Engine.BlueprintGeneratedClass'/Game/FactoryGame/Resource/Parts/IronPlate/Desc_IronPlate.Desc_IronPlate_C'",Amount=6),(ItemClass="/Script/Engine.BlueprintGeneratedClass'/Game/FactoryGame/Resource/Parts/IronScrew/Desc_IronScrew.Desc_IronScrew_C'",Amount=12))"#;
        let parsed = parse_item_amounts(raw);
        assert_eq!(
            parsed,
            vec![
                ("Desc_IronPlate_C".to_string(), 6.0),
                ("Desc_IronScrew_C".to_string(), 12.0)
            ]
        );
    }

    #[test]
    fn decodes_utf16le() {
        let text = "[{\"NativeClass\":\"x\",\"Classes\":[]}]";
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode(&bytes), text);
    }

    #[test]
    fn parses_bundled_fixture() {
        let gd = parse_docs(include_str!("../assets/docs-fixture.json"), "test").unwrap();
        let mf = &gd.recipes["Recipe_ModularFrame_C"];
        assert_eq!(mf.duration_s, 60.0);
        assert_eq!(
            mf.ingredients,
            vec![
                ("Desc_IronPlateReinforced_C".into(), 3.0),
                ("Desc_IronRod_C".into(), 12.0)
            ]
        );
        assert_eq!(mf.products, vec![("Desc_ModularFrame_C".into(), 2.0)]);
        assert_eq!(mf.produced_in, vec!["Build_AssemblerMk1_C".to_string()]);
        assert!(!mf.alternate);
        assert!(gd.recipes["Recipe_Alternate_Screw_C"].alternate);
        assert_eq!(gd.belts["Build_ConveyorBeltMk3_C"].capacity_per_min, 270.0);
        // mClearanceData on the fixture constructor → real derived footprint;
        // classes without the key stay honestly None.
        assert_eq!(
            gd.machines["Build_ConstructorMk1_C"].footprint_m,
            Some((8.0, 10.0)),
            "constructor hard box 800×1000 cm → 8×10 m (CT_Soft attachments excluded)"
        );
        assert_eq!(gd.machines["Build_SmelterMk1_C"].footprint_m, None);
        let miner = &gd.machines["Build_MinerMk1_C"];
        // Extractor arm wires footprint_m too: 600×1400 cm → 6×14 m.
        assert_eq!(miner.footprint_m, Some((6.0, 14.0)));
        assert_eq!(extraction_rate(miner, "normal", 1.0), 60.0);
        assert_eq!(extraction_rate(miner, "pure", 1.0), 120.0);
        assert_eq!(extraction_rate(miner, "impure", 1.0), 30.0);
        // full buildable catalog: everything FGBuildable* is displayable
        assert_eq!(
            gd.buildables["Build_ConveyorAttachmentSplitter_C"].display_name,
            "Conveyor Splitter"
        );
        assert_eq!(
            gd.buildables["Build_ConveyorAttachmentMerger_C"].display_name,
            "Conveyor Merger"
        );
        assert!(gd.buildables.contains_key("Build_StorageContainerMk1_C"));
        assert!(
            gd.buildables.contains_key("Build_ConveyorBeltMk3_C"),
            "belts are buildables too"
        );
        assert!(
            gd.buildables.contains_key("Build_SmelterMk1_C"),
            "machines are buildables too"
        );
        assert!(gd.buildables.contains_key("Build_ConveyorLiftMk2_C"));
        // generator fuel synthesis: 75 MW · 60 ÷ 300 MJ = 15 coal/min
        let gen = &gd.machines["Build_GeneratorCoal_C"];
        assert!(
            matches!(gen.kind, MachineKind::Generator { power_production_mw } if power_production_mw == 75.0)
        );
        assert_eq!(gen.power_mw, 0.0, "generators draw nothing");
        // Generator arm wires footprint_m too (real verbatim string incl. a
        // translated stack box that stays inside the base hull + CT_Soft
        // attachments): union 1000×2600 cm → 10×26 m.
        assert_eq!(gen.footprint_m, Some((10.0, 26.0)));
        let burn = gd
            .recipes
            .values()
            .find(|r| r.produced_in.contains(&"Build_GeneratorCoal_C".to_string()))
            .expect("synthesized burn recipe");
        assert_eq!(burn.ingredients, vec![("Desc_Coal_C".to_string(), 15.0)]);
        assert_eq!(burn.products, vec![(POWER_ITEM.to_string(), 75.0)]);
        assert_eq!(gd.items[POWER_ITEM].display_name, "Power");
    }

    #[test]
    fn footprint_unions_all_clearance_boxes() {
        // Verbatim Build_QuantumEncoder_C mClearanceData from the real
        // 1.0.0.5 Docs.json — six boxes, incl. two CT_Soft attachments. The
        // soft boxes are EXCLUDED from the union, but both happen to sit
        // inside the hard envelope, so the answer is unchanged: hard union
        // X −1100…1100 cm → 22.0 m wide, Y −2700…2300 cm → 50.0 m deep.
        let qe = "((ClearanceBox=(Min=(X=-800.000000,Y=-2100.000000,Z=0.000000),Max=(X=800.000000,Y=-1000.000000,Z=450.000000),IsValid=True)),(ClearanceBox=(Min=(X=-1100.000000,Y=-1000.000000,Z=0.000000),Max=(X=1100.000000,Y=1000.000000,Z=1400.000000),IsValid=True)),(ClearanceBox=(Min=(X=-450.000000,Y=-2700.000000,Z=0.000000),Max=(X=450.000000,Y=-2100.000000,Z=500.000000),IsValid=True)),(ClearanceBox=(Min=(X=-600.000000,Y=1000.000000,Z=0.000000),Max=(X=600.000000,Y=2300.000000,Z=500.000000),IsValid=True)),(Type=CT_Soft,ClearanceBox=(Min=(X=-50.000000,Y=-30.000000,Z=0.000000),Max=(X=30.000000,Y=30.000000,Z=270.000000),IsValid=True),RelativeTransform=(Translation=(X=860.000000,Y=-2030.000000,Z=250.000000)),ExcludeForSnapping=True),(Type=CT_Soft,ClearanceBox=(Min=(X=-200.000000,Y=-2000.000000,Z=0.000000),Max=(X=500.000000,Y=-1700.000000,Z=270.000000),IsValid=True),RelativeTransform=(Translation=(X=-690.000000,Y=20.000000,Z=450.000000)),ExcludeForSnapping=True))";
        assert_eq!(parse_clearance_footprint(qe), Some((22.0, 50.0)));
    }

    #[test]
    fn footprint_single_box_rounds_to_one_decimal() {
        // One box, non-round centimeters: 812.5 cm × 1993 cm → 8.1 × 19.9 m.
        let one = "((ClearanceBox=(Min=(X=-406.250000,Y=-996.500000,Z=0.000000),Max=(X=406.250000,Y=996.500000,Z=450.000000),IsValid=True)))";
        assert_eq!(parse_clearance_footprint(one), Some((8.1, 19.9)));
        // An 815 cm span sits exactly on the .05 m boundary: it ROUNDS to
        // 8.2 — truncation would read 8.1 (pins round-vs-trunc).
        let mid = "((ClearanceBox=(Min=(X=-407.500000,Y=-500.000000,Z=0.000000),Max=(X=407.500000,Y=500.000000,Z=450.000000),IsValid=True)))";
        assert_eq!(parse_clearance_footprint(mid), Some((8.2, 10.0)));
    }

    #[test]
    fn footprint_applies_box_translation() {
        // Verbatim Build_ManufacturerMk1_C mClearanceData from the real
        // 1.0.0.5 Docs.json. The second hard box carries Translation Y=-700:
        // applied, the union is X −900…900 × Y −1100…900 cm → 18.0 × 20.0 m.
        // Ignoring the transform read 18 × 13 — the BUILD SHEET told players
        // to pour a foundation pad 7 m short.
        let mfr = "((ClearanceBox=(Min=(X=-900.000000,Y=-300.000000,Z=0.000000),Max=(X=900.000000,Y=900.000000,Z=1100.000000),IsValid=True)),(ClearanceBox=(Min=(X=-900.000000,Y=-400.000000,Z=-400.000000),Max=(X=900.000000,Y=400.000000,Z=-20.000000),IsValid=True),RelativeTransform=(Translation=(X=0.000000,Y=-700.000000,Z=400.000000))),(Type=CT_Soft,ClearanceBox=(Min=(X=-250.000000,Y=200.000000,Z=-400.000000),Max=(X=0.000000,Y=350.000000,Z=-70.000000),IsValid=True),RelativeTransform=(Translation=(X=-500.000000,Y=250.000000,Z=1529.104455)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-400.000000,Y=100.000000,Z=-400.000000),Max=(X=400.000000,Y=400.000000,Z=125.133385),IsValid=True),RelativeTransform=(Translation=(X=0.000000,Y=-693.911698,Z=781.658355)),ExcludeForSnapping=True))";
        assert_eq!(parse_clearance_footprint(mfr), Some((18.0, 20.0)));
    }

    #[test]
    fn footprint_applies_box_rotation() {
        // Verbatim Build_HadronCollider_C mClearanceData from the real
        // 1.0.0.5 Docs.json. The ring segments rotate about the horizontal Y
        // axis, so each segment's long local X extent points into world Z —
        // ignoring rotation piled them all onto X and read 52 × 22 garbage.
        // Transform-aware union: 37.0 × 27.0 m. Wiki build dims are 24 × 38
        // (the building body); the clearance envelope legitimately exceeds
        // build dims, and clearance is what pad planning wants.
        let hc = "((ClearanceBox=(Min=(X=900.000000,Y=300.000000,Z=0.000000),Max=(X=1900.000000,Y=1300.000000,Z=800.000000),IsValid=True)),(ClearanceBox=(Min=(X=100.000000,Y=100.000000,Z=0.000000),Max=(X=500.000000,Y=400.000000,Z=1200.000000),IsValid=True),RelativeTransform=(Rotation=(X=0.000000,Y=-0.258819,Z=0.000000,W=0.965926),Translation=(X=1150.000000,Y=450.000000,Z=1600.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=100.000000,Y=100.000000,Z=-400.000000),Max=(X=500.000000,Y=400.000000,Z=400.000000),IsValid=True),RelativeTransform=(Rotation=(X=0.000000,Y=-0.573576,Z=0.000000,W=0.819152),Translation=(X=400.000000,Y=450.000000,Z=2600.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=100.000000,Y=100.000000,Z=-400.000000),Max=(X=500.000000,Y=400.000000,Z=400.000000),IsValid=True),RelativeTransform=(Rotation=(X=-0.000000,Y=-0.707107,Z=-0.000000,W=0.707107),Translation=(X=0.000000,Y=450.000000,Z=2700.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=100.000000,Y=100.000000,Z=-300.000000),Max=(X=500.000000,Y=400.000000,Z=500.000000),IsValid=True),RelativeTransform=(Rotation=(X=-0.000000,Y=0.573576,Z=0.000000,W=0.819152),Translation=(X=-900.000000,Y=450.000000,Z=3150.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=100.000000,Y=100.000000,Z=-400.000000),Max=(X=500.000000,Y=400.000000,Z=600.000000),IsValid=True),RelativeTransform=(Rotation=(X=-0.000000,Y=0.258819,Z=0.000000,W=0.965926),Translation=(X=-1600.000000,Y=450.000000,Z=2400.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-300.000000,Y=100.000000,Z=50.000000),Max=(X=200.000000,Y=400.000000,Z=2000.000000),IsValid=True),RelativeTransform=(Translation=(X=-1450.000000,Y=450.000000,Z=0.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-300.000000,Y=100.000000,Z=-600.000000),Max=(X=200.000000,Y=400.000000,Z=400.000000),IsValid=True),RelativeTransform=(Rotation=(X=0.000000,Y=-0.300706,Z=0.000000,W=0.953717),Translation=(X=-1200.000000,Y=450.000000,Z=850.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-1800.000000,Y=0.000000,Z=0.000000),Max=(X=1500.000000,Y=500.000000,Z=400.000000),IsValid=True),RelativeTransform=(Translation=(X=0.000000,Y=450.000000,Z=0.000000))),(ClearanceBox=(Min=(X=-350.000000,Y=-350.000000,Z=-900.000000),Max=(X=350.000000,Y=350.000000,Z=250.000000),IsValid=True),RelativeTransform=(Translation=(X=1400.000000,Y=700.000000,Z=1500.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-350.000000,Y=-100.000000,Z=-100.000000),Max=(X=150.000000,Y=100.000000,Z=76.893700),IsValid=True),RelativeTransform=(Rotation=(X=0.000000,Y=-0.000000,Z=0.258819,W=0.965926),Translation=(X=769.598034,Y=297.799257,Z=2270.920364)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=100.000000,Y=-100.000000,Z=-100.000000),Max=(X=600.000000,Y=100.000000,Z=76.893700),IsValid=True),RelativeTransform=(Rotation=(X=-0.066987,Y=0.250000,Z=0.250000,W=0.933013),Translation=(X=769.598034,Y=297.799257,Z=2343.817080)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-1000.000000,Y=-100.000000,Z=-100.000000),Max=(X=-400.000000,Y=100.000000,Z=76.893700),IsValid=True),RelativeTransform=(Rotation=(X=0.066987,Y=-0.250000,Z=0.250000,W=0.933013),Translation=(X=769.598034,Y=297.799257,Z=2440.172124)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-2100.000000,Y=-100.000000,Z=-100.000000),Max=(X=-1600.000000,Y=100.000000,Z=76.893700),IsValid=True),RelativeTransform=(Rotation=(X=0.129409,Y=-0.482963,Z=0.224144,W=0.836516),Translation=(X=769.598034,Y=297.799257,Z=3350.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-2800.000000,Y=-100.000000,Z=-100.000000),Max=(X=-1800.000000,Y=100.000000,Z=200.000000),IsValid=True),RelativeTransform=(Rotation=(X=0.183012,Y=-0.683013,Z=0.183012,W=0.683013),Translation=(X=-100.000000,Y=-200.000000,Z=3350.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-2000.000000,Y=-100.000000,Z=-800.000000),Max=(X=-1600.000000,Y=100.000000,Z=270.000000),IsValid=True),RelativeTransform=(Rotation=(X=0.117795,Y=-0.439620,Z=0.230459,W=0.860086),Translation=(X=769.598034,Y=297.799257,Z=2334.640177)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-1200.000000,Y=-100.000000,Z=-280.000000),Max=(X=1000.000000,Y=100.000000,Z=270.000000),IsValid=True),RelativeTransform=(Rotation=(X=-0.000001,Y=-0.000000,Z=0.258820,W=0.965926),Translation=(X=769.598034,Y=297.799257,Z=300.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-1100.000000,Y=-200.000000,Z=0.000000),Max=(X=-2300.000000,Y=200.000000,Z=200.000000),IsValid=True),RelativeTransform=(Rotation=(X=-0.000001,Y=-0.000000,Z=0.258820,W=0.965926),Translation=(X=769.598034,Y=297.799257,Z=0.000000))),(ClearanceBox=(Min=(X=-1100.000000,Y=-100.000000,Z=300.000000),Max=(X=-1500.000000,Y=100.000000,Z=500.000000),IsValid=True),RelativeTransform=(Rotation=(X=0.099045,Y=-0.369644,Z=0.239118,W=0.892399),Translation=(X=769.598034,Y=297.799257,Z=900.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-1200.000000,Y=-100.000000,Z=1270.000000),Max=(X=-1600.000000,Y=100.000000,Z=1400.000000),IsValid=True),RelativeTransform=(Rotation=(X=0.099045,Y=-0.369644,Z=0.239118,W=0.892399),Translation=(X=769.598034,Y=297.799257,Z=900.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-3300.000000,Y=-100.000000,Z=1000.000000),Max=(X=-2600.000000,Y=100.000000,Z=1200.000000),IsValid=True),RelativeTransform=(Rotation=(X=0.183012,Y=-0.683013,Z=0.183012,W=0.683013),Translation=(X=-100.000000,Y=-200.000000,Z=3350.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-1300.000000,Y=-100.000000,Z=-280.000000),Max=(X=-1900.000000,Y=100.000000,Z=-150.000000),IsValid=True),RelativeTransform=(Rotation=(X=-0.000001,Y=-0.000000,Z=0.258820,W=0.965926),Translation=(X=769.598034,Y=297.799257,Z=1180.000000)),ExcludeForSnapping=True),(ClearanceBox=(Min=(X=-700.000000,Y=-900.000000,Z=0.000000),Max=(X=600.000000,Y=800.000000,Z=520.000000),IsValid=True),RelativeTransform=(Translation=(X=1300.000000,Y=-500.000000,Z=0.000000))),(Type=CT_Soft,ClearanceBox=(Min=(X=-30.000000,Y=-30.000000,Z=0.000000),Max=(X=30.000000,Y=30.000000,Z=130.000000),IsValid=True),RelativeTransform=(Translation=(X=1830.000000,Y=-875.000000,Z=520.000000)),ExcludeForSnapping=True))";
        assert_eq!(parse_clearance_footprint(hc), Some((37.0, 27.0)));
    }

    #[test]
    fn footprint_rotation_convention_pinned_by_quarter_turn() {
        // Synthetic 90° yaw: a 2000×200 cm box, quarter turn about Z (UE
        // quat X,Y,Z,W = (0,0,√½,√½)), then Translation Y=+1000. (x,y) maps
        // to (−y,x): the long side lands on Y — X extent 200 cm → 2.0 m,
        // Y extent −1000…1000 cm shifted to 0…2000 cm → 20.0 m. Kills
        // quaternion-convention mutants (W-first, yaw-ignored, unrotated)
        // without a big verbatim vector.
        let syn = "((ClearanceBox=(Min=(X=-1000.000000,Y=-100.000000,Z=0.000000),Max=(X=1000.000000,Y=100.000000,Z=200.000000),IsValid=True),RelativeTransform=(Rotation=(X=0.0000000,Y=0.0000000,Z=0.7071068,W=0.7071068),Translation=(Y=1000.000000))))";
        assert_eq!(parse_clearance_footprint(syn), Some((2.0, 20.0)));
    }

    #[test]
    fn footprint_excludes_soft_boxes() {
        // A hard 800×1000 box plus a CT_Soft box TRANSLATED to poke 6 m past
        // the hard hull. Soft clearance is non-blocking in game (walkway /
        // attachment zones) — pad sizing wants the hard envelope only, so
        // the union stays 8.0 × 10.0 m.
        let raw = "((ClearanceBox=(Min=(X=-400.000000,Y=-500.000000,Z=0.000000),Max=(X=400.000000,Y=500.000000,Z=600.000000),IsValid=True)),(Type=CT_Soft,ClearanceBox=(Min=(X=-100.000000,Y=-100.000000,Z=0.000000),Max=(X=100.000000,Y=100.000000,Z=100.000000),IsValid=True),RelativeTransform=(Translation=(X=900.000000,Y=0.000000,Z=0.000000)),ExcludeForSnapping=True))";
        assert_eq!(parse_clearance_footprint(raw), Some((8.0, 10.0)));
    }

    #[test]
    fn footprint_none_when_clearance_absent() {
        assert_eq!(parse_clearance_footprint(""), None);
        assert_eq!(parse_clearance_footprint("()"), None);
        // A machine class without the key parses with footprint_m: None —
        // honest absence (trimmed fixtures), never an invented 0×0.
        let text = r#"[
          {
            "NativeClass": "/Script/CoreUObject.Class'/Script/FactoryGame.FGBuildableManufacturer'",
            "Classes": [
              { "ClassName": "Build_Bare_C", "mDisplayName": "Bare", "mPowerConsumption": "4.0" }
            ]
          }
        ]"#;
        let gd = parse_docs(text, "test").unwrap();
        assert_eq!(gd.machines["Build_Bare_C"].footprint_m, None);
    }

    #[test]
    fn geothermal_parses_as_generator_at_variable_average() {
        // Fuel-less variable-power generator: mPowerProduction is 0 in the
        // real file and mVariablePowerProductionFactor carries the 200 MW
        // normal-purity average. Missing this class made a real save's geo
        // farm read 0 MW and name its clusters "IMPORTED WORKS".
        let text = r#"[
          {
            "NativeClass": "/Script/CoreUObject.Class'/Script/FactoryGame.FGBuildableGeneratorGeoThermal'",
            "Classes": [
              {
                "ClassName": "Build_GeneratorGeoThermal_C",
                "mDisplayName": "Geothermal Generator",
                "mPowerProduction": "0.000000",
                "mVariablePowerProductionFactor": "200.000000"
              }
            ]
          }
        ]"#;
        let gd = parse_docs(text, "test").unwrap();
        let m = &gd.machines["Build_GeneratorGeoThermal_C"];
        assert!(
            matches!(m.kind, MachineKind::Generator { power_production_mw } if power_production_mw == 200.0),
            "geothermal is a generator at the variable-power average"
        );
        assert_eq!(m.display_name, "Geothermal Generator");
    }

    #[test]
    fn parses_fgschematic_recipe_unlocks() {
        // One schematic unlocking a recipe (plus a non-recipe unlock that must
        // be ignored). mUnlocks as a JSON array of unlock objects — the modern
        // Docs.json shape. The recipe-class ref uses the standard FG path form.
        let text = r#"[
          {
            "NativeClass": "/Script/CoreUObject.Class'/Script/FactoryGame.FGSchematic'",
            "Classes": [
              {
                "ClassName": "Schematic_TestAlt_C",
                "mUnlocks": [
                  { "mRecipes": "((/Script/Engine.BlueprintGeneratedClass'/Game/FactoryGame/Recipes/Alternate/Recipe_Alternate_Screw.Recipe_Alternate_Screw_C'))" },
                  { "mInventorySlotsToUnlock": 1 }
                ]
              }
            ]
          }
        ]"#;
        let gd = parse_docs(text, "test").unwrap();
        assert_eq!(
            gd.schematics.get("Schematic_TestAlt_C"),
            Some(&vec!["Recipe_Alternate_Screw_C".to_string()]),
            "recipe unlock is captured; the slot-unlock is ignored"
        );
    }

    #[test]
    fn resource_descriptors_carry_is_resource() {
        // World-sourced raws must be distinguishable from craftables: the real
        // catalog produces water via Unpackage Water, and a planner that treats
        // water as craftable recurses through the packaging pair forever.
        let gd = parse_docs(include_str!("../assets/docs-fixture.json"), "test").unwrap();
        assert!(
            gd.items["Desc_OreIron_C"].is_resource,
            "ore is a raw resource"
        );
        assert!(
            gd.items["Desc_LiquidOil_C"].is_resource,
            "crude oil is a raw resource"
        );
        assert!(
            !gd.items["Desc_IronIngot_C"].is_resource,
            "an ingot is a craftable, not a world resource"
        );
    }

    #[test]
    fn fixture_without_fgschematic_yields_empty_schematics() {
        // The trimmed fixture ships no FGSchematic section — the map stays
        // empty and the catalog loads unchanged (tolerant default).
        let gd = parse_docs(include_str!("../assets/docs-fixture.json"), "test").unwrap();
        assert!(gd.schematics.is_empty());
        // No milestones parse from a fixture with no FGSchematic section — the
        // family stays silent everywhere (byte-identical to before PR 4).
        assert!(gd.milestones.is_empty());
    }

    #[test]
    fn parses_est_milestone_tier_and_cost() {
        // An EST_Milestone schematic → tier + cost pairs; the recipe-ingredient
        // item-amount parser accepts mCost as-is (same ItemClass=/Amount= form).
        let text = r#"[
          {
            "NativeClass": "/Script/CoreUObject.Class'/Script/FactoryGame.FGItemDescriptor'",
            "Classes": [
              { "ClassName": "Desc_IronPlate_C", "mDisplayName": "Iron Plate", "mForm": "RF_SOLID", "mStackSize": "SS_MEDIUM" },
              { "ClassName": "Desc_Wire_C", "mDisplayName": "Wire", "mForm": "RF_SOLID", "mStackSize": "SS_MEDIUM" }
            ]
          },
          {
            "NativeClass": "/Script/CoreUObject.Class'/Script/FactoryGame.FGSchematic'",
            "Classes": [
              {
                "ClassName": "Schematic_3-1_C",
                "mDisplayName": "Coal Power",
                "mType": "EST_Milestone",
                "mTechTier": "3",
                "mCost": "((ItemClass=\"/Script/Engine.BlueprintGeneratedClass'/Game/FactoryGame/Resource/Parts/IronPlate/Desc_IronPlate.Desc_IronPlate_C'\",Amount=20),(ItemClass=\"/Script/Engine.BlueprintGeneratedClass'/Game/FactoryGame/Resource/Parts/Wire/Desc_Wire.Desc_Wire_C'\",Amount=10))"
              }
            ]
          }
        ]"#;
        let gd = parse_docs(text, "test").unwrap();
        let m = gd
            .milestones
            .get("Schematic_3-1_C")
            .expect("EST_Milestone lands in milestones");
        assert_eq!(m.display_name, "Coal Power");
        assert_eq!(m.tier, 3);
        assert_eq!(
            m.cost,
            vec![
                ("Desc_IronPlate_C".to_string(), 20.0),
                ("Desc_Wire_C".to_string(), 10.0),
            ]
        );
        // The schematic still lands in the unchanged recipe-unlock map (empty
        // unlocks here — the two maps are independent).
        assert!(gd.schematics.contains_key("Schematic_3-1_C"));
    }

    #[test]
    fn non_milestone_schematic_types_are_not_milestones() {
        // An EST_Alternate schematic carries a cost/tier shape too, but is NOT a
        // tier milestone — it must never land in `milestones`.
        let text = r#"[
          {
            "NativeClass": "/Script/CoreUObject.Class'/Script/FactoryGame.FGItemDescriptor'",
            "Classes": [
              { "ClassName": "Desc_IronPlate_C", "mDisplayName": "Iron Plate", "mForm": "RF_SOLID", "mStackSize": "SS_MEDIUM" }
            ]
          },
          {
            "NativeClass": "/Script/CoreUObject.Class'/Script/FactoryGame.FGSchematic'",
            "Classes": [
              {
                "ClassName": "Schematic_Alternate_Test_C",
                "mDisplayName": "Alternate: Test",
                "mType": "EST_Alternate",
                "mTechTier": "0",
                "mCost": "((ItemClass=\"/Game/FactoryGame/Resource/Parts/IronPlate/Desc_IronPlate.Desc_IronPlate_C'\",Amount=5))"
              }
            ]
          }
        ]"#;
        let gd = parse_docs(text, "test").unwrap();
        assert!(gd.milestones.is_empty(), "EST_Alternate is not a milestone");
        // still a normal schematic (recipe-unlock map)
        assert!(gd.schematics.contains_key("Schematic_Alternate_Test_C"));
    }

    #[test]
    fn milestone_cost_drops_unknown_item_entries() {
        // A cost entry naming an item the catalog doesn't carry is dropped
        // without panic — never a guessed item. The known entry survives.
        let text = r#"[
          {
            "NativeClass": "/Script/CoreUObject.Class'/Script/FactoryGame.FGItemDescriptor'",
            "Classes": [
              { "ClassName": "Desc_IronPlate_C", "mDisplayName": "Iron Plate", "mForm": "RF_SOLID", "mStackSize": "SS_MEDIUM" }
            ]
          },
          {
            "NativeClass": "/Script/CoreUObject.Class'/Script/FactoryGame.FGSchematic'",
            "Classes": [
              {
                "ClassName": "Schematic_9-9_C",
                "mDisplayName": "Mystery Tier",
                "mType": "EST_Milestone",
                "mTechTier": "9",
                "mCost": "((ItemClass=\"/Game/.../Desc_IronPlate.Desc_IronPlate_C'\",Amount=20),(ItemClass=\"/Game/.../Desc_Unobtainium.Desc_Unobtainium_C'\",Amount=99))"
              }
            ]
          }
        ]"#;
        let gd = parse_docs(text, "test").unwrap();
        let m = gd
            .milestones
            .get("Schematic_9-9_C")
            .expect("milestone parsed");
        assert_eq!(
            m.cost,
            vec![("Desc_IronPlate_C".to_string(), 20.0)],
            "the unknown-item entry is dropped, the known one survives"
        );
    }

    #[test]
    fn variable_power_machines_get_average_draw() {
        let gd = parse_docs(include_str!("../assets/docs-fixture.json"), "test").unwrap();
        // Machine-level estimate: (250 + 750) / 2, not the ~0 mPowerConsumption.
        let pa = &gd.machines["Build_HadronCollider_C"];
        assert_eq!(pa.power_mw, 500.0);
        assert!(matches!(pa.kind, MachineKind::Manufacturer));
        // Recipe-level average override: constant + factor/2.
        assert_eq!(
            gd.recipes["Recipe_Diamond_C"].variable_power_mw,
            Some(500.0)
        );
        // A hungrier recipe on the same machine beats the machine estimate.
        assert_eq!(
            gd.recipes["Recipe_DarkMatter_C"].variable_power_mw,
            Some(1000.0)
        );
        // Ordinary recipes carry the Docs.json default keys (constant 0,
        // factor 1) but are NOT produced in a variable-power machine — the
        // machine-class gate keeps them at None.
        assert_eq!(gd.recipes["Recipe_IngotIron_C"].variable_power_mw, None);
        // Fixed-power machines are untouched.
        assert_eq!(gd.machines["Build_SmelterMk1_C"].power_mw, 4.0);
    }

    #[test]
    fn liquid_fuel_burn_recipe_is_m3_per_min() {
        let gd = parse_docs(include_str!("../assets/docs-fixture.json"), "test").unwrap();
        // Authored fluid recipes: Docs stores liters; parse normalizes to m³.
        let fuel = &gd.recipes["Recipe_LiquidFuel_C"];
        assert_eq!(
            fuel.ingredients,
            vec![("Desc_LiquidOil_C".to_string(), 6.0)]
        );
        assert_eq!(fuel.products, vec![("Desc_LiquidFuel_C".to_string(), 4.0)]);
        // Synthesized burn recipes are computed from mEnergyValue, which for
        // fluids is MJ per m³ — so they are born in m³/min and must NOT go
        // through the liter→m³ division: 250 MW · 60 ÷ 750 MJ/m³ = 20 m³/min.
        let burn = gd
            .recipes
            .values()
            .find(|r| r.produced_in.contains(&"Build_GeneratorFuel_C".to_string()))
            .expect("synthesized fuel-generator burn recipe");
        assert_eq!(
            burn.ingredients,
            vec![("Desc_LiquidFuel_C".to_string(), 20.0)]
        );
        assert_eq!(burn.products, vec![(POWER_ITEM.to_string(), 250.0)]);
    }
}
