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

fn node_type_default() -> String {
    "node".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldNode {
    pub id: String,
    /// Item class, e.g. `Desc_OreIron_C`. For geysers this is the sentinel
    /// `Desc_Geyser_C` (a siting point for a Geothermal Generator, not an
    /// extractable resource).
    pub item: String,
    /// pure | normal | impure
    pub purity: String,
    /// node | geyser | fracking-satellite. `node` = a plain miner/oil-pump
    /// resource node; `geyser` = a geothermal siting point; `fracking-satellite`
    /// = one activated satellite of a resource well (see `well`). Defaults to
    /// `node` for pre-v3 snapshots.
    #[serde(default = "node_type_default")]
    pub node_type: String,
    /// Present only for `fracking-satellite` nodes: the reconstructed resource
    /// well this satellite belongs to (all satellites sharing a well are fed by
    /// one Resource Well Pressurizer). Reconstructed by proximity in
    /// `gen-world-nodes.py` — the vendor dataset carries no core grouping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub well: Option<String>,
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

impl WorldNode {
    /// True for a plain miner / oil-pump resource node — the only site type the
    /// MINER-CLAIM path (import bind, wizard siting, opportunity offers) touches.
    /// Fracking satellites (well claim) and geysers (geothermal placement) are
    /// their OWN claim paths — `ClaimWell` / `ClaimGeyser` — and must never take
    /// a miner claim, so those consumers gate on this. Callers that support the
    /// new types check `node_type` directly instead.
    pub fn is_plain_node(&self) -> bool {
        self.node_type == "node"
    }
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

/// Ambient world catalog for a session (W2b-C). Reads `$FICSIT_WORLD_NODES` when
/// set — a CATALOG SWAP, layered UNDER the plan-local node overrides, so the
/// compiled-in asset is never mutated. Any failure (unset, unreadable, or
/// unparseable) falls back to [`bundled`] with a logged warning — the app must
/// never panic on a bad override path (mirrors `FICSIT_DOCS_JSON`).
pub fn load() -> WorldSnapshot {
    match std::env::var("FICSIT_WORLD_NODES") {
        Ok(path) if !path.is_empty() => match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<WorldSnapshot>(&text) {
                Ok(snap) => snap,
                Err(e) => {
                    eprintln!("FICSIT_WORLD_NODES: parse failed ({e}); using the bundled catalog");
                    bundled()
                }
            },
            Err(e) => {
                eprintln!("FICSIT_WORLD_NODES: read failed ({e}); using the bundled catalog");
                bundled()
            }
        },
        _ => bundled(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_snapshot_parses_and_is_plausible() {
        let snap = super::bundled();
        assert!(snap.version >= 1);
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
                ["node", "geyser", "fracking-satellite"].contains(&n.node_type.as_str()),
                "{} node_type {}",
                n.id,
                n.node_type
            );
            // Only fracking satellites carry a well; every satellite must.
            assert_eq!(
                n.well.is_some(),
                n.node_type == "fracking-satellite",
                "{} well/type mismatch",
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
        // The community dataset carries no cave/entrance zoning yet (BACKLOG);
        // cave support is pinned by cave_nodes_parse_with_entrances below.
        let _ = caves;

        // v3 sites — EXACT counts so any generator/vendor drift is caught (a
        // loose `>=` would hide a WELL_EPS_M regression that reshapes the wells
        // or a dropped section). 608 = 459 plain + 31 geysers + 118 satellites.
        let count = |t: &str| snap.nodes.iter().filter(|n| n.node_type == t).count();
        assert_eq!(snap.nodes.len(), 608, "total nodes");
        assert_eq!(count("node"), 459, "plain nodes");
        assert_eq!(count("geyser"), 31, "geysers");
        assert_eq!(count("fracking-satellite"), 118, "fracking satellites");
        // Nitrogen exists ONLY as fracking satellites (nothing else produces it).
        assert!(
            snap.nodes
                .iter()
                .filter(|n| n.item == "Desc_NitrogenGas_C")
                .all(|n| n.node_type == "fracking-satellite"),
            "every nitrogen node is a fracking satellite"
        );
        let wells: std::collections::BTreeSet<_> = snap
            .nodes
            .iter()
            .filter_map(|n| n.well.as_deref())
            .collect();
        assert_eq!(
            wells.len(),
            17,
            "reconstructed wells (6 N + 8 water + 3 oil)"
        );
    }

    #[test]
    fn cave_nodes_parse_with_entrances() {
        let n: super::WorldNode = serde_json::from_str(
            r#"{"id":"c","item":"Desc_Coal_C","purity":"pure","x":1.0,"y":2.0,"z":-35.0,
                "zone":"cave","entrance":{"x":3.0,"y":4.0,"z":20.0},"region":"grass-fields"}"#,
        )
        .unwrap();
        assert_eq!(n.zone, "cave");
        let e = n.entrance.unwrap();
        assert!(n.z < e.z);
    }

    #[test]
    fn ficsit_world_nodes_load_overrides_then_falls_back() {
        // A valid override JSON is loaded verbatim (catalog swap); a missing or
        // invalid path degrades to bundled() with no panic, and the compiled-in
        // asset stays byte-identical throughout.
        let bundled_before = super::BUNDLED.to_string();
        let dir = std::env::temp_dir();
        let good = dir.join(format!("fwn-good-{}.json", std::process::id()));
        std::fs::write(
            &good,
            r#"{"version":99,"source":"override","bounds":{"minX":0.0,"minY":0.0,"maxX":1.0,"maxY":1.0},
                "regions":[{"id":"r","name":"R","labelX":0.0,"labelY":0.0}],
                "nodes":[{"id":"ov1","item":"Desc_OreIron_C","purity":"pure","x":0.5,"y":0.5,"region":"r"}]}"#,
        )
        .unwrap();

        std::env::set_var("FICSIT_WORLD_NODES", &good);
        let loaded = super::load();
        assert_eq!(loaded.version, 99, "valid override is loaded");
        assert_eq!(loaded.source, "override");
        assert_eq!(loaded.nodes.len(), 1);

        // Invalid path → bundled fallback, no panic.
        std::env::set_var("FICSIT_WORLD_NODES", dir.join("does-not-exist.json"));
        let fb = super::load();
        assert_eq!(fb.version, super::bundled().version);
        assert!(fb.nodes.len() >= 20);

        // Malformed JSON → bundled fallback, no panic.
        let bad = dir.join(format!("fwn-bad-{}.json", std::process::id()));
        std::fs::write(&bad, "{ not json").unwrap();
        std::env::set_var("FICSIT_WORLD_NODES", &bad);
        assert_eq!(super::load().version, super::bundled().version);

        // Unset → bundled.
        std::env::remove_var("FICSIT_WORLD_NODES");
        assert_eq!(super::load().source, super::bundled().source);

        std::fs::remove_file(&good).ok();
        std::fs::remove_file(&bad).ok();
        assert_eq!(super::BUNDLED, bundled_before, "bundled asset unchanged");
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
        assert_eq!(n.node_type, "node", "pre-v3 nodes default to plain node");
        assert!(n.well.is_none());
    }

    fn node(node_type: &str, well: Option<&str>) -> super::WorldNode {
        super::WorldNode {
            id: "x".into(),
            item: "Desc_NitrogenGas_C".into(),
            purity: "pure".into(),
            node_type: node_type.into(),
            well: well.map(Into::into),
            x: 0.0,
            y: 0.0,
            z: 0.0,
            zone: "surface".into(),
            entrance: None,
            region: "grass-fields".into(),
        }
    }

    #[test]
    fn is_plain_node_only_true_for_node_type() {
        // The single gate every node consumer keys off — a geyser or fracking
        // satellite must read as NOT plain, or the "inert" guarantee collapses.
        assert!(node("node", None).is_plain_node());
        assert!(!node("geyser", None).is_plain_node());
        assert!(!node("fracking-satellite", Some("well-nitrogen-1")).is_plain_node());
    }

    #[test]
    fn v3_node_round_trips_and_plain_nodes_omit_well() {
        // A satellite carries nodeType + well through serialize → deserialize.
        let sat = node("fracking-satellite", Some("well-nitrogen-1"));
        let back: super::WorldNode =
            serde_json::from_str(&serde_json::to_string(&sat).unwrap()).unwrap();
        assert_eq!(back.node_type, "fracking-satellite");
        assert_eq!(back.well.as_deref(), Some("well-nitrogen-1"));
        // A plain node serializes WITHOUT a `well` key (skip_serializing_if), so
        // it never round-trips a spurious null that downstream code might read.
        let plain = serde_json::to_string(&node("node", None)).unwrap();
        assert!(!plain.contains("well"), "plain node omits well: {plain}");
    }
}
