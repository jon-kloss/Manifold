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
                    };
                    if !recipe.class_name.is_empty() && !recipe.products.is_empty() {
                        gd.recipes.insert(class_name, recipe);
                    }
                }
            }
            "FGBuildableManufacturer" | "FGBuildableManufacturerVariablePower" => {
                for c in &classes {
                    let m = Machine {
                        class_name: s(c, "ClassName"),
                        display_name: s(c, "mDisplayName"),
                        power_mw: f(c, "mPowerConsumption"),
                        kind: MachineKind::Manufacturer,
                    };
                    if !m.class_name.is_empty() {
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
            "FGBuildableGeneratorFuel" | "FGBuildableGeneratorNuclear" => {
                for c in &classes {
                    let class_name = s(c, "ClassName");
                    if class_name.is_empty() {
                        continue;
                    }
                    let mw = f(c, "mPowerProduction");
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
                            kind: MachineKind::Generator {
                                power_production_mw: mw,
                            },
                        },
                    );
                    generator_fuels.push((class_name, mw, fuels));
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

    // Synthesize fuel-burn recipes: MW·60 MJ/min ÷ fuel MJ = fuel/min.
    // Supplemental fluids (water) wait for the pipe network model — noted in
    // DECISIONS.md; the fuel math itself is exact.
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
        },
    );

    // Liquids/gases: Docs amounts are liters; normalize to m³ (game rates are m³/min).
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
        let miner = &gd.machines["Build_MinerMk1_C"];
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
        let burn = gd
            .recipes
            .values()
            .find(|r| r.produced_in.contains(&"Build_GeneratorCoal_C".to_string()))
            .expect("synthesized burn recipe");
        assert_eq!(burn.ingredients, vec![("Desc_Coal_C".to_string(), 15.0)]);
        assert_eq!(burn.products, vec![(POWER_ITEM.to_string(), 75.0)]);
        assert_eq!(gd.items[POWER_ITEM].display_name, "Power");
    }
}
