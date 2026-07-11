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

/// Surface point where a cave node is actually reached from — routes and belt
/// runs to a cave node must go via here, not the node's overhead x/y.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Entrance {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub z: f64,
}

fn zone_surface() -> String {
    "surface".into()
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
    /// Elevation in meters (defaults 0 for older snapshots).
    #[serde(default)]
    pub z: f64,
    /// surface | cave — cave nodes render distinctly and warn until routed
    /// via their entrance.
    #[serde(default = "zone_surface")]
    pub zone: String,
    /// Present only for cave nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrance: Option<Entrance>,
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
        let mut caves = 0;
        for n in &snap.nodes {
            assert!(
                ["pure", "normal", "impure"].contains(&n.purity.as_str()),
                "{}",
                n.id
            );
            assert!(
                ["surface", "cave"].contains(&n.zone.as_str()),
                "{} zone",
                n.id
            );
            if n.zone == "cave" {
                caves += 1;
                assert!(n.entrance.is_some(), "{} cave node needs an entrance", n.id);
                assert!(n.z < 0.0, "{} cave nodes sit below their entrance", n.id);
            } else {
                assert!(n.entrance.is_none(), "{} surface node with entrance", n.id);
            }
            assert!(
                snap.regions.iter().any(|r| r.id == n.region),
                "{} region",
                n.id
            );
            assert!(n.x >= snap.bounds.min_x && n.x <= snap.bounds.max_x);
            assert!(n.y >= snap.bounds.min_y && n.y <= snap.bounds.max_y);
        }
        assert!(caves >= 1, "snapshot should include at least one cave node");
    }

    #[test]
    fn pre_elevation_snapshots_still_load() {
        // Older snapshots carry no z/zone/entrance — defaults must apply.
        let n: super::WorldNode = serde_json::from_str(
            r#"{"id":"t","item":"Desc_OreIron_C","purity":"pure","x":1.0,"y":2.0,"region":"grass-fields"}"#,
        )
        .unwrap();
        assert_eq!(n.z, 0.0);
        assert_eq!(n.zone, "surface");
        assert!(n.entrance.is_none());
    }
}
