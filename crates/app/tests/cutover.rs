//! W2a refactor/cutover: the cutover + downtime are DERIVED — never a mutation
//! of the ◆ built layer. A `replaces` link (a serde-default planner label) pairs
//! a ◇ replacement to the running ◆ factory; the cutover is BuildNew → Switch →
//! Dismantle; the downtime is scratch-solved per phase boundary with the full
//! downstream ripple; dismantle completion is derived from the ◆ layer.

use app::buildqueue::BuildStepState;
use app::cutover::{derive_cutovers, switch_step_id, CutoverPhase};
use app::import::{ImportMachine, ImportSnapshot};
use app::Session;
use planner_core::commands::Command;
use planner_core::entities::*;

const SCREW: &str = "Desc_IronScrew_C";
const IRON_ROD: &str = "Desc_IronRod_C";

fn mach(class: &str, recipe: &str, x: f64, y: f64) -> ImportMachine {
    ImportMachine {
        class: class.into(),
        recipe: Some(recipe.into()),
        clock: 1.0,
        x,
        y,
        z: 0.0,
    }
}

/// Import a single ◆ built factory (one machine) and return its factory id.
fn import_built(s: &mut Session, name: &str, recipe: &str, x: f64, y: f64) -> Id {
    s.import_save(ImportSnapshot {
        save_name: name.into(),
        machines: vec![mach("Build_ConstructorMk1_C", recipe, x, y)],
        ..Default::default()
    })
    .unwrap();
    s.state
        .factories
        .values()
        .filter(|f| f.status == Status::Built)
        .max_by(|a, b| a.position.x.partial_cmp(&b.position.x).unwrap())
        .map(|f| f.id.clone())
        .unwrap()
}

/// First import of a SEED factory (iron ingot, far away) + an OLD factory
/// (screw, at origin) in ONE call, so BOTH are ◆ built. Returns (seed, old).
fn import_seed_and_old(s: &mut Session) -> (Id, Id) {
    s.import_save(ImportSnapshot {
        save_name: "BASE".into(),
        machines: vec![
            mach(
                "Build_ConstructorMk1_C",
                "Recipe_IngotIron_C",
                5000.0,
                5000.0,
            ),
            mach("Build_ConstructorMk1_C", "Recipe_Screw_C", 0.0, 0.0),
        ],
        ..Default::default()
    })
    .unwrap();
    let by_x = |target: f64| {
        s.state
            .factories
            .values()
            .filter(|f| f.status == Status::Built)
            .min_by(|a, b| {
                (a.position.x - target)
                    .abs()
                    .partial_cmp(&(b.position.x - target).abs())
                    .unwrap()
            })
            .map(|f| f.id.clone())
            .unwrap()
    };
    (by_x(5000.0), by_x(0.0))
}

/// Re-import with ONLY the seed machine present → the OLD factory vanished in
/// game (a RemoveFactory drift). Accepts the drift proposal.
fn reimport_without_old(s: &mut Session) {
    let outcome = s
        .import_save(ImportSnapshot {
            save_name: "TORNDOWN".into(),
            machines: vec![mach(
                "Build_ConstructorMk1_C",
                "Recipe_IngotIron_C",
                5000.0,
                5000.0,
            )],
            ..Default::default()
        })
        .unwrap();
    let pid = match outcome {
        app::session::ImportOutcome::Drift { proposal, .. } => proposal,
        other => panic!("expected drift, got {other:?}"),
    };
    s.accept_proposal(&pid).unwrap();
}

fn planned_factory(s: &mut Session, name: &str, x: f64, y: f64) -> Id {
    s.edit(vec![Command::CreateFactory {
        name: name.into(),
        position: MapPos { x, y, z: 0.0 },
        region: "GRASS FIELDS".into(),
    }])
    .unwrap()
    .created[0]
        .clone()
}

fn add_port(s: &mut Session, fid: &Id, dir: PortDirection, item: &str, rate: f64) -> Id {
    s.edit(vec![Command::AddPort {
        factory: fid.clone(),
        direction: dir,
        item: item.into(),
        rate,
        rate_ceiling: None,
        graph_pos: GraphPos { x: 0.0, y: 0.0 },
    }])
    .unwrap()
    .created[0]
        .clone()
}

/// A self-contained ◇ screw factory: unconstrained iron-rod IN → 1 constructor
/// → screw OUT targeting 40/min. Solves to 40 screws given unlimited rod.
fn planned_screw_factory(s: &mut Session, name: &str, x: f64, y: f64) -> Id {
    let f = planned_factory(s, name, x, y);
    let inp = add_port(s, &f, PortDirection::In, IRON_ROD, 0.0);
    let g = s
        .edit(vec![Command::AddGroup {
            factory: f.clone(),
            machine: "Build_ConstructorMk1_C".into(),
            recipe: "Recipe_Screw_C".into(),
            count: 1,
            clock: 1.0,
            graph_pos: GraphPos { x: 0.0, y: 0.0 },
            floor: 0,
        }])
        .unwrap()
        .created[0]
        .clone();
    let out = add_port(s, &f, PortDirection::Out, SCREW, 40.0);
    s.edit(vec![Command::AddEdge {
        factory: f.clone(),
        from: EdgeEnd::Port(inp),
        to: EdgeEnd::Group(g.clone()),
        item: IRON_ROD.into(),
        tier: 3,
    }])
    .unwrap();
    s.edit(vec![Command::AddEdge {
        factory: f.clone(),
        from: EdgeEnd::Group(g),
        to: EdgeEnd::Port(out.clone()),
        item: SCREW.into(),
        tier: 3,
    }])
    .unwrap();
    s.edit(vec![Command::SetPortRate {
        id: out,
        rate: 40.0,
    }])
    .unwrap();
    f
}

/// Serde-default: a plan file predating W2a (no `replaces` key) deserializes to
/// `None` — no migration.
#[test]
fn factory_without_replaces_deserializes_none() {
    let json = serde_json::json!({
        "id": "01OLD",
        "name": "SITE",
        "position": { "x": 0.0, "y": 0.0, "z": 0.0 },
        "region": "",
        "nodeClaims": [],
        "groups": [],
        "ports": [],
        "styleGuide": null,
        "status": "built",
        "createdBy": { "kind": "manual" }
    });
    let f: Factory = serde_json::from_value(json).unwrap();
    assert_eq!(f.replaces, None);
}

/// SetFactoryReplaces rejects self, a missing target, and a ◇ planned target;
/// a valid link is one undoable step.
#[test]
fn set_factory_replaces_rejects_self_missing_and_planned_target() {
    let mut s = Session::in_memory(None).unwrap();
    let old = import_built(&mut s, "OLD", "Recipe_Screw_C", 0.0, 0.0);
    let new = planned_factory(&mut s, "NEW", 400.0, 0.0);

    // self
    assert!(s
        .edit(vec![Command::SetFactoryReplaces {
            id: new.clone(),
            replaces: Some(new.clone()),
        }])
        .is_err());
    // missing target
    assert!(s
        .edit(vec![Command::SetFactoryReplaces {
            id: new.clone(),
            replaces: Some("01MISSING".into()),
        }])
        .is_err());
    // a ◇ planned target (not Built) — a cutover replaces a RUNNING factory
    let other_planned = planned_factory(&mut s, "OTHER", 900.0, 0.0);
    assert!(s
        .edit(vec![Command::SetFactoryReplaces {
            id: new.clone(),
            replaces: Some(other_planned),
        }])
        .is_err());

    // valid link, and one undo restores replaces = None
    s.edit(vec![Command::SetFactoryReplaces {
        id: new.clone(),
        replaces: Some(old.clone()),
    }])
    .unwrap();
    assert_eq!(s.state.factories[&new].replaces, Some(old.clone()));
    s.undo().unwrap().unwrap();
    assert_eq!(s.state.factories[&new].replaces, None);
}

/// A `replaces` link derives a cutover pairing new → old, with steps ordered
/// BuildNew → Switch → Dismantle.
#[test]
fn cutover_pairs_new_to_old_and_orders_phases() {
    let mut s = Session::in_memory(None).unwrap();
    let old = import_built(&mut s, "OLD", "Recipe_Screw_C", 0.0, 0.0);
    // give the old factory an OUT port so the Switch phase has an item
    add_port(&mut s, &old, PortDirection::Out, SCREW, 40.0);
    let new = planned_screw_factory(&mut s, "NEW SCREWS", 400.0, 0.0);
    s.edit(vec![Command::SetFactoryReplaces {
        id: new.clone(),
        replaces: Some(old.clone()),
    }])
    .unwrap();

    let cutovers = derive_cutovers(&s.state, &s.gamedata);
    let c = cutovers
        .iter()
        .find(|c| c.new_factory == new)
        .expect("cutover for new factory");
    assert_eq!(c.old_factory, old);
    // phases appear in order, BuildNew first, Dismantle last
    let phases: Vec<CutoverPhase> = c.steps.iter().map(|st| st.phase).collect();
    assert_eq!(phases.first(), Some(&CutoverPhase::BuildNew));
    assert_eq!(phases.last(), Some(&CutoverPhase::Dismantle));
    assert!(phases.windows(2).all(|w| w[0] <= w[1]), "phases sorted");
    // exactly one Switch step for the single supplied item (screws)
    assert_eq!(
        c.steps
            .iter()
            .filter(|st| st.phase == CutoverPhase::Switch)
            .count(),
        1
    );
    // BuildNew tracks the new factory; Dismantle tracks the old ◆
    let build = c
        .steps
        .iter()
        .find(|st| st.phase == CutoverPhase::BuildNew)
        .unwrap();
    assert_eq!(build.id, new);
    let dismantle = c
        .steps
        .iter()
        .find(|st| st.phase == CutoverPhase::Dismantle)
        .unwrap();
    assert_eq!(dismantle.id, old);
}

/// Dismantle stays Pending while the old ◆ exists; when a re-import removes it
/// (SyncOp::RemoveFactory), re-deriving reads dismantle Done.
#[test]
fn dismantle_pending_while_old_exists_then_done_when_removed() {
    let mut s = Session::in_memory(None).unwrap();
    let (_seed, old) = import_seed_and_old(&mut s);
    let new = planned_screw_factory(&mut s, "NEW SCREWS", 400.0, 0.0);
    s.edit(vec![Command::SetFactoryReplaces {
        id: new.clone(),
        replaces: Some(old.clone()),
    }])
    .unwrap();

    let c = derive_cutovers(&s.state, &s.gamedata)
        .into_iter()
        .find(|c| c.new_factory == new)
        .unwrap();
    let dismantle = c
        .steps
        .iter()
        .find(|st| st.phase == CutoverPhase::Dismantle)
        .unwrap();
    assert!(!dismantle.done, "old still exists → dismantle pending");
    assert_eq!(dismantle.state, BuildStepState::Pending);

    // re-import a save where OLD is gone (only SEED survives) → RemoveFactory
    // drift; accept it.
    reimport_without_old(&mut s);

    assert!(
        !s.state.factories.contains_key(&old),
        "re-import synced the teardown"
    );
    // the new factory's `replaces` auto-nulled (dangling intent dissolved)
    assert_eq!(s.state.factories[&new].replaces, None);
    // and the cutover reads dismantle-complete on its own — but with replaces
    // nulled there is no cutover at all now: that IS the terminal state.
    assert!(derive_cutovers(&s.state, &s.gamedata)
        .iter()
        .all(|c| c.new_factory != new));
}

/// The downtime engine: old ◆ 40 screws/min + new ◇ 40/min. Baseline (k=0) ≈ 40,
/// the Switch boundary (k=1) drops to ≈ 0 with a dip carrying a machine-count
/// est, and the Dismantle boundary (k=2) recovers to ≈ 40. Proves the
/// intermediate-state scratch-solve + the ripple-inclusive rate.
#[test]
fn downtime_drop_across_boundaries() {
    let mut s = Session::in_memory(None).unwrap();
    // old ◆ screw factory producing 40/min (constructor fed by an unconstrained
    // rod IN, wired to a screw OUT). Import can't wire like this, so build the
    // old one from ◇ commands then treat it as the retirement target via the
    // downtime engine (which only needs the derived Out rate).
    let old = planned_screw_factory(&mut s, "OLD SCREWS", 0.0, 0.0);
    let new = planned_screw_factory(&mut s, "NEW SCREWS", 400.0, 0.0);
    // link new → old is normally guarded to a ◆ target; the downtime engine
    // works off a Cutover projection, so build one directly for the test.
    let cutover = app::cutover::Cutover {
        new_factory: new.clone(),
        new_name: "NEW SCREWS".into(),
        old_factory: old.clone(),
        old_name: "OLD SCREWS".into(),
        steps: vec![],
        node_reuse: false,
        number: 0,
    };
    // both factories individually produce ~40 screws/min
    let derived = s.solve_all_readonly();
    let base_old = derived
        .factories
        .get(&old)
        .and_then(|df| {
            s.state.factories[&old]
                .ports
                .iter()
                .find(|pid| s.state.ports[*pid].direction == PortDirection::Out)
                .and_then(|pid| df.ports.get(pid))
        })
        .copied()
        .unwrap_or(0.0);
    assert!(
        (base_old - 40.0).abs() < 1.0,
        "old factory produces ~40 screws, got {base_old}"
    );

    // drive the boundary shaping + solves directly (mirrors cutover_plan)
    let saved = s.state.clone();
    let rate_at = |s: &mut Session, k: usize| -> f64 {
        s.state = app::cutover::shape_for_boundary(&saved, &cutover, k);
        let d = s.solve_all_readonly();
        s.state
            .ports
            .values()
            .filter(|p| p.direction == PortDirection::Out && p.item == SCREW)
            .filter_map(|p| {
                d.factories
                    .get(&p.factory)
                    .and_then(|df| df.ports.get(&p.id))
            })
            .sum()
    };
    let k0 = rate_at(&mut s, 0);
    let k1 = rate_at(&mut s, 1);
    let k2 = rate_at(&mut s, 2);
    s.state = saved;

    assert!((k0 - 40.0).abs() < 1.0, "baseline ≈ 40, got {k0}");
    assert!(k1 < 1.0, "switch boundary drops to ≈ 0, got {k1}");
    assert!(
        (k2 - 40.0).abs() < 1.0,
        "dismantle boundary recovers, got {k2}"
    );
    assert!(k0 - k1 > 1.0, "there is a real dip during the switch");

    // est_hours is derived from the torn-down machine count × the const
    assert!(app::cutover::est_hours(1) > 0.0);
    assert_eq!(
        app::cutover::est_hours(3),
        3.0 * app::cutover::SWITCH_MIN_PER_MACHINE / 60.0
    );
}

/// Node reuse: the new ◇ claims a node the old ◆ still holds → hard flag set (and
/// the shared node lights the existing conflict marker).
#[test]
fn node_reuse_sets_hard_conflict() {
    let mut s = Session::in_memory(None).unwrap();
    let old = import_built(&mut s, "OLD", "Recipe_IngotIron_C", 0.0, 0.0);
    add_port(&mut s, &old, PortDirection::Out, "Desc_IronIngot_C", 30.0);
    // old claims a node
    s.edit(vec![Command::ClaimNode {
        factory: old.clone(),
        node: "bp_resourcenode496".into(),
        extractor: "Build_MinerMk2_C".into(),
        clock: 1.0,
    }])
    .unwrap();
    let new = planned_factory(&mut s, "NEW", 400.0, 0.0);
    // new claims the SAME node
    s.edit(vec![Command::ClaimNode {
        factory: new.clone(),
        node: "bp_resourcenode496".into(),
        extractor: "Build_MinerMk2_C".into(),
        clock: 1.0,
    }])
    .unwrap();
    s.edit(vec![Command::SetFactoryReplaces {
        id: new.clone(),
        replaces: Some(old.clone()),
    }])
    .unwrap();

    let c = derive_cutovers(&s.state, &s.gamedata)
        .into_iter()
        .find(|c| c.new_factory == new)
        .unwrap();
    assert!(c.node_reuse, "shared node → unavoidable downtime");
    // the shared node also lights the derived conflict marker
    let derived = s.solve_all_readonly();
    assert!(derived.nodes["bp_resourcenode496"].conflict);
}

/// A manual BuildOverride pins the dismantle step done; on re-import removing the
/// old factory the link nulls and the override is cascade-pruned with the ◆.
#[test]
fn build_override_pins_dismantle_and_dissolves_on_reimport() {
    let mut s = Session::in_memory(None).unwrap();
    let (_seed, old) = import_seed_and_old(&mut s);
    let new = planned_screw_factory(&mut s, "NEW SCREWS", 400.0, 0.0);
    s.edit(vec![Command::SetFactoryReplaces {
        id: new.clone(),
        replaces: Some(old.clone()),
    }])
    .unwrap();

    // pin the dismantle step (keyed on the old factory id) done by hand
    s.edit(vec![Command::SetBuildDone {
        id: old.clone(),
        done: Some(true),
    }])
    .unwrap();
    let c = derive_cutovers(&s.state, &s.gamedata)
        .into_iter()
        .find(|c| c.new_factory == new)
        .unwrap();
    let dismantle = c
        .steps
        .iter()
        .find(|st| st.phase == CutoverPhase::Dismantle)
        .unwrap();
    assert!(dismantle.done && dismantle.overridden, "override pins done");

    // re-import removes OLD → RemoveFactory sync; accept it
    reimport_without_old(&mut s);
    // the override rode on the old factory id → cascade-pruned with the ◆
    assert!(!s.state.build_overrides.contains_key(&old));
    assert_eq!(s.state.factories[&new].replaces, None);
}

/// Re-import removing the old ◆ nulls the new factory's `replaces` (mirrors the
/// planned-delta dissolve): the link is intent, and the intent is now spent.
#[test]
fn replaces_nulled_when_old_factory_removed_on_reimport() {
    let mut s = Session::in_memory(None).unwrap();
    let (_seed, old) = import_seed_and_old(&mut s);
    let new = planned_factory(&mut s, "NEW", 400.0, 0.0);
    s.edit(vec![Command::SetFactoryReplaces {
        id: new.clone(),
        replaces: Some(old.clone()),
    }])
    .unwrap();
    assert_eq!(s.state.factories[&new].replaces, Some(old.clone()));

    reimport_without_old(&mut s);
    assert_eq!(
        s.state.factories[&new].replaces, None,
        "dangling replaces nulled on re-import"
    );
    // synthetic switch-step id helper is stable/namespaced
    assert_eq!(switch_step_id(&old, SCREW), format!("switch:{old}:{SCREW}"));
}
