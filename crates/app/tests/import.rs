//! Save import: cluster → Built layer; re-import → drift proposal (never
//! writes); accepting drift syncs the Built layer in one undo entry.

use app::import::{ImportMachine, ImportSnapshot};
use app::session::ImportOutcome;
use app::Session;
use planner_core::entities::{EdgeEnd, PortDirection, Status};

fn m(class: &str, recipe: &str, x: f64, y: f64) -> ImportMachine {
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

// A re-import where BOTH the user and the save changed the SAME component is a
// conflict: accept is blocked until a side is chosen, and each side resolves as
// documented (keep-mine re-anchors the user's target on the new baseline;
// take-save discards the edit).
#[test]
fn same_component_edit_and_save_change_is_a_conflict() {
    use planner_core::commands::Command;
    use planner_core::entities::GroupDelta;
    use planner_core::proposals::ConflictSide;

    // 2 smelters @ 100% imported; user retunes the bank to 150% in the app.
    let mut s = Session::in_memory(None).unwrap();
    s.import_save(snapshot(vec![
        mc("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0, 1.0),
        mc("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0, 1.0),
    ]))
    .unwrap();
    let gid = s.state.groups.values().next().unwrap().id.clone();
    s.edit(vec![Command::SetGroupClock {
        id: gid.clone(),
        clock: 1.5,
    }])
    .unwrap();

    // The game ALSO retuned the same bank — but to 200%. Same component, both
    // sides changed ⇒ conflict.
    let ImportOutcome::Drift { proposal, .. } = s
        .import_save(snapshot(vec![
            mc("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0, 2.0),
            mc("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0, 2.0),
        ]))
        .unwrap()
    else {
        panic!("expected drift");
    };
    let item = s.state.proposals[&proposal]
        .items
        .iter()
        .find(|i| i.conflict.is_some())
        .expect("a conflict item")
        .clone();
    assert!(
        item.conflict.as_ref().unwrap().choice.is_none(),
        "conflict starts undecided"
    );
    // Undecided conflict blocks the whole accept — we always ask.
    assert!(s.accept_proposal(&proposal).is_err());

    // KEEP MINE: baseline moves to the save's 200%, but the user's 150% stays
    // the effective target (re-anchored, not reverted).
    s.edit(vec![Command::SetProposalItemChoice {
        proposal: proposal.clone(),
        item: item.id.clone(),
        choice: Some(ConflictSide::Mine),
    }])
    .unwrap();
    s.accept_proposal(&proposal).unwrap();
    let g = &s.state.groups[&gid];
    assert!((g.clock - 2.0).abs() < 1e-9, "baseline synced to the game");
    assert_eq!(
        g.planned_delta,
        Some(GroupDelta {
            count: None,
            clock: Some(1.5),
        }),
        "your 150% kept as the plan target"
    );
}

// When the user edited TWO components but only one collides, the "keep mine"
// label must advertise the true effective values (keep-mine preserves the whole
// delta), and accepting keep-mine must deliver exactly that.
#[test]
fn keep_mine_label_reflects_all_edited_components() {
    use planner_core::commands::Command;
    use planner_core::entities::GroupDelta;
    use planner_core::proposals::ConflictSide;

    // 4 smelters @ 100%; user edits BOTH count→8 and clock→150%.
    let mut s = Session::in_memory(None).unwrap();
    s.import_save(snapshot(
        (0..4)
            .map(|i| {
                mc(
                    "Build_SmelterMk1_C",
                    "Recipe_IngotIron_C",
                    50.0 * i as f64,
                    0.0,
                    1.0,
                )
            })
            .collect(),
    ))
    .unwrap();
    let gid = s.state.groups.values().next().unwrap().id.clone();
    s.edit(vec![
        Command::SetGroupCount {
            id: gid.clone(),
            count: 8,
        },
        Command::SetGroupClock {
            id: gid.clone(),
            clock: 1.5,
        },
    ])
    .unwrap();

    // The game changed ONLY the count (4→6), clock still 100%. Count collides
    // (user 8 ≠ save 6); clock does not (the save didn't move it).
    let ImportOutcome::Drift { proposal, .. } = s
        .import_save(snapshot(
            (0..6)
                .map(|i| {
                    mc(
                        "Build_SmelterMk1_C",
                        "Recipe_IngotIron_C",
                        50.0 * i as f64,
                        0.0,
                        1.0,
                    )
                })
                .collect(),
        ))
        .unwrap()
    else {
        panic!("expected drift");
    };
    let item = s.state.proposals[&proposal]
        .items
        .iter()
        .find(|i| i.conflict.is_some())
        .expect("a conflict item")
        .clone();
    let conflict = item.conflict.as_ref().unwrap();
    // The mine label must show BOTH of the user's edits, not just the colliding
    // count — 150% clock included even though the save didn't touch it.
    assert_eq!(conflict.mine, "×8 @ 150%");
    assert_eq!(conflict.theirs, "×6 @ 100%");

    s.edit(vec![Command::SetProposalItemChoice {
        proposal: proposal.clone(),
        item: item.id.clone(),
        choice: Some(ConflictSide::Mine),
    }])
    .unwrap();
    s.accept_proposal(&proposal).unwrap();
    let g = &s.state.groups[&gid];
    assert_eq!(g.count, 6, "baseline synced to the game count");
    assert_eq!(
        g.planned_delta,
        Some(GroupDelta {
            count: Some(8),
            clock: Some(1.5),
        }),
        "both edits kept — effective ×8 @ 150%, exactly what the label promised"
    );
}

#[test]
fn take_save_discards_the_in_app_edit() {
    use planner_core::commands::Command;
    use planner_core::proposals::ConflictSide;

    let mut s = Session::in_memory(None).unwrap();
    s.import_save(snapshot(vec![mc(
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        0.0,
        0.0,
        1.0,
    )]))
    .unwrap();
    let gid = s.state.groups.values().next().unwrap().id.clone();
    s.edit(vec![Command::SetGroupClock {
        id: gid.clone(),
        clock: 1.5,
    }])
    .unwrap();
    let ImportOutcome::Drift { proposal, .. } = s
        .import_save(snapshot(vec![mc(
            "Build_SmelterMk1_C",
            "Recipe_IngotIron_C",
            0.0,
            0.0,
            2.0,
        )]))
        .unwrap()
    else {
        panic!("expected drift");
    };
    let item_id = s.state.proposals[&proposal]
        .items
        .iter()
        .find(|i| i.conflict.is_some())
        .expect("a conflict item")
        .id
        .clone();
    s.edit(vec![Command::SetProposalItemChoice {
        proposal: proposal.clone(),
        item: item_id,
        choice: Some(ConflictSide::Theirs),
    }])
    .unwrap();
    s.accept_proposal(&proposal).unwrap();
    let g = &s.state.groups[&gid];
    assert!((g.clock - 2.0).abs() < 1e-9);
    assert_eq!(g.planned_delta, None, "your edit discarded — the save won");
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

// ── cluster(): grid-indexed DBSCAN vs the original O(n²) algorithm ─────────
//
// cluster() was rewritten to use a uniform grid index with mark-on-push
// (CODE-REVIEW M17). These tests pin its semantics to the ORIGINAL brute-force
// algorithm (inlined below as the reference oracle) and smoke-test the two
// failure modes of the old code: O(n²) time on big saves and multiplicative
// duplicate stack pushes on dense cliques.

use app::import::cluster;
use app::import::Cluster;

/// Fixed-seed LCG (Knuth MMIX constants) — deterministic, no new deps.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f()
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// The ORIGINAL cluster assignment loop (pre-grid, mark-on-pop, full scan),
/// kept verbatim as the reference oracle. Returns clusters in emission order
/// (ids assigned in outer-loop seed order), members ascending.
fn reference_clusters(pts: &[(f64, f64)]) -> Vec<Vec<usize>> {
    const EPS: f64 = 120.0;
    let mut cluster_of: Vec<Option<usize>> = vec![None; pts.len()];
    let mut n_clusters = 0usize;
    for i in 0..pts.len() {
        if cluster_of[i].is_some() {
            continue;
        }
        let id = n_clusters;
        n_clusters += 1;
        let mut stack = vec![i];
        while let Some(j) = stack.pop() {
            if cluster_of[j].is_some() {
                continue;
            }
            cluster_of[j] = Some(id);
            for (k, p) in pts.iter().enumerate() {
                if cluster_of[k].is_none() && (p.0 - pts[j].0).hypot(p.1 - pts[j].1) <= EPS {
                    stack.push(k);
                }
            }
        }
    }
    let mut clusters = vec![Vec::new(); n_clusters];
    for (k, c) in cluster_of.iter().enumerate() {
        clusters[c.unwrap()].push(k);
    }
    clusters
}

/// Recover each cluster's member indices from unique per-machine classes
/// ("C<idx>"), in emission order — observable through the public API because
/// every machine forms its own group.
fn member_sets(clusters: &[Cluster]) -> Vec<Vec<usize>> {
    clusters
        .iter()
        .map(|c| {
            let mut v: Vec<usize> = c
                .groups
                .iter()
                .map(|g| g.machine[1..].parse().unwrap())
                .collect();
            v.sort_unstable();
            v
        })
        .collect()
}

fn run_case(points: Vec<(f64, f64)>) {
    let machines: Vec<ImportMachine> = points
        .iter()
        .enumerate()
        .map(|(i, (x, y))| m(&format!("C{i}"), "", *x, *y))
        .collect();
    let gd = gamedata::docs::GameData::default();
    let got = member_sets(&cluster(&snapshot(machines), &gd));
    let want = reference_clusters(&points);
    assert_eq!(
        got,
        want,
        "cluster membership/emission order diverged from the original \
         algorithm on {} points",
        points.len()
    );
}

fn shuffle(rng: &mut Lcg, pts: &mut [(f64, f64)]) {
    for i in (1..pts.len()).rev() {
        pts.swap(i, rng.below(i + 1));
    }
}

#[test]
fn cluster_matches_bruteforce_reference_on_random_inputs() {
    let mut rng = Lcg(0x5EED_F1C5_17E5_2026);
    for case in 0..50 {
        let mut pts: Vec<(f64, f64)> = Vec::new();
        match case % 5 {
            // uniform sparse over the map
            0 => {
                let n = 1 + rng.below(400);
                for _ in 0..n {
                    pts.push((rng.range(-5000.0, 5000.0), rng.range(-5000.0, 5000.0)));
                }
            }
            // dense Gaussian-ish clumps, radius < eps
            1 => {
                for _ in 0..2 + rng.below(4) {
                    let (cx, cy) = (rng.range(-4000.0, 4000.0), rng.range(-4000.0, 4000.0));
                    for _ in 0..20 + rng.below(80) {
                        pts.push((cx + rng.range(-55.0, 55.0), cy + rng.range(-55.0, 55.0)));
                    }
                }
            }
            // chained lines at ~100 m spacing (cross-cell neighbors)
            2 => {
                for _ in 0..1 + rng.below(4) {
                    let (mut x, mut y) = (rng.range(-3000.0, 3000.0), rng.range(-3000.0, 3000.0));
                    let a = rng.range(0.0, std::f64::consts::TAU);
                    let (dx, dy) = (a.cos() * 100.0, a.sin() * 100.0);
                    for _ in 0..30 + rng.below(90) {
                        pts.push((x, y));
                        x += dx;
                        y += dy;
                    }
                }
            }
            // exact duplicates
            3 => {
                for _ in 0..1 + rng.below(40) {
                    let p = (rng.range(-2000.0, 2000.0), rng.range(-2000.0, 2000.0));
                    for _ in 0..1 + rng.below(8) {
                        pts.push(p);
                    }
                }
            }
            // mix of all regimes
            _ => {
                for _ in 0..rng.below(100) {
                    pts.push((rng.range(-5000.0, 5000.0), rng.range(-5000.0, 5000.0)));
                }
                let (cx, cy) = (rng.range(-1000.0, 1000.0), rng.range(-1000.0, 1000.0));
                for _ in 0..rng.below(120) {
                    pts.push((cx + rng.range(-55.0, 55.0), cy + rng.range(-55.0, 55.0)));
                }
                let p = (rng.range(-500.0, 500.0), rng.range(-500.0, 500.0));
                for _ in 0..rng.below(10) {
                    pts.push(p);
                }
            }
        }
        pts.truncate(400);
        shuffle(&mut rng, &mut pts);
        run_case(pts);
    }
}

#[test]
fn cluster_degenerate_inputs() {
    let gd = gamedata::docs::GameData::default();

    // empty snapshot → no clusters
    assert!(cluster(&snapshot(vec![]), &gd).is_empty());

    // single machine → one cluster
    let c = cluster(&snapshot(vec![m("Build_SmelterMk1_C", "", 3.0, 4.0)]), &gd);
    assert_eq!(c.len(), 1);

    // all-coincident points → one cluster, one group of 60
    let c = cluster(
        &snapshot(
            (0..60)
                .map(|_| m("Build_SmelterMk1_C", "", 42.0, -42.0))
                .collect(),
        ),
        &gd,
    );
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].groups.len(), 1);
    assert_eq!(c[0].groups[0].count, 60);

    // non-finite coordinates stay singletons and don't poison neighbors
    // (NaN/±inf fail the distance test against everything, as before)
    let c = cluster(
        &snapshot(vec![
            m("Normal", "", 0.0, 0.0),
            m("Normal2", "", 50.0, 0.0),
            m("NanBox", "", f64::NAN, 0.0),
            m("InfBox", "", f64::INFINITY, f64::NEG_INFINITY),
        ]),
        &gd,
    );
    assert_eq!(c.len(), 3, "pair + NaN singleton + inf singleton");
    assert_eq!(c[0].groups.len(), 2);
    assert_eq!(c[1].groups[0].machine, "NanBox");
    assert_eq!(c[2].groups[0].machine, "InfBox");
}

/// 20k machines in one giant chained grid (60 m spacing < eps): the old code
/// took seconds here (O(n²) scans); the grid index must stay well under 2 s.
#[test]
fn perf_smoke_20k_chained_grid() {
    let machines: Vec<ImportMachine> = (0..20_000)
        .map(|i| {
            m(
                "Build_SmelterMk1_C",
                "Recipe_IngotIron_C",
                (i % 142) as f64 * 60.0,
                (i / 142) as f64 * 60.0,
            )
        })
        .collect();
    let snap = snapshot(machines);
    let gd = gamedata::docs::GameData::default();
    let t = std::time::Instant::now();
    let clusters = cluster(&snap, &gd);
    let dt = t.elapsed();
    eprintln!("20k chained grid clustered in {dt:?}");
    assert_eq!(clusters.len(), 1, "one connected component");
    assert!(dt < std::time::Duration::from_secs(2), "took {dt:?}");
}

/// 10k machines on one 100 m pad — the old code's memory-blowup case
/// (duplicate pushes grew the stack toward O(n²) entries). Mark-on-push +
/// bucket pruning make it linear.
#[test]
fn perf_smoke_10k_clique() {
    let mut rng = Lcg(0xC11_0E2026);
    let machines: Vec<ImportMachine> = (0..10_000)
        .map(|_| {
            m(
                "Build_SmelterMk1_C",
                "Recipe_IngotIron_C",
                rng.range(-50.0, 50.0),
                rng.range(-50.0, 50.0),
            )
        })
        .collect();
    let snap = snapshot(machines);
    let gd = gamedata::docs::GameData::default();
    let t = std::time::Instant::now();
    let clusters = cluster(&snap, &gd);
    let dt = t.elapsed();
    eprintln!("10k clique clustered in {dt:?}");
    assert_eq!(clusters.len(), 1);
    assert!(dt < std::time::Duration::from_secs(2), "took {dt:?}");
}

/// Strict local bench — run with `cargo test -- --ignored`.
#[test]
#[ignore = "strict perf bound for local runs; CI uses the 2 s smoke above"]
fn perf_strict_20k_chained_grid() {
    let machines: Vec<ImportMachine> = (0..20_000)
        .map(|i| {
            m(
                "Build_SmelterMk1_C",
                "Recipe_IngotIron_C",
                (i % 142) as f64 * 60.0,
                (i / 142) as f64 * 60.0,
            )
        })
        .collect();
    let snap = snapshot(machines);
    let gd = gamedata::docs::GameData::default();
    let t = std::time::Instant::now();
    let clusters = cluster(&snap, &gd);
    let dt = t.elapsed();
    assert_eq!(clusters.len(), 1);
    assert!(dt < std::time::Duration::from_millis(250), "took {dt:?}");
}

// ---- W2b-A: snapshot carries unlocked schematics + extractor node context ----

/// A legacy snapshot JSON lacking every W2b-A field still deserializes: the new
/// fields are serde-default (empty schematics, `None` node context). Proves old
/// snapshots/plan files load with no migration.
#[test]
fn old_snapshot_without_new_fields_deserializes() {
    let json = r#"{
        "saveName": "LEGACY",
        "machines": [
            { "class": "Build_SmelterMk1_C", "recipe": "Recipe_IngotIron_C", "x": 0.0, "y": 0.0 }
        ]
    }"#;
    let snap: ImportSnapshot = serde_json::from_str(json).unwrap();
    assert!(snap.unlocked_schematics.is_empty());
    assert_eq!(snap.machines.len(), 1);
    let mc = &snap.machines[0];
    assert_eq!(mc.node_actor_id, None);
    assert_eq!(mc.resource, None);
    assert_eq!(mc.purity, None);
    assert_eq!(mc.extraction_rate, None);
    // serde default clock still applies.
    assert_eq!(mc.clock, 1.0);
}

/// A snapshot WITH the new extractor fields + unlocked schematics round-trips.
#[test]
fn new_extractor_fields_round_trip() {
    let json = r#"{
        "saveName": "W2B",
        "machines": [],
        "extractors": [
            {
                "class": "Build_MinerMk2_C",
                "recipe": null,
                "clock": 2.5,
                "x": 12.0,
                "y": 34.0,
                "nodeActorId": "Persistent_Level:PersistentLevel.BP_ResourceNode109",
                "resource": null,
                "purity": null
            }
        ],
        "unlockedSchematics": ["Schematic_1-2_C", "Recipe_Alternate_Screw_C"]
    }"#;
    let snap: ImportSnapshot = serde_json::from_str(json).unwrap();
    assert_eq!(
        snap.unlocked_schematics,
        vec![
            "Schematic_1-2_C".to_string(),
            "Recipe_Alternate_Screw_C".to_string()
        ]
    );
    let e = &snap.extractors[0];
    assert_eq!(
        e.node_actor_id.as_deref(),
        Some("Persistent_Level:PersistentLevel.BP_ResourceNode109")
    );
    assert_eq!(e.purity, None);
    assert_eq!(e.clock, 2.5);
    // Round-trip: serialize back and re-read the node ref survives.
    let round = serde_json::to_string(&snap).unwrap();
    let snap2: ImportSnapshot = serde_json::from_str(&round).unwrap();
    assert_eq!(snap2.extractors[0].node_actor_id, e.node_actor_id);
    assert_eq!(snap2.unlocked_schematics, snap.unlocked_schematics);
}

/// W2b: import resolves the unlocked recipe set from mPurchasedSchematics ×
/// FGSchematic unlocks, persists it as a META fact (outside the undo journal),
/// reloads it on reopen, and surfaces it through hydrate as `unlocked`.
#[cfg(feature = "sqlite")]
#[test]
fn unlocked_set_resolves_from_schematics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.ficsit");
    {
        let mut s = app::Session::open(&path, None, "fixture").unwrap();
        // synthetic FGSchematic mapping — the trimmed fixture ships none.
        s.gamedata.schematics.insert(
            "Schematic_Alt_C".into(),
            vec!["Recipe_Alternate_Screw_C".into()],
        );
        let snap = ImportSnapshot {
            save_name: "UNLOCK-01".into(),
            machines: vec![m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0)],
            unlocked_schematics: vec![
                "Schematic_Alt_C".into(),
                "Schematic_Unmapped_C".into(), // no mapping → contributes nothing
            ],
            ..Default::default()
        };
        s.import_save(snap).unwrap();
        assert!(
            s.unlocked.contains("Recipe_Alternate_Screw_C"),
            "purchased schematic resolves to its unlocked recipe"
        );
        assert_eq!(
            s.unlocked.len(),
            1,
            "unmapped schematics contribute nothing"
        );
        let h = s.hydrate();
        let arr = h["unlocked"]
            .as_array()
            .expect("hydrate carries an unlocked array");
        assert!(arr
            .iter()
            .any(|v| v.as_str() == Some("Recipe_Alternate_Screw_C")));
    }
    // reopen: the META blob round-trips through the persist layer.
    let mut s2 = app::Session::open(&path, None, "fixture").unwrap();
    assert!(
        s2.unlocked.contains("Recipe_Alternate_Screw_C"),
        "unlocked set survives reopen"
    );
    assert_eq!(s2.hydrate()["unlocked"].as_array().unwrap().len(), 1);
}

/// The trimmed fixture catalog ships no schematics → import resolves an empty
/// unlocked set → alternates behave exactly as before (no-regression guard).
#[test]
fn fixture_yields_empty_unlocked() {
    let mut s = Session::in_memory(None).unwrap();
    assert!(
        s.gamedata.schematics.is_empty(),
        "fixture has no schematics"
    );
    let snap = ImportSnapshot {
        save_name: "FIX-01".into(),
        machines: vec![m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0)],
        unlocked_schematics: vec!["Schematic_Whatever_C".into()],
        ..Default::default()
    };
    s.import_save(snap).unwrap();
    assert!(
        s.unlocked.is_empty(),
        "no schematic catalog → nothing unlocks"
    );
    assert!(s.hydrate()["unlocked"].as_array().unwrap().is_empty());
}

/// Review minor M13: each new drift diff supersedes every still-open one (a
/// newer diff is a cumulative superset). Stale open SaveReimport proposals are
/// rejected in the same edit that drafts the new one, so the review surface and
/// PLAN DRIFT tab can never offer obsolete SyncOps whose accept would rewrite
/// the ◆ layer with old counts.
#[test]
fn reimport_supersedes_stale_open_drift_proposals() {
    use planner_core::proposals::ProposalStatus;
    let mut s = Session::in_memory(None).unwrap();
    let base = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
    ];
    s.import_save(snapshot(base)).unwrap();

    // Drift #1: the bank grew to 3.
    let drift1 = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
    ];
    let ImportOutcome::Drift { proposal: p1, .. } = s.import_save(snapshot(drift1)).unwrap() else {
        panic!("expected drift #1");
    };
    assert_eq!(s.state.proposals[&p1].status, ProposalStatus::Draft);

    // Drift #2 (user kept playing): the bank grew to 4.
    let drift2 = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 30.0, 90.0),
    ];
    let ImportOutcome::Drift { proposal: p2, .. } = s.import_save(snapshot(drift2)).unwrap() else {
        panic!("expected drift #2");
    };
    // The stale diff is closed; only the newest is open — and accepting the
    // stale one is refused outright.
    assert_eq!(s.state.proposals[&p1].status, ProposalStatus::Rejected);
    assert_eq!(s.state.proposals[&p2].status, ProposalStatus::Draft);
    assert!(
        s.accept_proposal(&p1).is_err(),
        "stale drift cannot be applied"
    );
    // One undo unwinds the supersede + new draft together (one edit batch).
    s.undo().unwrap().unwrap();
    assert_eq!(s.state.proposals[&p1].status, ProposalStatus::Draft);
    assert!(!s.state.proposals.contains_key(&p2));
}

/// An in-sync re-import never writes, so an older drift diff stays Draft —
/// but its SyncOps describe a save state that no longer exists. Accept keys
/// on the last_import blob's proposal identity (the diff the NEWEST import
/// drafted, null for in-sync) and refuses the moot one.
#[test]
fn in_sync_reimport_makes_stale_drift_unacceptable() {
    use planner_core::proposals::ProposalStatus;
    let mut s = Session::in_memory(None).unwrap();
    let base = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
    ];
    s.import_save(snapshot(base.clone())).unwrap();

    // Drift: the bank grew to 3.
    let grown = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
    ];
    let ImportOutcome::Drift { proposal: p1, .. } = s.import_save(snapshot(grown)).unwrap() else {
        panic!("expected drift");
    };

    // The game caught back down (older save loaded): identical-to-built
    // re-import is IN SYNC and writes nothing — hash unchanged, P1 untouched.
    let hash = s.plan_hash();
    assert!(matches!(
        s.import_save(snapshot(base)).unwrap(),
        ImportOutcome::InSync
    ));
    assert_eq!(s.plan_hash(), hash, "in-sync re-import never writes");
    assert_eq!(s.state.proposals[&p1].status, ProposalStatus::Draft);

    // ... yet its ×2 → ×3 SyncOp is moot now: accept is refused, named as such.
    let err = s.accept_proposal(&p1).unwrap_err();
    assert!(
        format!("{err}").contains("superseded"),
        "names the supersede: {err}"
    );
    assert_eq!(
        s.state.proposals[&p1].status,
        ProposalStatus::Draft,
        "refused accept leaves the proposal untouched"
    );
    assert_eq!(
        s.state
            .groups
            .values()
            .find(|g| g.machine == "Build_SmelterMk1_C")
            .unwrap()
            .count,
        2,
        "the stale diff never rewrote the ◆ layer"
    );
}

/// The identity gate keys on the blob's proposal id, so the diff the newest
/// import drafted — the only one whose SyncOps match the save — accepts fine.
#[test]
fn current_drift_proposal_accepts_normally() {
    let mut s = Session::in_memory(None).unwrap();
    let base = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
    ];
    s.import_save(snapshot(base)).unwrap();
    let grown = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
    ];
    let ImportOutcome::Drift { proposal, .. } = s.import_save(snapshot(grown)).unwrap() else {
        panic!("expected drift");
    };
    s.accept_proposal(&proposal).unwrap();
    assert_eq!(
        s.state
            .groups
            .values()
            .find(|g| g.machine == "Build_SmelterMk1_C")
            .unwrap()
            .count,
        3,
        "current diff syncs the built layer"
    );
}

/// The supersede sweep covers Reviewing too — a diff mid-review when the next
/// import lands is just as stale as a Draft one.
#[test]
fn reimport_supersedes_reviewing_drift_proposal() {
    use planner_core::commands::Command;
    use planner_core::proposals::ProposalStatus;
    let mut s = Session::in_memory(None).unwrap();
    let base = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
    ];
    s.import_save(snapshot(base)).unwrap();
    let drift1 = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
    ];
    let ImportOutcome::Drift { proposal: p1, .. } = s.import_save(snapshot(drift1)).unwrap() else {
        panic!("expected drift #1");
    };
    s.edit(vec![Command::SetProposalStatus {
        id: p1.clone(),
        status: ProposalStatus::Reviewing,
    }])
    .unwrap();

    let drift2 = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 30.0, 90.0),
    ];
    let ImportOutcome::Drift { proposal: p2, .. } = s.import_save(snapshot(drift2)).unwrap() else {
        panic!("expected drift #2");
    };
    assert_eq!(s.state.proposals[&p1].status, ProposalStatus::Rejected);
    assert_eq!(s.state.proposals[&p2].status, ProposalStatus::Draft);
    assert!(s.accept_proposal(&p1).is_err(), "closed = unacceptable");
}

/// The supersede sweep is SaveReimport-scoped: an open draft from any other
/// source rides through a drift import untouched.
#[test]
fn drift_import_leaves_non_reimport_drafts_open() {
    use planner_core::proposals::{Proposal, ProposalSource, ProposalStatus};
    let mut s = Session::in_memory(None).unwrap();
    let base = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
    ];
    s.import_save(snapshot(base)).unwrap();
    let draft = Proposal {
        id: planner_core::entities::new_id(),
        source: ProposalSource::GlobalSolver,
        title: "WIZARD DRAFT".into(),
        goal: vec![],
        status: ProposalStatus::Draft,
        number: 0,
        snapshot_time: "2026-07-10T00:00:00Z".into(),
        input_hash: s.plan_hash(),
        provenance: "test".into(),
        items: vec![],
        milestone: None,
    };
    let wizard_pid = s
        .edit(vec![planner_core::commands::Command::CreateProposal {
            proposal: draft,
        }])
        .unwrap()
        .created[0]
        .clone();

    let grown = vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 60.0, 40.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 110.0, 80.0),
    ];
    let ImportOutcome::Drift { .. } = s.import_save(snapshot(grown)).unwrap() else {
        panic!("expected drift");
    };
    assert_eq!(
        s.state.proposals[&wizard_pid].status,
        ProposalStatus::Draft,
        "non-SaveReimport draft survives the sweep"
    );
    s.accept_proposal(&wizard_pid).unwrap();
}

/// Fuel-less generators (geothermal) are real machines: their clusters take
/// the machine's display name instead of the "IMPORTED" fallback, and their
/// variable-power average counts toward empire generation instead of 0.
/// Regression from the first real-save import: 20 geothermal units produced
/// 12 factories named "IMPORTED WORKS N" contributing no MW.
#[test]
fn geothermal_cluster_names_itself_and_counts_generation() {
    let mut s = Session::in_memory(None).unwrap();
    let machines = vec![
        m("Build_GeneratorGeoThermal_C", "", 0.0, 0.0),
        m("Build_GeneratorGeoThermal_C", "", 50.0, 0.0),
    ];
    s.import_save(snapshot(machines)).unwrap();

    let f = s
        .state
        .factories
        .values()
        .next()
        .expect("one imported factory");
    assert_eq!(
        f.name, "GEOTHERMAL GENERATOR WORKS 1",
        "fuel-less generator clusters name themselves by machine"
    );
    let hydrated = s.hydrate();
    let gen = hydrated["derived"]["totalGenerationMw"].as_f64().unwrap();
    assert!(
        (gen - 400.0).abs() < 1e-6,
        "2 geothermal x 200 MW average counts as generation, got {gen}"
    );
}

/// AUDIT #126 (1): a group demolished in game must cascade on drift accept
/// like DeleteGroup — no orphaned belts referencing the removed group, no
/// stale boundary port still exporting the item it made, and the surviving
/// port rates refreshed to the new net flow.
#[test]
fn demolished_group_sync_cascades_edges_and_ports() {
    let mut s = Session::in_memory(None).unwrap();
    // 2 smelters (60 ingot/min) + 1 constructor (15 rod/min, eats 15 ingot).
    s.import_save(snapshot(vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0),
        m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 100.0, 0.0),
    ]))
    .unwrap();
    let fid = s.state.factories.keys().next().unwrap().clone();
    let constructor_gid = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_ConstructorMk1_C")
        .expect("constructor group imported")
        .id
        .clone();
    assert!(
        s.state
            .ports
            .values()
            .any(|p| p.item == "Desc_IronRod_C" && p.direction == PortDirection::Out),
        "rod export exists before demolition"
    );

    // Re-import without the constructor → drift → accept.
    let ImportOutcome::Drift { proposal, .. } = s
        .import_save(snapshot(vec![
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0),
        ]))
        .unwrap()
    else {
        panic!("expected drift");
    };
    s.accept_proposal(&proposal).unwrap();

    assert!(
        !s.state.groups.contains_key(&constructor_gid),
        "demolished group removed"
    );
    let orphans: Vec<_> = s
        .state
        .edges
        .values()
        .filter(|e| {
            e.from == EdgeEnd::Group(constructor_gid.clone())
                || e.to == EdgeEnd::Group(constructor_gid.clone())
        })
        .collect();
    assert!(
        orphans.is_empty(),
        "no belt may reference the removed group"
    );
    assert!(
        !s.state
            .ports
            .values()
            .any(|p| p.factory == fid && p.item == "Desc_IronRod_C"),
        "the rod export the constructor sourced is gone"
    );
    // The ingot export absorbs the freed 15/min: 60 - 15 → 60.
    let ingot_out = s
        .state
        .ports
        .values()
        .find(|p| p.item == "Desc_IronIngot_C" && p.direction == PortDirection::Out)
        .expect("ingot export survives");
    assert!(
        (ingot_out.rate - 60.0).abs() < 1e-6,
        "ingot export refreshed to the full 60/min, got {}",
        ingot_out.rate
    );
    // One undo restores the constructor with its wiring.
    s.undo().unwrap().unwrap();
    assert!(s.state.groups.contains_key(&constructor_gid));
    assert!(s
        .state
        .ports
        .values()
        .any(|p| p.item == "Desc_IronRod_C" && p.direction == PortDirection::Out));
}

/// AUDIT #126 (2): a count-up drift accept must recompute the exported port
/// rate and raise the export belt's tier — otherwise the expanded factory
/// stays capped at its stale export (probe: 3→6 smelters kept reading 90).
#[test]
fn count_up_sync_recomputes_port_rate_and_belt_tier() {
    let mut s = Session::in_memory(None).unwrap();
    let three = || {
        vec![
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0),
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 100.0, 0.0),
        ]
    };
    s.import_save(snapshot(three())).unwrap();
    let out_port = s
        .state
        .ports
        .values()
        .find(|p| p.item == "Desc_IronIngot_C" && p.direction == PortDirection::Out)
        .expect("ingot export")
        .clone();
    assert!((out_port.rate - 90.0).abs() < 1e-6);

    let mut six = three();
    six.extend(vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 150.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 200.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 250.0, 0.0),
    ]);
    let ImportOutcome::Drift { proposal, .. } = s.import_save(snapshot(six)).unwrap() else {
        panic!("expected drift");
    };
    s.accept_proposal(&proposal).unwrap();

    let after = &s.state.ports[&out_port.id];
    assert!(
        (after.rate - 180.0).abs() < 1e-6,
        "export rate follows the doubled bank, got {}",
        after.rate
    );
    // 180/min outgrows MK.2 (120): the export belt must ride at least MK.3.
    let export_belt = s
        .state
        .edges
        .values()
        .find(|e| e.to == EdgeEnd::Port(out_port.id.clone()))
        .expect("export belt");
    assert!(
        export_belt.tier >= 3,
        "export belt raised to carry 180/min, got MK.{}",
        export_belt.tier
    );
    // The in-feed doubled too: 90 ore → 180 ore.
    let ore_in = s
        .state
        .ports
        .values()
        .find(|p| p.item == "Desc_OreIron_C" && p.direction == PortDirection::In)
        .expect("ore import");
    assert!(
        (ore_in.rate - 180.0).abs() < 1e-6,
        "ore import refreshed, got {}",
        ore_in.rate
    );
}

/// AUDIT #126 (3): a group added in game arrives wired on drift accept — belts
/// to/from its recipe partners and a refreshed boundary — not as an unwired
/// card at a hardcoded position.
#[test]
fn added_group_sync_auto_wires_into_the_factory() {
    let mut s = Session::in_memory(None).unwrap();
    s.import_save(snapshot(vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0),
    ]))
    .unwrap();

    let ImportOutcome::Drift { proposal, .. } = s
        .import_save(snapshot(vec![
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0),
            m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 100.0, 0.0),
        ]))
        .unwrap()
    else {
        panic!("expected drift");
    };
    s.accept_proposal(&proposal).unwrap();

    let smelter = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_SmelterMk1_C")
        .unwrap()
        .id
        .clone();
    let constructor = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_ConstructorMk1_C")
        .expect("added group materialized")
        .id
        .clone();
    assert!(
        s.state
            .edges
            .values()
            .any(|e| e.from == EdgeEnd::Group(smelter.clone())
                && e.to == EdgeEnd::Group(constructor.clone())
                && e.item == "Desc_IronIngot_C"),
        "ingot feed belted into the added constructor"
    );
    let rod_out = s
        .state
        .ports
        .values()
        .find(|p| p.item == "Desc_IronRod_C" && p.direction == PortDirection::Out)
        .expect("rod export created for the new product");
    assert!((rod_out.rate - 15.0).abs() < 1e-6);
    assert!(
        s.state
            .edges
            .values()
            .any(|e| e.from == EdgeEnd::Group(constructor.clone())
                && e.to == EdgeEnd::Port(rod_out.id.clone())),
        "rod export belted from the constructor"
    );
    // The ingot export shrinks to what the constructor leaves over: 60 → 45.
    let ingot_out = s
        .state
        .ports
        .values()
        .find(|p| p.item == "Desc_IronIngot_C" && p.direction == PortDirection::Out)
        .expect("ingot export survives");
    assert!(
        (ingot_out.rate - 45.0).abs() < 1e-6,
        "ingot export refreshed down to 45/min, got {}",
        ingot_out.rate
    );
}

/// Review fix (PR #56): a dried-up ◆ Built boundary port anchoring a
/// user-drawn inter-factory route is SPARED by the resync — kept at rate 0 so
/// the route survives — instead of cascading the planned route away with it.
#[test]
fn dried_up_port_with_user_route_keeps_the_route() {
    use planner_core::commands::Command;
    use planner_core::entities::{GraphPos, MapPos, RouteKind};

    let mut s = Session::in_memory(None).unwrap();
    // 2 smelters, no consumer → Built factory exporting 60 ingot/min.
    s.import_save(snapshot(vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0),
    ]))
    .unwrap();
    let out_port = s
        .state
        .ports
        .values()
        .find(|p| p.item == "Desc_IronIngot_C" && p.direction == PortDirection::Out)
        .expect("ingot export")
        .id
        .clone();

    // The user plans a downstream factory and draws a belt route from the
    // Built export to it.
    s.edit(vec![Command::CreateFactory {
        name: "DOWNSTREAM".into(),
        position: MapPos {
            x: 2000.0,
            y: 0.0,
            z: 0.0,
        },
        region: String::new(),
    }])
    .unwrap();
    let downstream = s
        .state
        .factories
        .values()
        .find(|f| f.name == "DOWNSTREAM")
        .unwrap()
        .id
        .clone();
    s.edit(vec![Command::AddPort {
        factory: downstream,
        direction: PortDirection::In,
        item: "Desc_IronIngot_C".into(),
        rate: 60.0,
        rate_ceiling: None,
        graph_pos: GraphPos { x: 0.0, y: 100.0 },
    }])
    .unwrap();
    let in_port = s
        .state
        .ports
        .values()
        .find(|p| {
            p.direction == PortDirection::In
                && p.item == "Desc_IronIngot_C"
                && p.status == Status::Planned
        })
        .unwrap()
        .id
        .clone();
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Belt { tier: 3 },
        from: out_port.clone(),
        to: in_port,
        path: vec![],
    }])
    .unwrap();
    let route = s.state.ports[&out_port]
        .bound_route
        .clone()
        .expect("route bound");

    // In game the player adds 4 constructors that eat the whole 60/min → the
    // ingot export dries up on drift accept.
    let ImportOutcome::Drift { proposal, .. } = s
        .import_save(snapshot(vec![
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0),
            m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 100.0, 0.0),
            m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 150.0, 0.0),
            m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 100.0, 50.0),
            m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 150.0, 50.0),
        ]))
        .unwrap()
    else {
        panic!("expected drift");
    };
    s.accept_proposal(&proposal).unwrap();

    // The routed port survives at rate 0; the user's route is untouched.
    let p = s.state.ports.get(&out_port).expect("routed port spared");
    assert!(
        p.rate.abs() < 1e-9,
        "spared port reads 0/min, got {}",
        p.rate
    );
    assert_eq!(p.bound_route.as_ref(), Some(&route));
    assert!(
        s.state.routes.contains_key(&route),
        "user-drawn route survives the drift accept"
    );
}

/// Review fix (PR #56): the planned-port coexistence guard — resync must NOT
/// create a Built boundary port for an item the user already models with a
/// planned port of the same direction.
#[test]
fn resync_skips_port_creation_when_user_planned_port_exists() {
    use planner_core::commands::Command;
    use planner_core::entities::GraphPos;

    let mut s = Session::in_memory(None).unwrap();
    s.import_save(snapshot(vec![
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
        m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0),
    ]))
    .unwrap();
    let fid = s.state.factories.keys().next().unwrap().clone();

    // The user already plans a rod export on the Built factory.
    s.edit(vec![Command::AddPort {
        factory: fid.clone(),
        direction: PortDirection::Out,
        item: "Desc_IronRod_C".into(),
        rate: 15.0,
        rate_ceiling: None,
        graph_pos: GraphPos { x: 900.0, y: 100.0 },
    }])
    .unwrap();

    // The game adds a rod constructor → drift accept nets rods out.
    let ImportOutcome::Drift { proposal, .. } = s
        .import_save(snapshot(vec![
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0, 0.0),
            m("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0, 0.0),
            m("Build_ConstructorMk1_C", "Recipe_IronRod_C", 100.0, 0.0),
        ]))
        .unwrap()
    else {
        panic!("expected drift");
    };
    s.accept_proposal(&proposal).unwrap();

    // Exactly ONE rod Out port — the user's planned one; no Built double.
    let rod_ports: Vec<_> = s
        .state
        .ports
        .values()
        .filter(|p| {
            p.factory == fid && p.item == "Desc_IronRod_C" && p.direction == PortDirection::Out
        })
        .collect();
    assert_eq!(
        rod_ports.len(),
        1,
        "no Built double for a user-planned boundary"
    );
    assert_eq!(rod_ports[0].status, Status::Planned);
}

/// Review fix (PR #56): raise_built_tier never LOWERS — after a count-down
/// drift the export belt keeps the higher tier the player may have overbuilt,
/// while the port rate honestly shrinks.
#[test]
fn count_down_sync_shrinks_rate_but_never_lowers_tier() {
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
    // 6 smelters → 180/min export over a MK.3 belt.
    s.import_save(snapshot(smelters(6))).unwrap();
    let out_port = s
        .state
        .ports
        .values()
        .find(|p| p.item == "Desc_IronIngot_C" && p.direction == PortDirection::Out)
        .expect("ingot export")
        .id
        .clone();
    let belt_before = s
        .state
        .edges
        .values()
        .find(|e| e.to == EdgeEnd::Port(out_port.clone()))
        .expect("export belt")
        .tier;
    assert!(belt_before >= 3);

    // The player tears down half the bank → 3 smelters.
    let ImportOutcome::Drift { proposal, .. } = s.import_save(snapshot(smelters(3))).unwrap()
    else {
        panic!("expected drift");
    };
    s.accept_proposal(&proposal).unwrap();

    let p = &s.state.ports[&out_port];
    assert!(
        (p.rate - 90.0).abs() < 1e-6,
        "export rate shrinks to 90/min, got {}",
        p.rate
    );
    let belt_after = s
        .state
        .edges
        .values()
        .find(|e| e.to == EdgeEnd::Port(out_port.clone()))
        .expect("export belt survives")
        .tier;
    assert_eq!(
        belt_after, belt_before,
        "tier is never lowered — the player may have overbuilt"
    );
}
