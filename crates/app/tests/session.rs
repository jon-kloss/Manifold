//! Session integration test shaped like the Phase 1 exit criterion:
//! place a factory, build the Modular Frame chain, drag the target, re-solve,
//! reopen the file, verify persistence + undo.

use app::Session;
use planner_core::commands::Command;
use planner_core::entities::*;

fn gp(x: f64, y: f64) -> GraphPos {
    GraphPos { x, y }
}

fn add_group(s: &mut Session, fid: &str, machine: &str, recipe: &str, pos: GraphPos) -> Id {
    let r = s
        .edit(vec![Command::AddGroup {
            factory: fid.into(),
            machine: machine.into(),
            recipe: recipe.into(),
            count: 1,
            clock: 1.0,
            graph_pos: pos,
        }])
        .unwrap();
    r.created[0].clone()
}

fn connect(s: &mut Session, fid: &str, from: EdgeEnd, to: EdgeEnd, item: &str, tier: u8) -> Id {
    let r = s
        .edit(vec![Command::AddEdge {
            factory: fid.into(),
            from,
            to,
            item: item.into(),
            tier,
        }])
        .unwrap();
    r.created[0].clone()
}

fn build_modular_frame_factory(s: &mut Session) -> (Id, Id, Id) {
    let r = s
        .edit(vec![Command::CreateFactory {
            name: "MODULAR WORKS".into(),
            position: MapPos {
                x: -1400.0,
                y: 2400.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap();
    let fid = r.created[0].clone();

    // Claim an iron node + input port with the extraction ceiling (Mk.1, normal purity = 60/min... use Mk.2 = 120).
    let r = s
        .edit(vec![Command::ClaimNode {
            factory: fid.clone(),
            node: "iron-gf-01".into(),
            extractor: "Build_MinerMk2_C".into(),
            clock: 1.0,
        }])
        .unwrap();
    let claim = s.state.node_claims.get(&r.created[0]).unwrap().clone();
    let ceiling = s.claim_rate(&claim);
    assert_eq!(ceiling, 120.0, "Mk.2 miner on a normal node");

    let r = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::In,
            item: "Desc_OreIron_C".into(),
            rate: 0.0,
            rate_ceiling: Some(ceiling),
            graph_pos: gp(0.0, 200.0),
        }])
        .unwrap();
    let in_port = r.created[0].clone();
    let r = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::Out,
            item: "Desc_ModularFrame_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: gp(1400.0, 200.0),
        }])
        .unwrap();
    let out_port = r.created[0].clone();

    let smelt = add_group(
        s,
        &fid,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        gp(200.0, 200.0),
    );
    let rods = add_group(
        s,
        &fid,
        "Build_ConstructorMk1_C",
        "Recipe_IronRod_C",
        gp(420.0, 80.0),
    );
    let plates = add_group(
        s,
        &fid,
        "Build_ConstructorMk1_C",
        "Recipe_IronPlate_C",
        gp(420.0, 320.0),
    );
    let screws = add_group(
        s,
        &fid,
        "Build_ConstructorMk1_C",
        "Recipe_Screw_C",
        gp(640.0, 80.0),
    );
    let rip = add_group(
        s,
        &fid,
        "Build_AssemblerMk1_C",
        "Recipe_IronPlateReinforced_C",
        gp(860.0, 260.0),
    );
    let mf = add_group(
        s,
        &fid,
        "Build_AssemblerMk1_C",
        "Recipe_ModularFrame_C",
        gp(1100.0, 160.0),
    );

    let g = EdgeEnd::Group;
    connect(
        s,
        &fid,
        EdgeEnd::Port(in_port.clone()),
        g(smelt.clone()),
        "Desc_OreIron_C",
        3,
    );
    connect(
        s,
        &fid,
        g(smelt.clone()),
        g(rods.clone()),
        "Desc_IronIngot_C",
        3,
    );
    connect(
        s,
        &fid,
        g(smelt.clone()),
        g(plates.clone()),
        "Desc_IronIngot_C",
        3,
    );
    connect(
        s,
        &fid,
        g(rods.clone()),
        g(screws.clone()),
        "Desc_IronRod_C",
        2,
    );
    connect(s, &fid, g(rods.clone()), g(mf.clone()), "Desc_IronRod_C", 2);
    connect(
        s,
        &fid,
        g(plates.clone()),
        g(rip.clone()),
        "Desc_IronPlate_C",
        2,
    );
    connect(
        s,
        &fid,
        g(screws.clone()),
        g(rip.clone()),
        "Desc_IronScrew_C",
        2,
    );
    connect(
        s,
        &fid,
        g(rip.clone()),
        g(mf.clone()),
        "Desc_IronPlateReinforced_C",
        1,
    );
    connect(
        s,
        &fid,
        g(mf.clone()),
        EdgeEnd::Port(out_port.clone()),
        "Desc_ModularFrame_C",
        1,
    );

    (fid, out_port, smelt)
}

#[test]
fn exit_criterion_flow() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.ficsit");

    let (fid, out_port, smelt);
    {
        let mut s = Session::open(&path, None, "fixture").unwrap();
        let ids = build_modular_frame_factory(&mut s);
        fid = ids.0;
        out_port = ids.1;
        smelt = ids.2;

        // Drag the target to 2/min and commit — the whole chain re-solves.
        let r = s
            .edit(vec![Command::SetPortRate {
                id: out_port.clone(),
                rate: 2.0,
            }])
            .unwrap();
        let df = &r.derived.factories[&fid];
        assert!((df.ports[&out_port] - 2.0).abs() < 1e-6);
        // Golden chain: ore 24T = 48/min through the input port.
        let in_port_rate: f64 = df
            .ports
            .iter()
            .filter(|(pid, _)| s.state.ports[*pid].direction == PortDirection::In)
            .map(|(_, r)| *r)
            .sum();
        assert!(
            (in_port_rate - 48.0).abs() < 1e-4,
            "ore at 24T: {in_port_rate}"
        );
        // Solver wrote counts back into canonical state (same undo entry).
        assert_eq!(s.state.groups[&smelt].count, 2);
        assert!((s.state.groups[&smelt].clock - 0.8).abs() < 1e-6);
        // Ceiling honest: ore = 24 per frame, so 120 ore/min caps the target at 5.
        let ceiling = df.target_ceiling.as_ref().expect("ceiling reported");
        assert!(
            (ceiling.max_rate - 5.0).abs() < 1e-4,
            "max {}",
            ceiling.max_rate
        );

        // Saturation coloring data: mk1 RIP belt at 3/min ÷ 60 = 5%.
        assert!(df.edges.values().any(|e| e.saturation > 0.0));

        // Undo folds the solve into the same entry: counts revert with the rate.
        let r = s.undo().unwrap().unwrap();
        assert_eq!(
            s.state.groups[&smelt].count, 1,
            "solve write-back undone with the edit"
        );
        assert!(r.can_redo);
        let _ = s.redo().unwrap().unwrap();
        assert_eq!(s.state.groups[&smelt].count, 2);
    }

    // Reopen: everything persisted, undo still works.
    {
        let mut s = Session::open(&path, None, "fixture").unwrap();
        assert_eq!(s.state.factories[&fid].name, "MODULAR WORKS");
        assert_eq!(s.state.ports[&out_port].rate, 2.0);
        assert_eq!(s.state.groups[&smelt].count, 2);
        let hydrate = s.hydrate();
        assert!(hydrate["canUndo"].as_bool().unwrap());
        let r = s.undo().unwrap().unwrap();
        assert!(!r.patches.is_empty());
        assert_eq!(s.state.groups[&smelt].count, 1);
        assert!((s.state.ports[&out_port].rate - 0.0).abs() < 1e-9);
    }
}

#[test]
fn infeasible_target_clamps_and_names_constraint() {
    let mut s = Session::in_memory(None).unwrap();
    let (fid, out_port, _) = build_modular_frame_factory(&mut s);
    // 120 ore/min ceiling → max 5 frames/min. Ask for 8.
    let r = s
        .edit(vec![Command::SetPortRate {
            id: out_port.clone(),
            rate: 8.0,
        }])
        .unwrap();
    let df = &r.derived.factories[&fid];
    let ceiling = df.target_ceiling.as_ref().expect("ceiling");
    assert!((ceiling.max_rate - 5.0).abs() < 1e-4);
    // The committed rate settled at the ceiling, not the request (hard stop).
    assert!((s.state.ports[&out_port].rate - 5.0).abs() < 1e-4);
    match &ceiling.binding {
        solver::model::Constraint::InputCeiling { item, .. } => assert_eq!(item, "Desc_OreIron_C"),
        other => panic!("expected input ceiling, got {other:?}"),
    }
}

#[test]
fn failed_multi_command_edit_rolls_back() {
    let mut s = Session::in_memory(None).unwrap();
    let r = s
        .edit(vec![Command::CreateFactory {
            name: "X".into(),
            position: MapPos { x: 0.0, y: 0.0 },
            region: "".into(),
        }])
        .unwrap();
    let fid = r.created[0].clone();
    let before = s.state.clone();
    let err = s.edit(vec![
        Command::RenameFactory {
            id: fid.clone(),
            name: "Y".into(),
        },
        Command::RenameFactory {
            id: "missing".into(),
            name: "Z".into(),
        },
    ]);
    assert!(err.is_err());
    assert_eq!(s.state, before, "partial edit must leave no trace");
}
