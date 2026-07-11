//! Save import: cluster → Built layer; re-import → drift proposal (never
//! writes); accepting drift syncs the Built layer in one undo entry.

use app::import::{ImportMachine, ImportSnapshot};
use app::session::ImportOutcome;
use app::Session;
use planner_core::entities::Status;

fn m(class: &str, recipe: &str, x: f64, y: f64) -> ImportMachine {
    ImportMachine {
        class: class.into(),
        recipe: Some(recipe.into()),
        clock: 1.0,
        x,
        y,
        z: 0.0,
    }
}

fn mc(class: &str, recipe: &str, x: f64, y: f64, clock: f64) -> ImportMachine {
    ImportMachine {
        clock,
        ..m(class, recipe, x, y)
    }
}

fn snapshot(machines: Vec<ImportMachine>) -> ImportSnapshot {
    ImportSnapshot {
        save_name: "TEST-01".into(),
        machines,
        ..Default::default()
    }
}

#[test]
fn first_import_writes_built_layer_then_reimport_diffs() {
    let mut s = Session::in_memory(None).unwrap();

    // two spatial clusters: 3 smelters at origin-ish, 2 constructors 1km away
    let machines = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
        m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 1000.0, 1000.0),
        m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 1050.0, 1000.0),
    ];
    let outcome = s.import_save(snapshot(machines.clone())).unwrap();
    let ImportOutcome::Imported {
        factories,
        machines: mcount,
        ..
    } = outcome
    else {
        panic!("expected first import");
    };
    assert_eq!(factories, 2, "DBSCAN separates the 1km-apart banks");
    assert_eq!(mcount, 5);
    assert_eq!(s.state.factories.len(), 2);
    assert!(
        s.state
            .factories
            .values()
            .all(|f| f.status == Status::Built),
        "imported = ◆ built"
    );
    assert!(s
        .state
        .factories
        .values()
        .any(|f| f.name.starts_with("IRON INGOT")));
    let smelter_bank = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_SmelterMk1_C")
        .unwrap();
    assert_eq!(smelter_bank.count, 3);
    assert_eq!(smelter_bank.status, Status::Built);

    // one undo entry removes the whole import
    s.undo().unwrap().unwrap();
    assert_eq!(s.state.factories.len(), 0);
    s.redo().unwrap().unwrap();
    assert_eq!(s.state.factories.len(), 2);

    // identical re-import: in sync, nothing written
    let outcome = s.import_save(snapshot(machines)).unwrap();
    assert!(matches!(outcome, ImportOutcome::InSync));
    assert!(s.state.proposals.is_empty());

    // game changed: smelter bank grew to 5, a new far cluster appeared
    let changed = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 30.0, 90.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 80.0, 10.0),
        m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 1000.0, 1000.0),
        m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 1050.0, 1000.0),
        m(
            "Build_AssemblerMk1_C",
            "Recipe_ModularFrame_C",
            3000.0,
            -2000.0,
        ),
    ];
    let factories_before = s.state.factories.len();
    let outcome = s.import_save(snapshot(changed)).unwrap();
    let ImportOutcome::Drift { proposal, .. } = outcome else {
        panic!("expected drift");
    };
    // re-import never writes: still 2 built factories, drift is a proposal
    assert_eq!(s.state.factories.len(), factories_before);
    let p = &s.state.proposals[&proposal];
    assert_eq!(
        p.source,
        planner_core::proposals::ProposalSource::SaveReimport
    );
    assert!(
        p.items.iter().any(|i| i.detail.contains("×3 built → ×5")),
        "count drift: {:?}",
        p.items
            .iter()
            .map(|i| (&i.label, &i.detail))
            .collect::<Vec<_>>()
    );
    assert!(p.items.iter().any(|i| i.label.contains("NEW IN GAME")));

    // accepting drift syncs the built layer (documented ◇-only exception)
    s.accept_proposal(&proposal).unwrap();
    let smelter_bank = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_SmelterMk1_C")
        .unwrap();
    assert_eq!(smelter_bank.count, 5, "built count synced");
    assert_eq!(
        s.state.factories.len(),
        factories_before + 1,
        "new cluster materialized"
    );
    // one undo restores the pre-sync built layer
    s.undo().unwrap().unwrap();
    assert_eq!(
        s.state
            .groups
            .values()
            .find(|g| g.machine == "Build_SmelterMk1_C")
            .unwrap()
            .count,
        3
    );
    assert_eq!(s.state.factories.len(), factories_before);
}

#[test]
fn sync_clears_caught_up_delta_and_keeps_still_ahead_delta() {
    use planner_core::commands::Command;
    use planner_core::entities::GroupDelta;

    let mut s = Session::in_memory(None).unwrap();
    let smelters = |n: usize| -> Vec<ImportMachine> {
        (0..n)
            .map(|i| {
                m(
                    "Build_SmelterMk1_C",
                    "Recipe_IngotIron_C",
                    50.0 * i as f64,
                    0.0,
                )
            })
            .collect()
    };
    s.import_save(snapshot(smelters(3))).unwrap();
    let gid = s.state.groups.values().next().unwrap().id.clone();

    // Plan an expansion on the ◆ bank: ×3 → ×5, retuned to 150%.
    s.edit(vec![
        Command::SetGroupCount {
            id: gid.clone(),
            count: 5,
        },
        Command::SetGroupClock {
            id: gid.clone(),
            clock: 1.5,
        },
    ])
    .unwrap();
    assert_eq!(
        s.state.groups[&gid].planned_delta,
        Some(GroupDelta {
            count: Some(5),
            clock: Some(1.5),
        })
    );

    // The game built the 2 extra smelters (still at 100%): accepting the drift
    // sync moves the baseline and dissolves the caught-up count component,
    // while the still-ahead clock retune stays user intent.
    let ImportOutcome::Drift { proposal, .. } = s.import_save(snapshot(smelters(5))).unwrap()
    else {
        panic!("expected drift");
    };
    s.accept_proposal(&proposal).unwrap();
    let g = &s.state.groups[&gid];
    assert_eq!(g.count, 5, "baseline synced to the game");
    assert_eq!(
        g.planned_delta,
        Some(GroupDelta {
            count: None,
            clock: Some(1.5),
        }),
        "count caught up → cleared; clock still ahead → kept"
    );

    // Baseline-keyed drift diff is unaffected by the remaining delta: an
    // identical save re-imports IN SYNC and the delta survives untouched.
    assert!(matches!(
        s.import_save(snapshot(smelters(5))).unwrap(),
        ImportOutcome::InSync
    ));
    assert_eq!(
        s.state.groups[&gid].planned_delta,
        Some(GroupDelta {
            count: None,
            clock: Some(1.5),
        })
    );
}

#[test]
fn import_auto_wires_groups_ports_and_preserves_built_counts() {
    let mut s = Session::in_memory(None).unwrap();
    // one cluster: 2 smelters (60 ingot/min) feeding 1 rod constructor
    // (consumes 15 ingot/min, makes 15 rod/min)
    let outcome = s
        .import_save(snapshot(vec![
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0),
            m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 100.0, 0.0),
        ]))
        .unwrap();
    assert!(matches!(
        outcome,
        ImportOutcome::Imported { factories: 1, .. }
    ));
    let f = s.state.factories.values().next().unwrap().clone();

    // boundary ports materialize the factory's net I/O
    let port = |dir: planner_core::entities::PortDirection, item: &str| {
        f.ports
            .iter()
            .filter_map(|pid| s.state.ports.get(pid))
            .find(|p| p.direction == dir && p.item == item)
    };
    use planner_core::entities::PortDirection::{In, Out};
    let ore = port(In, "Desc_OreIron_C").expect("ore In port");
    assert!(
        (ore.rate - 60.0).abs() < 1e-6,
        "ore need 60/min, got {}",
        ore.rate
    );
    let surplus = port(Out, "Desc_IronIngot_C").expect("ingot surplus Out port");
    assert!((surplus.rate - 45.0).abs() < 1e-6);
    let rods = port(Out, "Desc_IronRod_C").expect("rod Out port");
    assert!((rods.rate - 15.0).abs() < 1e-6);

    // internal wiring: smelter group edges into the constructor group
    let smelters = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_SmelterMk1_C")
        .unwrap();
    let rodmakers = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_ConstructorMk1_C")
        .unwrap();
    use planner_core::entities::EdgeEnd;
    assert!(
        s.state.edges.values().any(|e| e.item == "Desc_IronIngot_C"
            && e.from == EdgeEnd::Group(smelters.id.clone())
            && e.to == EdgeEnd::Group(rodmakers.id.clone())),
        "ingot edge smelters→constructor"
    );

    // the import already empire-solved: built counts are ground truth and
    // must survive the solver untouched
    assert_eq!(smelters.count, 2);
    assert_eq!(rodmakers.count, 1);
    assert_eq!(smelters.status, Status::Built);

    // layered layout: flow reads left→right (ore port → smelters →
    // constructor → rod port)
    assert!(ore.graph_pos.x < smelters.graph_pos.x);
    assert!(smelters.graph_pos.x < rodmakers.graph_pos.x);
    assert!(rodmakers.graph_pos.x < rods.graph_pos.x);
}

#[test]
fn reimport_new_nearby_cluster_cannot_steal_identity() {
    let mut s = Session::in_memory(None).unwrap();

    // Built factory F: 3 smelters, centroid ≈ (56.7, 40).
    let smelters = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
    ];
    s.import_save(snapshot(smelters.clone())).unwrap();
    assert_eq!(s.state.factories.len(), 1);

    // Re-import: a new 2-constructor outpost ~195 m from F's centroid,
    // emitted FIRST by DBSCAN (listed first), plus F's unchanged smelters.
    // Greedy-in-iteration-order matching would let the outpost steal F.
    let mut machines = vec![
        m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 250.0, 40.0),
        m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 250.0, 90.0),
    ];
    machines.extend(smelters);
    let ImportOutcome::Drift { proposal, .. } = s.import_save(snapshot(machines)).unwrap() else {
        panic!("expected drift (new outpost)");
    };
    let p = &s.state.proposals[&proposal];
    assert_eq!(
        p.items.len(),
        1,
        "only the new outpost drifts: {:?}",
        p.items
            .iter()
            .map(|i| (&i.label, &i.detail))
            .collect::<Vec<_>>()
    );
    assert!(p.items[0].label.contains("NEW IN GAME"));
    assert!(!p
        .items
        .iter()
        .any(|i| i.label.contains("demolished") || i.label.contains("reclocked")));

    // Accept: F keeps its identity (smelter bank untouched), outpost is new.
    s.accept_proposal(&proposal).unwrap();
    let smelter_bank = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_SmelterMk1_C")
        .unwrap();
    assert_eq!(smelter_bank.count, 3, "F's bank not corrupted by the steal");
    assert_eq!(s.state.factories.len(), 2);
}

#[test]
fn demolished_factory_emits_drift_and_accept_removes_cleanly() {
    let mut s = Session::in_memory(None).unwrap();

    // Two factories far apart: A (smelters) and B (constructors).
    s.import_save(snapshot(vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
        m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 5000.0, 5000.0),
        m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 5050.0, 5000.0),
    ]))
    .unwrap();
    let b = s
        .state
        .factories
        .values()
        .find(|f| f.name.starts_with("IRON ROD"))
        .unwrap()
        .clone();
    let count_owned = |s: &Session| {
        (
            s.state
                .groups
                .values()
                .filter(|g| g.factory == b.id)
                .count(),
            s.state.ports.values().filter(|p| p.factory == b.id).count(),
            s.state.edges.values().filter(|e| e.factory == b.id).count(),
        )
    };
    let before = count_owned(&s);
    assert!(before.0 > 0 && before.1 > 0 && before.2 > 0, "B is wired");

    // Re-import with B fully demolished in game.
    let ImportOutcome::Drift { proposal, .. } = s
        .import_save(snapshot(vec![
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
        ]))
        .unwrap()
    else {
        panic!("expected drift (demolished factory)");
    };
    let p = &s.state.proposals[&proposal];
    assert!(
        p.items
            .iter()
            .any(|i| i.label.contains("demolished in game") && i.label.contains(&b.name)),
        "honest drift item: {:?}",
        p.items
            .iter()
            .map(|i| (&i.label, &i.detail))
            .collect::<Vec<_>>()
    );

    // Accept: B and everything it owns is gone — no orphans.
    s.accept_proposal(&proposal).unwrap();
    assert!(!s.state.factories.contains_key(&b.id));
    assert_eq!(count_owned(&s), (0, 0, 0), "no orphaned groups/ports/edges");
    // A untouched.
    assert_eq!(
        s.state
            .groups
            .values()
            .find(|g| g.machine == "Build_SmelterMk1_C")
            .unwrap()
            .count,
        3
    );

    // One undo restores B with identical entity counts.
    s.undo().unwrap().unwrap();
    assert!(s.state.factories.contains_key(&b.id));
    assert_eq!(count_owned(&s), before, "undo restores the full cascade");
}

#[test]
fn clock_only_drift_emits_honest_item() {
    let mut s = Session::in_memory(None).unwrap();
    let smelters = |clock: f64| -> Vec<ImportMachine> {
        vec![
            mc("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0, clock),
            mc(
                "Build_SmelterMk1_C",
                "Recipe_IngotIron_C",
                60.0,
                40.0,
                clock,
            ),
            mc(
                "Build_SmelterMk1_C",
                "Recipe_IngotIron_C",
                110.0,
                80.0,
                clock,
            ),
        ]
    };
    s.import_save(snapshot(smelters(1.0))).unwrap();

    // Same machines, retuned to 150% in game: count matches, clock drifts.
    let ImportOutcome::Drift { proposal, .. } = s.import_save(snapshot(smelters(1.5))).unwrap()
    else {
        panic!("expected drift (reclocked)");
    };
    let p = &s.state.proposals[&proposal];
    let item = p
        .items
        .iter()
        .find(|i| i.label.contains("reclocked in game"))
        .expect("honest clock-drift item");
    assert!(item.detail.contains("100"), "detail: {}", item.detail);
    assert!(item.detail.contains("150"), "detail: {}", item.detail);

    // Accept syncs the clock, keeps the count; then re-import is IN SYNC.
    s.accept_proposal(&proposal).unwrap();
    let g = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_SmelterMk1_C")
        .unwrap();
    assert_eq!(g.count, 3);
    assert!((g.clock - 1.5).abs() < 1e-9);
    assert!(matches!(
        s.import_save(snapshot(smelters(1.5))).unwrap(),
        ImportOutcome::InSync
    ));
}

#[test]
fn clock_noise_within_tolerance_is_in_sync() {
    let mut s = Session::in_memory(None).unwrap();
    let smelters = |clock: f64| -> Vec<ImportMachine> {
        vec![
            mc("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0, clock),
            mc(
                "Build_SmelterMk1_C",
                "Recipe_IngotIron_C",
                60.0,
                40.0,
                clock,
            ),
        ]
    };
    s.import_save(snapshot(smelters(1.0))).unwrap();
    // 0.2% off is representation noise, not a player reclock.
    assert!(matches!(
        s.import_save(snapshot(smelters(1.002))).unwrap(),
        ImportOutcome::InSync
    ));
}
