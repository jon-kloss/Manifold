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
