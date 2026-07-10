//! Bundled static world snapshot (SDD §3): resource node positions/purities.
//! Saves don't contain node metadata, so this ships with the app, versioned.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub id: String,
    pub name: String,
    pub label_x: f64,
    pub label_y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldNode {
    pub id: String,
    /// Item class, e.g. `Desc_OreIron_C`.
    pub item: String,
    /// pure | normal | impure
    pub purity: String,
    pub x: f64,
    pub y: f64,
    pub region: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldSnapshot {
    pub version: u32,
    pub source: String,
    pub bounds: Bounds,
    pub regions: Vec<Region>,
    pub nodes: Vec<WorldNode>,
}

pub const BUNDLED: &str = include_str!("../assets/world-nodes.json");

pub fn bundled() -> WorldSnapshot {
    serde_json::from_str(BUNDLED).expect("bundled world-nodes.json must parse")
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_snapshot_parses_and_is_plausible() {
        let snap = super::bundled();
        assert_eq!(snap.version, 1);
        assert!(snap.nodes.len() >= 20);
        assert!(snap.regions.iter().any(|r| r.name == "GRASS FIELDS"));
        for n in &snap.nodes {
            assert!(
                ["pure", "normal", "impure"].contains(&n.purity.as_str()),
                "{}",
                n.id
            );
            assert!(
                snap.regions.iter().any(|r| r.id == n.region),
                "{} region",
                n.id
            );
            assert!(n.x >= snap.bounds.min_x && n.x <= snap.bounds.max_x);
            assert!(n.y >= snap.bounds.min_y && n.y <= snap.bounds.max_y);
        }
    }
}
