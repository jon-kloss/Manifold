//! W2b-C node reconciliation: import binds ◆ NodeClaims to real save nodes by
//! stable id; positions reconcile through a plan-local `node_overrides` overlay
//! (snapshot ⊕ override, the bundled asset never mutated) with re-import drift
//! rows that auto-dissolve when the save agrees with the catalog again.

use std::collections::BTreeMap;

use app::import::{resolved_node_pos, ImportMachine, ImportSnapshot};
use app::session::ImportOutcome;
use app::Session;
use planner_core::commands::Command;
use planner_core::entities::{CreatedBy, MapPos, NodeOverride, Status};

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
