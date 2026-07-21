//! W2b-C node reconciliation: import binds ◆ NodeClaims to real save nodes by
//! stable id; positions reconcile through a plan-local `node_overrides` overlay
//! (snapshot ⊕ override, the bundled asset never mutated) with re-import drift
//! rows that auto-dissolve when the save agrees with the catalog again.

use std::collections::BTreeMap;

use app::import::{resolved_node_pos, ImportMachine, ImportSnapshot};
use app::session::ImportOutcome;
use app::Session;
use planner_core::commands::Command;
use planner_core::entities::{CreatedBy, EdgeEnd, MapPos, NodeOverride, PortDirection, Status};

fn smelter(x: f64, y: f64) -> ImportMachine {
    ImportMachine {
        class: "Build_SmelterMk1_C".into(),
        recipe: Some("Recipe_IngotIron_C".into()),
        clock: 1.0,
        x,
        y,
        z: 0.0,
        ..Default::default()
    }
}

fn miner(x: f64, y: f64, actor: &str) -> ImportMachine {
    ImportMachine {
        class: "Build_MinerMk2_C".into(),
        recipe: None,
        clock: 1.0,
        x,
        y,
        z: 0.0,
        node_actor_id: Some(actor.into()),
        ..Default::default()
    }
}

/// A miner sitting on a bundled node binds to that snapshot id with the save's
/// stable ref recorded; a miner on no known node mints a `save:<id>` claim.
#[test]
fn claim_binding_snapshot_and_save_local() {
    let mut s = Session::in_memory(None).unwrap();
    let node = s.world.nodes[0].clone();

    let snap = ImportSnapshot {
        save_name: "NODES".into(),
        machines: vec![smelter(node.x, node.y), smelter(100_000.0, 100_000.0)],
        extractors: vec![
            miner(node.x, node.y, "actor-near"),
            miner(100_000.0, 100_000.0, "actor-far"),
        ],
        ..Default::default()
    };
    let outcome = s.import_save(snap).unwrap();
    assert!(matches!(outcome, ImportOutcome::Imported { .. }));

    // import created ◆ claims — the "zero claims" gap is closed.
    assert_eq!(s.state.node_claims.len(), 2, "one claim per miner");
    let near = s
        .state
        .node_claims
        .values()
        .find(|c| c.save_node_id.as_deref() == Some("actor-near"))
        .expect("near miner claim");
    assert_eq!(near.node, node.id, "bound to the bundled snapshot node");
    assert_eq!(near.status, Status::Built);
    assert!(matches!(near.created_by, CreatedBy::Import(_)));
    // within noise of the catalog coordinate → no correction written.
    assert!(!s.state.node_overrides.contains_key(&node.id));

    let far = s
        .state
        .node_claims
        .values()
        .find(|c| c.save_node_id.as_deref() == Some("actor-far"))
        .expect("far miner claim");
    assert_eq!(
        far.node, "save:actor-far",
        "no catalog node → plan-local id"
    );
    // the save-only node's position lives in the overlay alone.
    let ov = s.state.node_overrides.get("save:actor-far").unwrap();
    assert_eq!(ov.pos.unwrap().x, 100_000.0);

    // every claim is wired into its factory's claim list.
    let claimed: usize = s
        .state
        .factories
        .values()
        .map(|f| f.node_claims.len())
        .sum();
    assert_eq!(claimed, 2);
}

/// Regression for the inert-catalog gate: `bind_extractors` matches by PROXIMITY
/// (not item), so a v3 geyser/satellite sitting NEARER an imported miner than its
/// real node must NOT steal the bind. The `is_plain_node()` gate keeps the miner
/// on its plain iron node even though a geyser is closer.
#[test]
fn imported_miner_binds_plain_node_not_a_closer_geyser() {
    let mut s = Session::in_memory(None).unwrap();
    let plain = s.world.nodes[0].clone();
    // A geyser 3 m from the plain node — the miner will sit between them, closer
    // to the geyser, so a proximity-only bind (no gate) would pick the geyser.
    s.world.nodes.push(gamedata::worldnodes::WorldNode {
        id: "geyser_bait".into(),
        item: "Desc_Geyser_C".into(),
        purity: "pure".into(),
        node_type: "geyser".into(),
        well: None,
        x: plain.x + 3.0,
        y: plain.y,
        z: 0.0,
        zone: "surface".into(),
        entrance: None,
        region: plain.region.clone(),
    });

    let snap = ImportSnapshot {
        save_name: "GEYSERBAIT".into(),
        machines: vec![smelter(plain.x, plain.y)],
        // miner 2 m from the plain node, 1 m from the geyser → geyser is nearer.
        extractors: vec![miner(plain.x + 2.0, plain.y, "actor")],
        ..Default::default()
    };
    s.import_save(snap).unwrap();

    let claim = s.state.node_claims.values().next().expect("one claim");
    assert_eq!(
        claim.node, plain.id,
        "miner binds its plain node, not the nearer geyser"
    );
}

/// The save's `mPurityOverride` is authoritative: importing a miner whose save
/// purity disagrees with the bundled catalog records a purity override and bakes
/// it into the session's world, so the map + claim rate read the real purity
/// (this is what makes randomized / modded purities correct).
#[test]
fn save_purity_overrides_the_catalog() {
    let mut s = Session::in_memory(None).unwrap();
    let node = s.world.nodes[0].clone();
    // A save purity that differs from the catalog's for this node.
    let save_purity = if node.purity == "impure" {
        "pure"
    } else {
        "impure"
    };
    let mut m = miner(node.x, node.y, "actor");
    m.purity = Some(save_purity.into());
    let outcome = s
        .import_save(ImportSnapshot {
            save_name: "PURITY".into(),
            machines: vec![smelter(node.x, node.y)],
            extractors: vec![m],
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(outcome, ImportOutcome::Imported { .. }));

    // A purity override is recorded …
    let ov = s
        .state
        .node_overrides
        .get(&node.id)
        .expect("a purity override is written");
    assert_eq!(ov.purity.as_deref(), Some(save_purity));

    // … and baked into this session's world (a solve re-syncs it), so every
    // downstream read sees the save purity, not the catalog's.
    let _ = s.solve_all_readonly();
    let corrected = s.world.nodes.iter().find(|n| n.id == node.id).unwrap();
    assert_eq!(
        corrected.purity, save_purity,
        "save purity wins over the catalog"
    );
    // The bundled asset on disk is untouched.
    assert_eq!(
        gamedata::worldnodes::bundled().nodes[0].purity,
        node.purity,
        "the ambient catalog is never mutated"
    );
}

/// A node override corrects the resolved position; the bundled asset is unchanged.
#[test]
fn node_override_resolution_never_mutates_bundled() {
    let world = gamedata::worldnodes::bundled();
    let node = world.nodes[0].clone();
    let mut overrides: BTreeMap<String, NodeOverride> = BTreeMap::new();

    // no override → resolved is the catalog coordinate.
    let base = resolved_node_pos(&world, &overrides, &node.id).unwrap();
    assert_eq!((base.x, base.y), (node.x, node.y));

    // override → resolved is the corrected coordinate.
    let corrected = MapPos {
        x: node.x + 500.0,
        y: node.y - 250.0,
        z: 12.0,
    };
    overrides.insert(
        node.id.clone(),
        NodeOverride {
            id: node.id.clone(),
            pos: Some(corrected),
            save_actor: Some("actor".into()),
            purity: None,
        },
    );
    let resolved = resolved_node_pos(&world, &overrides, &node.id).unwrap();
    assert_eq!(
        (resolved.x, resolved.y, resolved.z),
        (corrected.x, corrected.y, corrected.z)
    );

    // the ambient catalog is byte-for-byte untouched by resolution.
    assert_eq!(gamedata::worldnodes::bundled(), world);
}

/// First import binds silently; a divergent re-import emits a CorrectNodePosition
/// drift row (never auto-applied); accepting writes the override + lights the
/// derived drift flag; an identical re-import is IN SYNC; and the override
/// auto-dissolves once the save agrees with the snapshot again.
#[test]
fn position_drift_on_reimport_then_auto_dissolve() {
    let mut s = Session::in_memory(None).unwrap();
    let node = s.world.nodes[0].clone();

    // first import: miner exactly on the node → silent bind, no override.
    let base = ImportSnapshot {
        save_name: "DRIFT".into(),
        machines: vec![smelter(node.x, node.y)],
        extractors: vec![miner(node.x, node.y, "actor1")],
        ..Default::default()
    };
    s.import_save(base.clone()).unwrap();
    assert!(
        s.state.node_overrides.is_empty(),
        "silent first-import bind"
    );
    let claim_node = s.state.node_claims.values().next().unwrap().node.clone();

    // re-import with the miner moved 100 m (machine unmoved → factory matches).
    let moved = ImportSnapshot {
        save_name: "DRIFT".into(),
        machines: vec![smelter(node.x, node.y)],
        extractors: vec![miner(node.x + 100.0, node.y, "actor1")],
        ..Default::default()
    };
    let outcome = s.import_save(moved).unwrap();
    let ImportOutcome::Drift { proposal, .. } = outcome else {
        panic!("expected node-position drift");
    };
    let p = &s.state.proposals[&proposal];
    assert!(
        p.items.iter().any(|i| i.label.contains("moved in game")),
        "a CorrectNodePosition drift row: {:?}",
        p.items.iter().map(|i| &i.label).collect::<Vec<_>>()
    );

    // accept → override written, derived drift flag lit, catalog untouched.
    let resp = s.accept_proposal(&proposal).unwrap();
    let ov = s
        .state
        .node_overrides
        .get(&claim_node)
        .expect("override written");
    assert_eq!(ov.pos.unwrap().x, node.x + 100.0);
    assert!(
        resp.derived.nodes[&claim_node].drift,
        "derived node drift set"
    );
    assert!(
        !resp.derived.nodes[&claim_node].conflict,
        "single claim: no conflict"
    );

    // identical re-import (miner still at +100): resolved == save → IN SYNC.
    let same = ImportSnapshot {
        save_name: "DRIFT".into(),
        machines: vec![smelter(node.x, node.y)],
        extractors: vec![miner(node.x + 100.0, node.y, "actor1")],
        ..Default::default()
    };
    assert!(matches!(
        s.import_save(same).unwrap(),
        ImportOutcome::InSync
    ));

    // the save agrees with the snapshot again → drift row → accept dissolves it.
    let back = s.import_save(base).unwrap();
    let ImportOutcome::Drift { proposal, .. } = back else {
        panic!("expected a correction back toward the snapshot");
    };
    s.accept_proposal(&proposal).unwrap();
    assert!(
        !s.state.node_overrides.contains_key(&claim_node),
        "override auto-dissolves once the save agrees with the catalog"
    );
}

/// SetNodeOverride is a single undoable step (plan-local metadata, no guard).
#[test]
fn set_node_override_is_one_undo_entry() {
    let mut s = Session::in_memory(None).unwrap();
    assert!(s.state.node_overrides.is_empty());
    s.edit(vec![Command::SetNodeOverride {
        id: "save:x".into(),
        node_override: Some(NodeOverride {
            id: "save:x".into(),
            pos: Some(MapPos {
                x: 1.0,
                y: 2.0,
                z: 0.0,
            }),
            save_actor: None,
            purity: None,
        }),
    }])
    .unwrap();
    assert_eq!(s.state.node_overrides.len(), 1);
    s.undo().unwrap().unwrap();
    assert!(s.state.node_overrides.is_empty(), "one undo removes it");
    s.redo().unwrap().unwrap();
    assert_eq!(s.state.node_overrides.len(), 1, "one redo restores it");
}

/// T5 (N-batch) — a FIRST import that binds a miner more than the drift
/// threshold off its catalog node silently writes a plan-local override and the
/// derived node lights `drift` (no proposal — first import is authoritative).
#[test]
fn first_import_off_catalog_node_writes_silent_override_and_drift_flag() {
    let mut s = Session::in_memory(None).unwrap();
    let node = s.world.nodes[0].clone();

    // miner 50 m off the catalog node (< NODE_MATCH_M so it still binds, but
    // > NODE_DRIFT_M so a ground-truth correction is written).
    let snap = ImportSnapshot {
        save_name: "OFFNODE".into(),
        machines: vec![smelter(node.x, node.y)],
        extractors: vec![miner(node.x + 50.0, node.y, "actor1")],
        ..Default::default()
    };
    let outcome = s.import_save(snap).unwrap();
    let ImportOutcome::Imported { response, .. } = outcome else {
        panic!("first import writes the built layer");
    };

    // bound to the catalog node, with a silent plan-local override at the save pos.
    let claim = s.state.node_claims.values().next().unwrap();
    assert_eq!(claim.node, node.id, "bound to the catalog node");
    let ov = s
        .state
        .node_overrides
        .get(&node.id)
        .expect("silent override");
    assert_eq!(ov.pos.unwrap().x, node.x + 50.0);
    // the derived node reports drift; the bundled catalog is untouched.
    assert!(
        response.derived.nodes[&node.id].drift,
        "derived drift lit for the off-node bind"
    );
    assert_eq!(gamedata::worldnodes::bundled(), s.world);
}

/// T5 (N4 + N2) — a SAVE-ONLY miner (no catalog node) relocated past the drift
/// threshold on re-import emits a reviewable `CorrectNodePosition` (N4: the
/// `save:` skip is gone); accept writes the new override; an unmoved re-import
/// is IN SYNC (no false drift). Releasing the claim leaves an override-only node
/// that the projection no longer renders (N2).
#[test]
fn save_only_relocation_corrects_and_override_only_node_not_rendered() {
    let mut s = Session::in_memory(None).unwrap();

    // far from every catalog node → a plan-local `save:<actor>` claim + override.
    let base = ImportSnapshot {
        save_name: "SAVEONLY".into(),
        machines: vec![smelter(100_000.0, 100_000.0)],
        extractors: vec![miner(100_000.0, 100_000.0, "far")],
        ..Default::default()
    };
    s.import_save(base).unwrap();
    let claim_node = s.state.node_claims.values().next().unwrap().node.clone();
    assert_eq!(claim_node, "save:far");
    assert_eq!(s.state.node_overrides["save:far"].pos.unwrap().x, 100_000.0);

    // re-import with the SAVE-ONLY miner moved 100 m (machine unmoved → the
    // factory re-matches). Before N4 the `save:` skip suppressed this row.
    let moved = ImportSnapshot {
        save_name: "SAVEONLY".into(),
        machines: vec![smelter(100_000.0, 100_000.0)],
        extractors: vec![miner(100_100.0, 100_000.0, "far")],
        ..Default::default()
    };
    let ImportOutcome::Drift { proposal, .. } = s.import_save(moved).unwrap() else {
        panic!("relocated save-only miner must emit a drift proposal");
    };
    let p = &s.state.proposals[&proposal];
    assert!(
        p.items
            .iter()
            .any(|i| i.label.contains("save:far") && i.label.contains("moved in game")),
        "a CorrectNodePosition row for the save node: {:?}",
        p.items.iter().map(|i| &i.label).collect::<Vec<_>>()
    );

    // accept → apply_sync writes the new override position.
    s.accept_proposal(&proposal).unwrap();
    assert_eq!(
        s.state.node_overrides["save:far"].pos.unwrap().x,
        100_100.0,
        "override moved to the save's new position"
    );

    // an unmoved re-import (miner still at +100) is IN SYNC — no false drift.
    let same = ImportSnapshot {
        save_name: "SAVEONLY".into(),
        machines: vec![smelter(100_000.0, 100_000.0)],
        extractors: vec![miner(100_100.0, 100_000.0, "far")],
        ..Default::default()
    };
    assert!(matches!(
        s.import_save(same).unwrap(),
        ImportOutcome::InSync
    ));

    // N2: release the claim → the override-only node stays inert in state but the
    // projection no longer renders it (only claimed nodes draw).
    let claim_id = s
        .state
        .node_claims
        .values()
        .find(|c| c.node == "save:far")
        .unwrap()
        .id
        .clone();
    // §3.1.1 (audit #122): a ◆ Built claim no longer releases directly — flip
    // it Planned first (SetClaim converts deliberately), then release.
    let claim = s.state.node_claims.get(&claim_id).unwrap().clone();
    s.edit(vec![Command::SetClaim {
        id: claim_id.clone(),
        extractor: claim.extractor.clone(),
        clock: claim.clock,
    }])
    .unwrap();
    let resp = s.edit(vec![Command::ReleaseNode { id: claim_id }]).unwrap();
    assert!(
        !resp.derived.nodes.contains_key("save:far"),
        "an owner-less override-only node is not rendered"
    );
    assert!(
        s.state.node_overrides.contains_key("save:far"),
        "the override stays inert until re-import dissolves it"
    );
}

/// Audit #122 — §3.1.1: releasing a ◆ Built (imported) claim is REJECTED like
/// every other delete on built entities; a Planned claim still releases fine.
#[test]
fn release_node_rejects_built_claim() {
    let mut s = Session::in_memory(None).unwrap();
    let node = s.world.nodes[0].clone();
    let snap = ImportSnapshot {
        machines: vec![smelter(node.x, node.y)],
        extractors: vec![miner(node.x, node.y, "BP_A1")],
        ..Default::default()
    };
    let outcome = s.import_save(snap).unwrap();
    assert!(matches!(outcome, ImportOutcome::Imported { .. }));
    let (cid, claim) = s
        .state
        .node_claims
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .next()
        .expect("import minted a claim");
    assert_eq!(claim.status, Status::Built, "imported claim is ◆ Built");

    let err = s
        .edit(vec![Command::ReleaseNode { id: cid.clone() }])
        .expect_err("releasing a built claim must be rejected");
    assert!(
        format!("{err:?}").contains("BuiltImmutable"),
        "expected BuiltImmutable, got {err:?}"
    );
    assert!(
        s.state.node_claims.contains_key(&cid),
        "the claim survives the rejected release"
    );

    // A Planned claim (drawer-made) still releases.
    let fid = s.state.factories.keys().next().unwrap().clone();
    let other = s.world.nodes[1].clone();
    let resp = s
        .edit(vec![Command::ClaimNode {
            factory: fid,
            node: other.id.clone(),
            extractor: "Build_MinerMk1_C".into(),
            clock: 1.0,
        }])
        .unwrap();
    let planned_id = resp.created[0].clone();
    s.edit(vec![Command::ReleaseNode {
        id: planned_id.clone(),
    }])
    .expect("planned claims release freely");
    assert!(!s.state.node_claims.contains_key(&planned_id));
}

/// Audit #122 — a no-op edit (empty transaction) must not push a phantom undo
/// step or truncate the redo tail.
#[test]
fn noop_edit_preserves_redo_tail() {
    let mut s = Session::in_memory(None).unwrap();
    let resp = s
        .edit(vec![Command::CreateFactory {
            name: "REDO KEEPER".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap();
    let fid = resp.created[0].clone();
    // rename → undo → canRedo
    s.edit(vec![Command::RenameFactory {
        id: fid.clone(),
        name: "X".into(),
    }])
    .unwrap();
    let u = s.undo().unwrap().expect("one step to undo");
    assert!(u.can_redo, "undo leaves a redoable tail");

    // no-op: tidy an empty factory records nothing
    let n = s
        .edit(vec![Command::TidyLayout {
            factory: fid.clone(),
        }])
        .unwrap();
    assert!(n.can_redo, "no-op edit must NOT truncate the redo tail");
    assert!(n.patches.is_empty(), "no-op edit emits no patches");

    let r = s.redo().unwrap().expect("redo tail intact after no-op");
    assert_eq!(
        s.state.factories[&fid].name, "X",
        "the rename redoes after the no-op (got patches: {:?})",
        r.patches
    );
}

/// A Water Extractor has no world node to claim (water is drawn from any
/// surface), so on import it becomes a ◆ Built GROUP running the synthesized
/// zero-ingredient extraction recipe (producing Desc_Water_C) — not an inert
/// save-only claim. Its water is then a real, routable output the empire solves.
#[test]
fn imported_water_extractor_produces_routable_water() {
    let mut s = Session::in_memory(None).unwrap();
    let claims_before = s.state.node_claims.len();
    // A generator seeds a cluster (extractors attach to the nearest machine
    // cluster); the water pump sits in it. The pump's water nets to a surplus —
    // a recipe-less imported generator carries no water demand to consume it.
    let snap = ImportSnapshot {
        save_name: "WATER-IMPORT".into(),
        machines: vec![ImportMachine {
            class: "Build_GeneratorCoal_C".into(),
            recipe: None,
            clock: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            ..Default::default()
        }],
        extractors: vec![ImportMachine {
            class: "Build_WaterPump_C".into(),
            recipe: None,
            clock: 1.0,
            x: 60.0,
            y: 0.0,
            z: 0.0,
            node_actor_id: Some("watervol-1".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    s.import_save(snap).unwrap();

    // The pump imported as a ◆ Built GROUP with the extraction recipe...
    let (pump_id, pump_recipe, pump_status) = {
        let pump = s
            .state
            .groups
            .values()
            .find(|g| g.machine == "Build_WaterPump_C")
            .expect("water pump imports as a producing group, not a claim");
        (pump.id.clone(), pump.recipe.clone(), pump.status)
    };
    assert_eq!(pump_recipe, "Recipe_Extract_Build_WaterPump");
    assert_eq!(pump_status, Status::Built);
    // ...and NOT as a node claim (there is no water node to claim).
    assert_eq!(
        s.state.node_claims.len(),
        claims_before,
        "a water pump is a group, not a claim"
    );
    // Its water is a real solved output (nets to a routable OUT port).
    let d = s.solve_all_readonly();
    let water: f64 = d
        .factories
        .values()
        .filter_map(|f| f.groups.get(&pump_id))
        .filter_map(|g| g.out_rates.get("Desc_Water_C").copied())
        .sum();
    assert!(
        (water - 120.0).abs() < 1e-4,
        "one pump at clock 1 produces its full 120 m³/min, got {water}"
    );
}

fn extractor(class: &str, x: f64, y: f64, actor: &str) -> ImportMachine {
    ImportMachine {
        class: class.into(),
        recipe: None,
        clock: 1.0,
        x,
        y,
        z: 0.0,
        node_actor_id: Some(actor.into()),
        ..Default::default()
    }
}

/// Push a synthetic fracking satellite so a test controls resource + purity
/// exactly, independent of catalog data ordering.
fn push_sat(s: &mut Session, id: &str, item: &str, purity: &str, x: f64, y: f64) {
    s.world.nodes.push(gamedata::worldnodes::WorldNode {
        id: id.into(),
        item: item.into(),
        purity: purity.into(),
        node_type: "fracking-satellite".into(),
        well: Some("well-test".into()),
        x,
        y,
        z: 0.0,
        zone: "surface".into(),
        entrance: None,
        region: "grass-fields".into(),
    });
}

/// Sum the solved out-rate of `item` across every group of `machine`.
fn produced(s: &Session, d: &app::session::Derived, machine: &str, item: &str) -> f64 {
    let ids: std::collections::BTreeSet<String> = s
        .state
        .groups
        .values()
        .filter(|g| g.machine == machine)
        .map(|g| g.id.clone())
        .collect();
    d.factories
        .values()
        .flat_map(|f| f.groups.iter())
        .filter(|(id, _)| ids.contains(*id))
        .filter_map(|(_, g)| g.out_rates.get(item).copied())
        .sum()
}

/// A Resource Well Extractor imports as a producing GROUP (like a water pump, not
/// a claim), and its rate is the satellite's TRUE per-purity extraction — 30 / 60
/// / 120 m³/min for impure / normal / pure. Purity lives in the recipe, so all
/// three arms are exercised (the old test only ever hit `pure`).
#[test]
fn imported_fracking_extractor_produces_its_satellites_fluid_at_purity() {
    for (purity, rate) in [("impure", 30.0), ("normal", 60.0), ("pure", 120.0)] {
        let mut s = Session::in_memory(None).unwrap();
        push_sat(
            &mut s,
            "n-sat",
            "Desc_NitrogenGas_C",
            purity,
            90_000.0,
            90_000.0,
        );
        s.import_save(ImportSnapshot {
            save_name: "N-WELL".into(),
            machines: vec![smelter(90_000.0, 90_000.0)],
            extractors: vec![extractor(
                "Build_FrackingExtractor_C",
                90_000.0,
                90_000.0,
                "f",
            )],
            ..Default::default()
        })
        .unwrap();
        let d = s.solve_all_readonly();
        let nitro = produced(&s, &d, "Build_FrackingExtractor_C", "Desc_NitrogenGas_C");
        assert!(
            (nitro - rate).abs() < 1e-3,
            "{purity} nitrogen = {rate}, got {nitro}"
        );
    }
}

/// Purity is in the recipe, NOT a folded clock, so an OVERCLOCKED pure well is
/// not silently clamped: a pure satellite (120 m³/min) at a 200% save clock
/// extracts 240 m³/min — the old fold (120 × 2.0 × 2.0 → clamp 2.5) would report
/// 300, and folding into the clamped clock would report only ~150.
#[test]
fn overclocked_pure_fracking_well_is_not_clamped() {
    let mut s = Session::in_memory(None).unwrap();
    push_sat(
        &mut s,
        "n-sat",
        "Desc_NitrogenGas_C",
        "pure",
        90_000.0,
        90_000.0,
    );
    let mut ext = extractor("Build_FrackingExtractor_C", 90_000.0, 90_000.0, "f");
    ext.clock = 2.0;
    s.import_save(ImportSnapshot {
        save_name: "OC".into(),
        machines: vec![smelter(90_000.0, 90_000.0)],
        extractors: vec![ext],
        ..Default::default()
    })
    .unwrap();
    let d = s.solve_all_readonly();
    let nitro = produced(&s, &d, "Build_FrackingExtractor_C", "Desc_NitrogenGas_C");
    assert!(
        (nitro - 240.0).abs() < 1e-3,
        "pure × 200% clock = 240, got {nitro}"
    );
}

/// A full well — one Pressurizer + satellites of DIFFERENT purity — imports as
/// ONE factory whose fluid is the SUM of the per-satellite rates (aggregation is
/// linear), that draws the Pressurizer's 150 MW, and is named after the fluid
/// (never the "WATER"/"RESOURCE WELL" fallback).
#[test]
fn imported_full_nitrogen_well_sums_purities_and_draws_150mw() {
    let mut s = Session::in_memory(None).unwrap();
    // one well: impure + pure nitrogen satellites, clustered with their Pressurizer
    push_sat(
        &mut s,
        "n-impure",
        "Desc_NitrogenGas_C",
        "impure",
        90_000.0,
        90_000.0,
    );
    push_sat(
        &mut s,
        "n-pure",
        "Desc_NitrogenGas_C",
        "pure",
        90_030.0,
        90_000.0,
    );
    s.import_save(ImportSnapshot {
        save_name: "FULL-WELL".into(),
        machines: vec![],
        extractors: vec![
            extractor("Build_FrackingSmasher_C", 90_010.0, 90_000.0, "pz"),
            extractor("Build_FrackingExtractor_C", 90_000.0, 90_000.0, "e-impure"),
            extractor("Build_FrackingExtractor_C", 90_030.0, 90_000.0, "e-pure"),
        ],
        ..Default::default()
    })
    .unwrap();

    // exactly one factory (the well), named after nitrogen
    assert_eq!(s.state.factories.len(), 1, "the well is one factory");
    let f = s.state.factories.values().next().unwrap();
    assert!(
        f.name.contains("NITROGEN"),
        "well named after its fluid: {}",
        f.name
    );

    let d = s.solve_all_readonly();
    let nitro = produced(&s, &d, "Build_FrackingExtractor_C", "Desc_NitrogenGas_C");
    assert!(
        (nitro - 150.0).abs() < 1e-3,
        "impure 30 + pure 120 = 150, got {nitro}"
    );
    let power: f64 = d.factories.values().map(|f| f.total_power_mw).sum();
    assert!(
        (power - 150.0).abs() < 1e-3,
        "the Pressurizer draws 150 MW, got {power}"
    );
}

/// An OIL fracking satellite yields Crude Oil (not nitrogen/water), proving the
/// per-resource recipe resolves by the satellite's item — and it produces via a
/// GROUP, never a Build_OilPump_C node claim (no collision with the oil pump).
#[test]
fn imported_oil_fracking_satellite_produces_crude_oil() {
    let mut s = Session::in_memory(None).unwrap();
    push_sat(
        &mut s,
        "oil-sat",
        "Desc_LiquidOil_C",
        "normal",
        90_000.0,
        90_000.0,
    );
    s.import_save(ImportSnapshot {
        save_name: "OIL-WELL".into(),
        machines: vec![smelter(90_000.0, 90_000.0)],
        extractors: vec![extractor(
            "Build_FrackingExtractor_C",
            90_000.0,
            90_000.0,
            "f",
        )],
        ..Default::default()
    })
    .unwrap();
    let d = s.solve_all_readonly();
    let oil = produced(&s, &d, "Build_FrackingExtractor_C", "Desc_LiquidOil_C");
    assert!(
        (oil - 60.0).abs() < 1e-3,
        "normal oil satellite = 60 m³/min, got {oil}"
    );
    assert!(
        !s.state
            .node_claims
            .values()
            .any(|c| c.extractor == "Build_OilPump_C"),
        "a fracking oil satellite is a group, never an oil-pump claim"
    );
}

/// Re-importing a well does not duplicate its groups (parity with the water-pump
/// re-import test): the recipe-less Pressurizer group and the fracking extractor
/// group each stay at one.
#[test]
fn reimporting_fracking_well_does_not_duplicate() {
    let build = |s: &mut Session| {
        s.import_save(ImportSnapshot {
            save_name: "WELL".into(),
            machines: vec![],
            extractors: vec![
                extractor("Build_FrackingSmasher_C", 90_010.0, 90_000.0, "pz"),
                extractor("Build_FrackingExtractor_C", 90_000.0, 90_000.0, "e"),
            ],
            ..Default::default()
        })
        .unwrap();
    };
    let mut s = Session::in_memory(None).unwrap();
    push_sat(
        &mut s,
        "n-sat",
        "Desc_NitrogenGas_C",
        "pure",
        90_000.0,
        90_000.0,
    );
    build(&mut s);
    let count = |s: &Session, m: &str| s.state.groups.values().filter(|g| g.machine == m).count();
    assert_eq!(count(&s, "Build_FrackingExtractor_C"), 1);
    assert_eq!(count(&s, "Build_FrackingSmasher_C"), 1);
    build(&mut s); // re-import the same well
    assert_eq!(
        count(&s, "Build_FrackingExtractor_C"),
        1,
        "extractor not duplicated"
    );
    assert_eq!(
        count(&s, "Build_FrackingSmasher_C"),
        1,
        "pressurizer not duplicated"
    );
}

/// PLANNED placement: `ClaimWell` stamps a new factory for the whole well — one
/// Extractor group per satellite purity (aggregated), one Pressurizer, and a
/// routable fluid OUT port. The factory produces Σ per-satellite fluid and draws
/// the Pressurizer's 150 MW.
#[test]
fn claim_well_stamps_a_planned_well_factory() {
    let mut s = Session::in_memory(None).unwrap();
    // one nitrogen well: 2 pure + 1 impure satellites
    push_sat(
        &mut s,
        "w-p1",
        "Desc_NitrogenGas_C",
        "pure",
        90_000.0,
        90_000.0,
    );
    push_sat(
        &mut s,
        "w-p2",
        "Desc_NitrogenGas_C",
        "pure",
        90_020.0,
        90_000.0,
    );
    push_sat(
        &mut s,
        "w-i1",
        "Desc_NitrogenGas_C",
        "impure",
        90_040.0,
        90_000.0,
    );
    for n in s.world.nodes.iter_mut().filter(|n| n.id.starts_with("w-")) {
        n.well = Some("well-n".into());
    }

    let before = s.state.factories.len();
    s.edit(vec![Command::ClaimWell {
        well: "well-n".into(),
    }])
    .unwrap();

    // a new factory, named after the fluid
    assert_eq!(s.state.factories.len(), before + 1, "one new well factory");
    let f = s
        .state
        .factories
        .values()
        .find(|f| f.name.contains("NITROGEN"))
        .expect("well factory named after nitrogen");
    // groups: one extractor group per purity (pure + impure) + one pressurizer
    let ext = s
        .state
        .groups
        .values()
        .filter(|g| g.factory == f.id && g.machine == "Build_FrackingExtractor_C")
        .count();
    assert_eq!(ext, 2, "one extractor group per purity (pure, impure)");
    assert_eq!(
        s.state
            .groups
            .values()
            .filter(|g| g.factory == f.id && g.machine == "Build_FrackingSmasher_C")
            .count(),
        1,
        "one Pressurizer"
    );
    // the OUT port is sized to total production (2×120 + 30 = 270), not just present
    let port = s
        .state
        .ports
        .values()
        .find(|p| {
            p.factory == f.id && p.item == "Desc_NitrogenGas_C" && p.direction == PortDirection::Out
        })
        .expect("routable nitrogen OUT port");
    assert!(
        (port.rate - 270.0).abs() < 1e-3,
        "OUT port sized to Σ, got {}",
        port.rate
    );
    // each extractor group is wired to the OUT port (Group→Port edge) — without
    // these the source idles at 0 (the whole point of the wiring).
    let ext_ids: std::collections::BTreeSet<&str> = s
        .state
        .groups
        .values()
        .filter(|g| g.factory == f.id && g.machine == "Build_FrackingExtractor_C")
        .map(|g| g.id.as_str())
        .collect();
    for gid in &ext_ids {
        assert!(
            s.state.edges.values().any(|e| e.factory == f.id
                && matches!(&e.from, EdgeEnd::Group(g) if g == gid)
                && matches!(&e.to, EdgeEnd::Port(p) if p == &port.id)),
            "extractor group {gid} is wired to the OUT port"
        );
    }
    // the factory pins on the well centroid (x ∈ {90000,90020,90040} → 90020)
    assert!(
        (f.position.x - 90_020.0).abs() < 1e-3,
        "pin on centroid, got {}",
        f.position.x
    );

    // solve: 2 pure (120 each) + 1 impure (30) = 270 m³/min, 150 MW draw
    let d = s.solve_all_readonly();
    let nitro = produced(&s, &d, "Build_FrackingExtractor_C", "Desc_NitrogenGas_C");
    assert!(
        (nitro - 270.0).abs() < 1e-3,
        "Σ well output = 270, got {nitro}"
    );
    let power: f64 = d.factories.values().map(|f| f.total_power_mw).sum();
    assert!(
        (power - 150.0).abs() < 1e-3,
        "Pressurizer draws 150 MW, got {power}"
    );
}

/// Claiming the same well twice is refused (no duplicate factory) — the centroid
/// guard catches it. And undo of a well claim removes EVERYTHING it stamped
/// (factory + all groups + port + edges); redo restores it.
#[test]
fn claim_well_is_idempotent_and_undoes_atomically() {
    let mut s = Session::in_memory(None).unwrap();
    push_sat(
        &mut s,
        "u-p",
        "Desc_NitrogenGas_C",
        "pure",
        91_000.0,
        91_000.0,
    );
    push_sat(
        &mut s,
        "u-i",
        "Desc_NitrogenGas_C",
        "impure",
        91_020.0,
        91_000.0,
    );
    for n in s.world.nodes.iter_mut().filter(|n| n.id.starts_with("u-")) {
        n.well = Some("well-u".into());
    }
    let base = (
        s.state.factories.len(),
        s.state.groups.len(),
        s.state.ports.len(),
        s.state.edges.len(),
    );
    let claim = || Command::ClaimWell {
        well: "well-u".into(),
    };

    s.edit(vec![claim()]).unwrap();
    let after = (
        s.state.factories.len(),
        s.state.groups.len(),
        s.state.ports.len(),
        s.state.edges.len(),
    );
    assert_eq!(after.0, base.0 + 1, "one factory");

    // re-claim the same well → refused, nothing added
    assert!(
        s.edit(vec![claim()]).is_err(),
        "second claim of the same well is refused"
    );
    assert_eq!(
        s.state.factories.len(),
        after.0,
        "no duplicate well factory"
    );

    // undo removes the whole well atomically; redo restores it
    s.undo().unwrap().unwrap();
    assert_eq!(
        (
            s.state.factories.len(),
            s.state.groups.len(),
            s.state.ports.len(),
            s.state.edges.len()
        ),
        base,
        "undo removes factory + groups + port + edges"
    );
    s.redo().unwrap().unwrap();
    assert_eq!(
        (
            s.state.factories.len(),
            s.state.groups.len(),
            s.state.ports.len(),
            s.state.edges.len()
        ),
        after,
        "redo restores everything"
    );
}

/// A well with all three purities → three extractor groups summing to
/// 120 + 60 + 30 = 210 m³/min. Also proves an OIL well produces Crude Oil (not
/// only nitrogen is wired), and that claim_well errors on an unknown well id.
#[test]
fn claim_well_all_purities_and_oil() {
    let mut s = Session::in_memory(None).unwrap();
    push_sat(&mut s, "a-p", "Desc_NitrogenGas_C", "pure", 92_000.0, 0.0);
    push_sat(&mut s, "a-n", "Desc_NitrogenGas_C", "normal", 92_020.0, 0.0);
    push_sat(&mut s, "a-i", "Desc_NitrogenGas_C", "impure", 92_040.0, 0.0);
    push_sat(&mut s, "o-1", "Desc_LiquidOil_C", "normal", 93_000.0, 0.0);
    for n in s.world.nodes.iter_mut() {
        if n.id.starts_with("a-") {
            n.well = Some("well-a".into());
        } else if n.id.starts_with("o-") {
            n.well = Some("well-o".into());
        }
    }
    assert!(
        s.edit(vec![Command::ClaimWell {
            well: "nope".into()
        }])
        .is_err(),
        "unknown well errors"
    );

    s.edit(vec![Command::ClaimWell {
        well: "well-a".into(),
    }])
    .unwrap();
    let na = s
        .state
        .factories
        .values()
        .find(|f| f.name.contains("NITROGEN"))
        .unwrap();
    assert_eq!(
        s.state
            .groups
            .values()
            .filter(|g| g.factory == na.id && g.machine == "Build_FrackingExtractor_C")
            .count(),
        3,
        "one group per purity"
    );
    let d = s.solve_all_readonly();
    let nitro = produced(&s, &d, "Build_FrackingExtractor_C", "Desc_NitrogenGas_C");
    assert!((nitro - 210.0).abs() < 1e-3, "120+60+30 = 210, got {nitro}");

    s.edit(vec![Command::ClaimWell {
        well: "well-o".into(),
    }])
    .unwrap();
    let d = s.solve_all_readonly();
    let oil = produced(&s, &d, "Build_FrackingExtractor_C", "Desc_LiquidOil_C");
    assert!((oil - 60.0).abs() < 1e-3, "normal oil well = 60, got {oil}");
}

/// The dispatch wiring: `edit()` intercepts ClaimWell (never reaching planner-core),
/// while a RAW `commands::apply` of ClaimWell hits the defensive error. And a raw
/// claim_node on a fracking satellite is refused at the session layer.
#[test]
fn claim_well_dispatch_wiring_and_satellite_claim_guard() {
    use planner_core::commands::{apply, DomainError};
    use planner_core::state::PlanState;

    // raw planner-core apply of ClaimWell → defensive session-layer-only error
    let mut st = PlanState::default();
    let err = apply(&mut st, &Command::ClaimWell { well: "x".into() }).unwrap_err();
    assert!(matches!(err, DomainError::Invalid { .. }));

    // a raw claim_node on a fracking satellite is rejected by edit()
    let mut s = Session::in_memory(None).unwrap();
    let sat = s
        .world
        .nodes
        .iter()
        .find(|n| n.node_type == "fracking-satellite")
        .unwrap()
        .id
        .clone();
    let f = s
        .edit(vec![Command::CreateFactory {
            name: "F".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: String::new(),
        }])
        .unwrap()
        .created[0]
        .clone();
    assert!(
        s.edit(vec![Command::ClaimNode {
            factory: f,
            node: sat,
            extractor: "Build_MinerMk2_C".into(),
            clock: 1.0,
        }])
        .is_err(),
        "claiming a fracking satellite as a miner node is refused"
    );
}

/// An imported Resource Well Pressurizer draws its 150 MW: it produces nothing
/// (recipe-less, skipped by the material solve) so its nameplate is injected as a
/// power DRAW on its group, and the well factory's power reads 150 MW, not 0.
#[test]
fn imported_pressurizer_draws_its_nameplate_power() {
    let mut s = Session::in_memory(None).unwrap();
    let sat = s
        .world
        .nodes
        .iter()
        .find(|n| n.node_type == "fracking-satellite")
        .expect("bundled catalog has a fracking satellite")
        .clone();
    let snap = ImportSnapshot {
        save_name: "PRESSURIZED".into(),
        machines: vec![smelter(sat.x, sat.y)],
        extractors: vec![extractor("Build_FrackingSmasher_C", sat.x, sat.y, "pz-1")],
        ..Default::default()
    };
    s.import_save(snap).unwrap();

    let pz_id = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_FrackingSmasher_C")
        .expect("Pressurizer imports as a group, not a claim")
        .id
        .clone();
    let d = s.solve_all_readonly();
    let draw: f64 = d
        .factories
        .values()
        .filter_map(|f| f.groups.get(&pz_id))
        .map(|g| g.power_mw)
        .sum();
    assert!(
        (draw - 150.0).abs() < 1e-3,
        "the Pressurizer draws its 150 MW nameplate, got {draw}"
    );
}

fn water_pump(x: f64, clock: f64, actor: &str) -> ImportMachine {
    ImportMachine {
        class: "Build_WaterPump_C".into(),
        recipe: None,
        clock,
        x,
        y: 0.0,
        z: 0.0,
        node_actor_id: Some(actor.into()),
        ..Default::default()
    }
}

fn coal_gen(recipe: Option<String>) -> ImportMachine {
    ImportMachine {
        class: "Build_GeneratorCoal_C".into(),
        recipe,
        clock: 1.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        ..Default::default()
    }
}

/// Several water pumps in one cluster collapse to ONE ◆ group with count = N and
/// the MEAN clock — not N groups, and not a summed clock.
#[test]
fn imported_water_pumps_aggregate_into_one_group() {
    let mut s = Session::in_memory(None).unwrap();
    s.import_save(ImportSnapshot {
        save_name: "PUMPS".into(),
        machines: vec![coal_gen(None)], // seeds the cluster the pumps attach to
        extractors: vec![
            water_pump(40.0, 1.0, "wv1"),
            water_pump(60.0, 0.5, "wv2"),
            water_pump(80.0, 0.75, "wv3"),
        ],
        ..Default::default()
    })
    .unwrap();
    let pumps: Vec<_> = s
        .state
        .groups
        .values()
        .filter(|g| g.machine == "Build_WaterPump_C")
        .collect();
    assert_eq!(pumps.len(), 1, "three pumps collapse to one group");
    assert_eq!(pumps[0].count, 3, "count aggregates");
    // mean of 1.0, 0.5, 0.75 = 0.75 (not the 2.25 sum).
    assert!(
        (pumps[0].clock - 0.75).abs() < 1e-3,
        "clock is the mean, got {}",
        pumps[0].clock
    );
}

/// A water pump with no machine cluster in range is NOT dropped — it forms a
/// standalone ◆ water factory that still produces its water.
#[test]
fn imported_lone_water_pump_becomes_standalone_factory() {
    let mut s = Session::in_memory(None).unwrap();
    s.import_save(ImportSnapshot {
        save_name: "LONE-WATER".into(),
        machines: vec![],
        extractors: vec![water_pump(0.0, 1.0, "wv")],
        ..Default::default()
    })
    .unwrap();
    let pid = {
        let pump = s
            .state
            .groups
            .values()
            .find(|g| g.machine == "Build_WaterPump_C")
            .expect("a lone water pump forms a standalone factory, not dropped");
        assert_eq!(pump.status, Status::Built);
        pump.id.clone()
    };
    let d = s.solve_all_readonly();
    let water: f64 = d
        .factories
        .values()
        .filter_map(|f| f.groups.get(&pid))
        .filter_map(|g| g.out_rates.get("Desc_Water_C").copied())
        .sum();
    assert!(
        (water - 120.0).abs() < 1e-4,
        "the standalone pump produces its water, got {water}"
    );
}

/// The pump's water auto-wires to a water CONSUMER in the same cluster (an
/// internal edge), rather than only netting to an OUT port. (A generator carrying
/// its water-consuming burn recipe stands in for the consumer — the general
/// write_built_layer producer→consumer wiring, exercised for water.)
#[test]
fn imported_water_pump_auto_wires_to_a_same_cluster_consumer() {
    let mut s = Session::in_memory(None).unwrap();
    let burn = s
        .gamedata
        .recipes
        .values()
        .find(|r| r.produced_in.contains(&"Build_GeneratorCoal_C".to_string()))
        .unwrap()
        .class_name
        .clone();
    s.import_save(ImportSnapshot {
        save_name: "WIRED".into(),
        machines: vec![coal_gen(Some(burn))],
        extractors: vec![water_pump(40.0, 1.0, "wv")],
        ..Default::default()
    })
    .unwrap();
    let pump = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_WaterPump_C")
        .unwrap()
        .id
        .clone();
    let gen = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_GeneratorCoal_C")
        .unwrap()
        .id
        .clone();
    let wired = s.state.edges.values().any(|e| {
        e.item == "Desc_Water_C"
            && matches!(&e.from, EdgeEnd::Group(g) if *g == pump)
            && matches!(&e.to, EdgeEnd::Group(g) if *g == gen)
    });
    assert!(
        wired,
        "the pump's water auto-wires to the same-cluster water consumer"
    );
}

/// Re-importing the same save re-matches the water group (keyed by machine +
/// recipe) instead of duplicating it.
#[test]
fn reimporting_water_pump_does_not_duplicate() {
    let mut s = Session::in_memory(None).unwrap();
    let snap = || ImportSnapshot {
        save_name: "RE".into(),
        machines: vec![coal_gen(None)],
        extractors: vec![water_pump(40.0, 1.0, "wv")],
        ..Default::default()
    };
    s.import_save(snap()).unwrap();
    let count = |s: &Session| {
        s.state
            .groups
            .values()
            .filter(|g| g.machine == "Build_WaterPump_C")
            .count()
    };
    assert_eq!(count(&s), 1);
    s.import_save(snap()).unwrap();
    assert_eq!(
        count(&s),
        1,
        "re-import re-matches the water group, no duplicate"
    );
}
