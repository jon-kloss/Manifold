//! W1c build queue: the queue is DERIVED (no ordering entity, no stored
//! done-flag) and the manual override is one undoable overlay that resolves as
//! `override ?? derived` and auto-dissolves on re-import.

use app::buildqueue::{BuildStepKind, BuildStepState};
use app::import::{ImportMachine, ImportSnapshot};
use app::Session;
use planner_core::commands::Command;
use planner_core::entities::*;
use planner_core::proposals::*;

fn mach(class: &str, recipe: &str, x: f64, y: f64) -> ImportMachine {
    ImportMachine {
        class: class.into(),
        recipe: Some(recipe.into()),
        clock: 1.0,
        x,
        y,
        z: 0.0,
        ..Default::default()
    }
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

fn add_group(s: &mut Session, fid: &Id, machine: &str, recipe: &str) -> Id {
    s.edit(vec![Command::AddGroup {
        factory: fid.clone(),
        machine: machine.into(),
        recipe: recipe.into(),
        count: 1,
        clock: 1.0,
        graph_pos: GraphPos { x: 0.0, y: 0.0 },
        floor: 0,
    }])
    .unwrap()
    .created[0]
        .clone()
}

fn add_port(s: &mut Session, fid: &Id, dir: PortDirection, item: &str) -> Id {
    s.edit(vec![Command::AddPort {
        factory: fid.clone(),
        direction: dir,
        item: item.into(),
        rate: 0.0,
        rate_ceiling: None,
        graph_pos: GraphPos { x: 0.0, y: 0.0 },
    }])
    .unwrap()
    .created[0]
        .clone()
}

/// A planned factory with no built twin is a single ◇ Pending step.
#[test]
fn all_pending_with_no_built_twin() {
    let mut s = Session::in_memory(None).unwrap();
    let f = planned_factory(&mut s, "IRON WORKS", 0.0, 0.0);
    add_group(&mut s, &f, "Build_SmelterMk1_C", "Recipe_IngotIron_C");
    let q = s.solve_all_readonly().build_queue;
    let step = q.iter().find(|st| st.id == f).expect("factory step");
    assert_eq!(step.kind, BuildStepKind::Factory);
    assert_eq!(step.state, BuildStepState::Pending);
    assert!(!step.done);
    assert!(!step.overridden);
    // the planned group rolls up into the factory — no separate step
    assert_eq!(
        q.iter()
            .filter(|st| st.kind == BuildStepKind::Group)
            .count(),
        0
    );
}

/// A ◆ built twin within 250m covering the (machine,recipe) flips the step Done.
#[test]
fn built_twin_within_range_flips_done() {
    let mut s = Session::in_memory(None).unwrap();
    let f = planned_factory(&mut s, "IRON WORKS", 0.0, 0.0);
    add_group(&mut s, &f, "Build_SmelterMk1_C", "Recipe_IngotIron_C");
    // import a built smelter at the same site → a built twin
    s.import_save(ImportSnapshot {
        save_name: "TWIN".into(),
        machines: vec![mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0)],
        ..Default::default()
    })
    .unwrap();
    let q = s.solve_all_readonly().build_queue;
    let step = q.iter().find(|st| st.id == f).expect("factory step");
    assert_eq!(step.state, BuildStepState::Done);
    assert!(step.done);
    assert!(!step.overridden, "derived Done needs no override");
}

/// A twin covering only some of the factory's groups is ◈ Partial.
#[test]
fn partial_coverage_is_half_built() {
    let mut s = Session::in_memory(None).unwrap();
    let f = planned_factory(&mut s, "MIXED WORKS", 0.0, 0.0);
    add_group(&mut s, &f, "Build_SmelterMk1_C", "Recipe_IngotIron_C");
    add_group(&mut s, &f, "Build_ConstructorMk1_C", "Recipe_IronRod_C");
    // built twin has ONLY the smelter
    s.import_save(ImportSnapshot {
        save_name: "HALF".into(),
        machines: vec![mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0)],
        ..Default::default()
    })
    .unwrap();
    let step = s
        .solve_all_readonly()
        .build_queue
        .into_iter()
        .find(|st| st.id == f)
        .unwrap();
    assert_eq!(step.state, BuildStepState::Partial);
    assert!(!step.done, "◈ half-built is not done");
}

/// A ◆ built group carrying a planned delta is a Pending step; when the delta
/// dissolves (game caught up) the step disappears — no Done flag stored.
#[test]
fn built_group_with_delta_is_pending_then_gone() {
    let mut s = Session::in_memory(None).unwrap();
    s.import_save(ImportSnapshot {
        save_name: "BASE".into(),
        machines: vec![
            mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
            mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0),
        ],
        ..Default::default()
    })
    .unwrap();
    let gid = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_SmelterMk1_C")
        .unwrap()
        .id
        .clone();
    // no delta yet → the built group is not a step
    assert!(!s
        .solve_all_readonly()
        .build_queue
        .iter()
        .any(|st| st.id == gid));
    // plan a bigger bank → a ◆-with-delta Pending step appears
    s.edit(vec![Command::SetGroupCount {
        id: gid.clone(),
        count: 4,
    }])
    .unwrap();
    let step = s
        .solve_all_readonly()
        .build_queue
        .into_iter()
        .find(|st| st.id == gid)
        .expect("delta step");
    assert_eq!(step.kind, BuildStepKind::Group);
    assert_eq!(step.state, BuildStepState::Pending);
    // set it back to the built baseline → delta dissolves → step gone
    s.edit(vec![Command::SetGroupCount {
        id: gid.clone(),
        count: 2,
    }])
    .unwrap();
    assert!(!s
        .solve_all_readonly()
        .build_queue
        .iter()
        .any(|st| st.id == gid));
}

/// Routes and claims cannot auto-complete (the save has machines, not belt
/// connectivity) — they are Pending + manual-only until an override, which
/// resolves `override ?? derived` and reverts to derived on removal.
#[test]
fn route_and_claim_are_manual_only_and_override_resolves() {
    let mut s = Session::in_memory(None).unwrap();
    let a = planned_factory(&mut s, "A", 0.0, 0.0);
    let b = planned_factory(&mut s, "B", 100.0, 0.0);
    let out = add_port(&mut s, &a, PortDirection::Out, "Desc_IronIngot_C");
    let inp = add_port(&mut s, &b, PortDirection::In, "Desc_IronIngot_C");
    let route = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Belt { tier: 1 },
            from: out,
            to: inp,
            path: vec![
                MapPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                MapPos {
                    x: 100.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        }])
        .unwrap()
        .created[0]
        .clone();
    let claim = s
        .edit(vec![Command::ClaimNode {
            factory: a.clone(),
            node: "bp_resourcenode496".into(),
            extractor: "Build_MinerMk2_C".into(),
            clock: 1.0,
        }])
        .unwrap()
        .created[0]
        .clone();

    let q = s.solve_all_readonly().build_queue;
    let rstep = q.iter().find(|st| st.id == route).expect("route step");
    assert_eq!(rstep.kind, BuildStepKind::Route);
    assert!(rstep.manual_only);
    assert_eq!(rstep.state, BuildStepState::Pending);
    let cstep = q.iter().find(|st| st.id == claim).expect("claim step");
    assert_eq!(cstep.kind, BuildStepKind::Claim);
    assert!(cstep.manual_only);

    // manual mark-done: derived stays Pending, resolved done flips true
    s.edit(vec![Command::SetBuildDone {
        id: route.clone(),
        done: Some(true),
    }])
    .unwrap();
    let step = s
        .solve_all_readonly()
        .build_queue
        .into_iter()
        .find(|st| st.id == route)
        .unwrap();
    assert!(step.done && step.overridden);
    assert_eq!(step.state, BuildStepState::Pending, "derived is untouched");

    // clear the override → reverts to derived
    s.edit(vec![Command::SetBuildDone {
        id: route.clone(),
        done: None,
    }])
    .unwrap();
    let step = s
        .solve_all_readonly()
        .build_queue
        .into_iter()
        .find(|st| st.id == route)
        .unwrap();
    assert!(!step.done && !step.overridden);
}

/// Steps sort by (proposal number, ULID): the MANUAL bucket (0) leads, and
/// within a bucket ULIDs read chronologically.
#[test]
fn ordering_by_number_then_ulid() {
    let mut s = Session::in_memory(None).unwrap();
    // a numbered proposal that creates a factory (stamped with its number)
    let proposal = Proposal {
        id: String::new(),
        source: ProposalSource::GlobalSolver,
        title: "P".into(),
        goal: vec![],
        status: ProposalStatus::Draft,
        number: 5,
        snapshot_time: String::new(),
        input_hash: String::new(),
        provenance: String::new(),
        items: vec![ProposalItem {
            id: "it1".into(),
            kind: ProposalItemKind::Create,
            included: true,
            label: "+ SITE".into(),
            detail: String::new(),
            impact: String::new(),
            commands: vec![Command::CreateFactory {
                name: "PROP SITE".into(),
                position: MapPos {
                    x: 900.0,
                    y: 900.0,
                    z: 0.0,
                },
                region: String::new(),
            }],
            aliases: vec![None],
            depends_on: vec![],
            sync: None,
        }],
        milestone: None,
    };
    let pid = s
        .edit(vec![Command::CreateProposal { proposal }])
        .unwrap()
        .created[0]
        .clone();
    s.accept_proposal(&pid).unwrap();
    // two manual factories in creation order
    let m1 = planned_factory(&mut s, "MANUAL ONE", 0.0, 0.0);
    let m2 = planned_factory(&mut s, "MANUAL TWO", 10.0, 0.0);

    let q = s.solve_all_readonly().build_queue;
    let pos = |id: &Id| q.iter().position(|st| &st.id == id).unwrap();
    let prop_pos = q.iter().position(|st| st.label == "PROP SITE").unwrap();
    // manual bucket (0) sorts ahead of the proposal (5) regardless of ULID
    assert!(pos(&m1) < prop_pos);
    assert!(pos(&m2) < prop_pos);
    // ULID chronological within the manual bucket
    assert!(pos(&m1) < pos(&m2));
    assert_eq!(q[prop_pos].number, 5, "proposal step carries its number");
    assert_eq!(q[pos(&m1)].number, 0, "manual step is bucket 0");
}

/// Milestone progress lights on a step whose proposal carries a milestone;
/// "built" is empire production of the item from the ◆ built layer.
#[test]
fn milestone_progress_lights_from_built_production() {
    let mut s = Session::in_memory(None).unwrap();
    // a ◆ built smelter already producing iron ingot
    s.import_save(ImportSnapshot {
        save_name: "GEN".into(),
        machines: vec![mach(
            "Build_SmelterMk1_C",
            "Recipe_IngotIron_C",
            1000.0,
            1000.0,
        )],
        ..Default::default()
    })
    .unwrap();
    let proposal = Proposal {
        id: String::new(),
        source: ProposalSource::GlobalSolver,
        title: "M".into(),
        goal: vec![("Desc_IronIngot_C".into(), 10.0)],
        status: ProposalStatus::Draft,
        number: 0,
        snapshot_time: String::new(),
        input_hash: String::new(),
        provenance: String::new(),
        items: vec![ProposalItem {
            id: "m1".into(),
            kind: ProposalItemKind::Create,
            included: true,
            label: "+ MS".into(),
            detail: String::new(),
            impact: String::new(),
            commands: vec![Command::CreateFactory {
                name: "MS".into(),
                position: MapPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                region: String::new(),
            }],
            aliases: vec![None],
            depends_on: vec![],
            sync: None,
        }],
        milestone: Some(Milestone {
            item: "Desc_IronIngot_C".into(),
            total: 100.0,
            rate: 10.0,
        }),
    };
    let pid = s
        .edit(vec![Command::CreateProposal { proposal }])
        .unwrap()
        .created[0]
        .clone();
    s.accept_proposal(&pid).unwrap();
    let step = s
        .solve_all_readonly()
        .build_queue
        .into_iter()
        .find(|st| st.label == "MS")
        .expect("planned milestone site");
    let prog = step.progress.as_ref().expect("milestone progress lit");
    assert_eq!(prog.item, "Desc_IronIngot_C");
    assert_eq!(prog.total, 100.0);
    assert!(
        prog.built > 0.0,
        "built production from the ◆ smelter: {}",
        prog.built
    );
}

/// SetBuildDone is one undoable step: the override appears, then ⌘Z removes
/// just the override (the factory it rode on survives).
#[test]
fn set_build_done_is_one_undoable_step() {
    let mut s = Session::in_memory(None).unwrap();
    let f = planned_factory(&mut s, "SITE", 0.0, 0.0);
    assert!(
        !s.solve_all_readonly()
            .build_queue
            .iter()
            .find(|st| st.id == f)
            .unwrap()
            .done
    );

    s.edit(vec![Command::SetBuildDone {
        id: f.clone(),
        done: Some(true),
    }])
    .unwrap();
    assert_eq!(
        s.state.build_overrides.get(&f).map(|o| o.done),
        Some(true),
        "override upserted"
    );
    let step = s
        .solve_all_readonly()
        .build_queue
        .into_iter()
        .find(|st| st.id == f)
        .unwrap();
    assert!(step.done && step.overridden);

    // one undo removes ONLY the override — the factory remains
    s.undo().unwrap().unwrap();
    assert!(!s.state.build_overrides.contains_key(&f), "override gone");
    assert!(
        s.state.factories.contains_key(&f),
        "factory untouched by the undo"
    );
    let step = s
        .solve_all_readonly()
        .build_queue
        .into_iter()
        .find(|st| st.id == f)
        .unwrap();
    assert!(!step.done && !step.overridden, "reverted to derived");
}

/// Re-import drift accept auto-dissolves an override the game has caught up to
/// (mirrors the planned-delta dissolve): mark a planned factory done by hand,
/// then re-import a save that actually builds it — the override becomes
/// redundant and is dropped, leaving derived Done.
#[test]
fn override_auto_dissolves_on_reimport() {
    let mut s = Session::in_memory(None).unwrap();
    // seed a built layer elsewhere so the next import is a re-import (drift)
    s.import_save(ImportSnapshot {
        save_name: "SEED".into(),
        machines: vec![mach(
            "Build_SmelterMk1_C",
            "Recipe_IngotIron_C",
            5000.0,
            5000.0,
        )],
        ..Default::default()
    })
    .unwrap();
    // a planned factory the player hand-marks done (ahead of the game)
    let f = planned_factory(&mut s, "IRON WORKS", 0.0, 0.0);
    add_group(&mut s, &f, "Build_SmelterMk1_C", "Recipe_IngotIron_C");
    s.edit(vec![Command::SetBuildDone {
        id: f.clone(),
        done: Some(true),
    }])
    .unwrap();
    assert!(s.state.build_overrides.contains_key(&f));

    // re-import: the seed factory is unchanged, plus a NEW built cluster at the
    // planned site → drift proposal, accept it
    let outcome = s
        .import_save(ImportSnapshot {
            save_name: "BUILT".into(),
            machines: vec![
                mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", 5000.0, 5000.0),
                mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
            ],
            ..Default::default()
        })
        .unwrap();
    let pid = match outcome {
        app::session::ImportOutcome::Drift { proposal, .. } => proposal,
        other => panic!("expected drift, got {other:?}"),
    };
    s.accept_proposal(&pid).unwrap();

    // the game caught up: the override agrees with derived → dissolved
    assert!(
        !s.state.build_overrides.contains_key(&f),
        "redundant override auto-dissolved on re-import"
    );
    let step = s
        .solve_all_readonly()
        .build_queue
        .into_iter()
        .find(|st| st.id == f)
        .unwrap();
    assert!(
        step.done && !step.overridden,
        "derived Done stands on its own"
    );
}
