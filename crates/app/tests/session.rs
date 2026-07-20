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
            floor: 0,
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
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap();
    let fid = r.created[0].clone();

    // Claim an iron node + input port with the extraction ceiling (Mk.1, normal purity = 60/min... use Mk.2 = 120).
    let r = s
        .edit(vec![Command::ClaimNode {
            factory: fid.clone(),
            node: "bp_resourcenode496".into(),
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

#[cfg(feature = "sqlite")]
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
fn built_group_delta_feeds_the_solver_and_survives_write_back() {
    let mut s = Session::in_memory(None).unwrap();
    // built layer via import: 2 smelters (60 ingot/min) feeding 1 rod constructor
    let mach = |class: &str, recipe: &str, x: f64| app::import::ImportMachine {
        class: class.into(),
        recipe: Some(recipe.into()),
        clock: 1.0,
        x,
        y: 0.0,
        z: 0.0,
        ..Default::default()
    };
    s.import_save(app::import::ImportSnapshot {
        save_name: "TEST-01".into(),
        machines: vec![
            mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0),
            mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", 50.0),
            mach("Build_ConstructorMk1_C", "Recipe_IronRod_C", 100.0),
        ],
        ..Default::default()
    })
    .unwrap();
    let fid = s.state.factories.keys().next().unwrap().clone();
    let gid = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_SmelterMk1_C")
        .unwrap()
        .id
        .clone();
    assert_eq!(s.state.groups[&gid].status, Status::Built);
    assert_eq!(s.state.groups[&gid].count, 2);

    // A count edit on ◆ succeeds as a ◇ delta; the baseline stays ground truth.
    let r = s
        .edit(vec![Command::SetGroupCount {
            id: gid.clone(),
            count: 4,
        }])
        .unwrap();
    let g = &s.state.groups[&gid];
    assert_eq!(g.count, 2, "baseline untouched");
    assert_eq!(
        g.planned_delta,
        Some(GroupDelta {
            count: Some(4),
            clock: None,
        })
    );
    // The solve ran inside the same edit — its write-back must not squash the
    // delta or the baseline (◆ groups are skipped).
    let base_power = r.derived.factories[&fid].groups[&gid].power_mw;
    assert!(base_power > 0.0);

    // The solver snapshot plans with the effective values.
    let snap = s.snapshot(&fid).unwrap();
    let gs = snap.groups.iter().find(|g| g.id == gid).unwrap();
    assert_eq!(gs.count, 4, "snapshot reads the delta count");

    // A clock delta participates in the solve it triggers: underclocking the
    // bank spreads the same throughput at the sub-linear power law.
    let r = s
        .edit(vec![Command::SetGroupClock {
            id: gid.clone(),
            clock: 0.5,
        }])
        .unwrap();
    let g = &s.state.groups[&gid];
    assert!((g.clock - 1.0).abs() < 1e-9, "baseline clock untouched");
    assert_eq!(g.planned_delta.unwrap().clock, Some(0.5));
    let under_power = r.derived.factories[&fid].groups[&gid].power_mw;
    assert!(
        under_power < base_power,
        "underclock delta lowers derived power: {under_power} vs {base_power}"
    );

    // An unrelated solve-inducing edit leaves the delta alone.
    let out_port = s
        .state
        .ports
        .values()
        .find(|p| p.direction == PortDirection::Out && p.item == "Desc_IronRod_C")
        .unwrap()
        .id
        .clone();
    s.edit(vec![Command::SetPortRate {
        id: out_port,
        rate: 10.0,
    }])
    .unwrap();
    assert_eq!(
        s.state.groups[&gid].planned_delta,
        Some(GroupDelta {
            count: Some(4),
            clock: Some(0.5),
        })
    );
    assert_eq!(s.state.groups[&gid].count, 2);

    // Each delta edit is one undo step: unwind to the pristine built layer.
    s.undo().unwrap().unwrap(); // rate
    s.undo().unwrap().unwrap(); // clock delta
    assert_eq!(
        s.state.groups[&gid].planned_delta,
        Some(GroupDelta {
            count: Some(4),
            clock: None,
        })
    );
    s.undo().unwrap().unwrap(); // count delta
    assert_eq!(s.state.groups[&gid].planned_delta, None);
    assert_eq!(s.state.groups[&gid].count, 2);
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
fn partially_wired_factory_degrades_without_error_or_write_back() {
    let mut s = Session::in_memory(None).unwrap();
    let (fid, out_port, smelt) = build_modular_frame_factory(&mut s);
    // Sever the ore feed — the routine mid-construction state (SDD §5.2).
    let ore_edge = s
        .state
        .edges
        .values()
        .find(|e| e.item == "Desc_OreIron_C")
        .unwrap()
        .id
        .clone();
    s.edit(vec![Command::DeleteEdge { id: ore_edge }]).unwrap();
    let r = s
        .edit(vec![Command::SetPortRate {
            id: out_port.clone(),
            rate: 2.0,
        }])
        .unwrap();
    let df = &r.derived.factories[&fid];
    assert!(
        df.solve_error.is_none(),
        "must degrade, not dead-end: {:?}",
        df.solve_error
    );
    assert!((df.ports[&out_port] - 0.0).abs() < 1e-6, "achieved rate 0");
    let sf = df.shortfalls.get(&out_port).expect("shortfall reported");
    assert!((sf.requested - 2.0).abs() < 1e-6);
    assert!((sf.missing - 2.0).abs() < 1e-6);
    match &sf.binding {
        Some(solver::model::Constraint::Disconnected { node, item }) => {
            assert_eq!(node, &smelt);
            assert_eq!(item, "Desc_OreIron_C");
        }
        other => panic!("expected disconnected binding, got {other:?}"),
    }
    // Degraded solves are advisory: the user's target is NOT clamped away and
    // group counts are NOT rewritten to the starved values.
    assert!(
        (s.state.ports[&out_port].rate - 2.0).abs() < 1e-9,
        "target untouched: {}",
        s.state.ports[&out_port].rate
    );
    assert_eq!(s.state.groups[&smelt].count, 1, "counts not rewritten");
}

#[test]
fn junctions_split_and_enforce_port_caps() {
    let mut s = Session::in_memory(None).unwrap();
    let (fid, out_port, _) = build_modular_frame_factory(&mut s);

    // Insert a splitter on the rod run: rods → splitter → (screws, assembler).
    let rods = s
        .state
        .groups
        .values()
        .find(|g| g.recipe == "Recipe_IronRod_C")
        .unwrap()
        .id
        .clone();
    let screws = s
        .state
        .groups
        .values()
        .find(|g| g.recipe == "Recipe_Screw_C")
        .unwrap()
        .id
        .clone();
    let mf = s
        .state
        .groups
        .values()
        .find(|g| g.recipe == "Recipe_ModularFrame_C")
        .unwrap()
        .id
        .clone();
    let old_edges: Vec<Id> = s
        .state
        .edges
        .values()
        .filter(|e| e.from == EdgeEnd::Group(rods.clone()))
        .map(|e| e.id.clone())
        .collect();
    for eid in old_edges {
        s.edit(vec![Command::DeleteEdge { id: eid }]).unwrap();
    }
    let r = s
        .edit(vec![Command::AddJunction {
            factory: fid.clone(),
            kind: JunctionKind::Splitter,
            graph_pos: GraphPos { x: 700.0, y: 140.0 },
            floor: 0,
        }])
        .unwrap();
    let split = r.created[0].clone();
    let je = EdgeEnd::Junction(split.clone());
    connect_ends(
        &mut s,
        &fid,
        EdgeEnd::Group(rods.clone()),
        je.clone(),
        "Desc_IronRod_C",
        2,
    );
    connect_ends(
        &mut s,
        &fid,
        je.clone(),
        EdgeEnd::Group(screws),
        "Desc_IronRod_C",
        2,
    );
    connect_ends(
        &mut s,
        &fid,
        je.clone(),
        EdgeEnd::Group(mf),
        "Desc_IronRod_C",
        2,
    );

    // A splitter has exactly one input — a second feed must refuse.
    let err = s.edit(vec![Command::AddEdge {
        factory: fid.clone(),
        from: EdgeEnd::Group(rods.clone()),
        to: je.clone(),
        item: "Desc_IronRod_C".into(),
        tier: 1,
    }]);
    assert!(err.is_err(), "splitter input budget is 1");

    // The golden chain still solves identically through the junction.
    let r = s
        .edit(vec![Command::SetPortRate {
            id: out_port.clone(),
            rate: 2.0,
        }])
        .unwrap();
    let df = &r.derived.factories[&fid];
    assert!((df.ports[&out_port] - 2.0).abs() < 1e-6);
    let trunk = s
        .state
        .edges
        .values()
        .find(|e| e.to == je)
        .map(|e| e.id.clone())
        .unwrap();
    assert!(
        (df.edges[&trunk].flow - 21.0).abs() < 1e-4,
        "trunk carries full 10.5T rods"
    );
}

fn connect_ends(s: &mut Session, fid: &str, from: EdgeEnd, to: EdgeEnd, item: &str, tier: u8) {
    s.edit(vec![Command::AddEdge {
        factory: fid.into(),
        from,
        to,
        item: item.into(),
        tier,
    }])
    .unwrap();
}

#[test]
fn empire_routes_propagate_supply_and_deficits() {
    let mut s = Session::in_memory(None).unwrap();

    // Upstream: iron rod factory shipping rods out.
    let rod_fid = s
        .edit(vec![Command::CreateFactory {
            name: "ROD WORKS".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
        .clone();
    let rod_in = s
        .edit(vec![Command::AddPort {
            factory: rod_fid.clone(),
            direction: PortDirection::In,
            item: "Desc_OreIron_C".into(),
            rate: 0.0,
            rate_ceiling: Some(240.0),
            graph_pos: GraphPos { x: 0.0, y: 100.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let rod_out = s
        .edit(vec![Command::AddPort {
            factory: rod_fid.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronRod_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: GraphPos { x: 600.0, y: 100.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let smelt = add_group(
        &mut s,
        &rod_fid,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        gp(200.0, 100.0),
    );
    let rods = add_group(
        &mut s,
        &rod_fid,
        "Build_ConstructorMk1_C",
        "Recipe_IronRod_C",
        gp(400.0, 100.0),
    );
    let g = EdgeEnd::Group;
    connect_in(
        &mut s,
        &rod_fid,
        EdgeEnd::Port(rod_in),
        g(smelt.clone()),
        "Desc_OreIron_C",
        3,
    );
    connect_in(
        &mut s,
        &rod_fid,
        g(smelt),
        g(rods.clone()),
        "Desc_IronIngot_C",
        3,
    );
    connect_in(
        &mut s,
        &rod_fid,
        g(rods),
        EdgeEnd::Port(rod_out.clone()),
        "Desc_IronRod_C",
        3,
    );

    // Downstream: screw factory that wants rods from the route.
    let screw_fid = s
        .edit(vec![Command::CreateFactory {
            name: "SCREW WORKS".into(),
            position: MapPos {
                x: 500.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
        .clone();
    let screw_in = s
        .edit(vec![Command::AddPort {
            factory: screw_fid.clone(),
            direction: PortDirection::In,
            item: "Desc_IronRod_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: GraphPos { x: 0.0, y: 100.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let screw_out = s
        .edit(vec![Command::AddPort {
            factory: screw_fid.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronScrew_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: GraphPos { x: 600.0, y: 100.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let screws = add_group(
        &mut s,
        &screw_fid,
        "Build_ConstructorMk1_C",
        "Recipe_Screw_C",
        gp(300.0, 100.0),
    );
    connect_in(
        &mut s,
        &screw_fid,
        EdgeEnd::Port(screw_in.clone()),
        g(screws.clone()),
        "Desc_IronRod_C",
        3,
    );
    connect_in(
        &mut s,
        &screw_fid,
        g(screws),
        EdgeEnd::Port(screw_out.clone()),
        "Desc_IronScrew_C",
        3,
    );

    // Draw the route: rod OUT → screw IN, Mk.1 belt (60/min cap).
    let r = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Belt { tier: 1 },
            from: rod_out.clone(),
            to: screw_in.clone(),
            path: vec![
                MapPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                MapPos {
                    x: 300.0,
                    y: 400.0,
                    z: 0.0,
                },
            ],
        }])
        .unwrap();
    let route = r.created[0].clone();
    assert_eq!(
        s.state.ports[&rod_out].bound_route.as_deref(),
        Some(route.as_str())
    );

    // Upstream ships 30 rods/min; downstream wants 100 screws/min = 25 rods.
    s.edit(vec![Command::SetPortRate {
        id: rod_out.clone(),
        rate: 30.0,
    }])
    .unwrap();
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: screw_out.clone(),
            rate: 100.0,
        }])
        .unwrap();
    let d = &resp.derived;
    let rt = &d.routes[&route];
    assert!(
        (rt.supplied - 30.0).abs() < 1e-6,
        "supply = upstream rate: {}",
        rt.supplied
    );
    assert!(
        (rt.flow - 25.0).abs() < 1e-6,
        "downstream intake 25 rods: {}",
        rt.flow
    );
    assert!((rt.length_m - 500.0).abs() < 1e-6, "3-4-5 route length");
    assert!(d.deficits.is_empty(), "supply covers demand");
    // manifest is canonical and solver-maintained (§3.1.4)
    assert_eq!(
        s.state.routes[&route].manifest,
        vec![("Desc_IronRod_C".to_string(), 25.0)]
    );

    // Now demand beyond the supply: 200 screws needs 50 rods, only 30 ship.
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: screw_out.clone(),
            rate: 200.0,
        }])
        .unwrap();
    let d = &resp.derived;
    // downstream hard-stops at the supplied ceiling: 30 rods → 120 screws
    let clamped = s.state.ports[&screw_out].rate;
    assert!(
        (clamped - 120.0).abs() < 1e-4,
        "clamped at supply: {clamped}"
    );
    // the route runs saturated at 30/60
    assert!((d.routes[&route].flow - 30.0).abs() < 1e-4);
    // raising upstream un-starves downstream up to the Mk.1 belt cap
    s.edit(vec![Command::SetPortRate {
        id: rod_out.clone(),
        rate: 80.0,
    }])
    .unwrap();
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: screw_out.clone(),
            rate: 200.0,
        }])
        .unwrap();
    let clamped = s.state.ports[&screw_out].rate;
    assert!(
        (clamped - 200.0).abs() < 1e-4,
        "belt still allows 50 rods: {clamped}"
    );
    assert!((resp.derived.routes[&route].flow - 50.0).abs() < 1e-4);

    // Upstream drops back to 30 while downstream still wants 200: the target
    // must NOT be silently rewritten — it surfaces as a deficit instead.
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: rod_out.clone(),
            rate: 30.0,
        }])
        .unwrap();
    assert!(
        (s.state.ports[&screw_out].rate - 200.0).abs() < 1e-4,
        "target untouched"
    );
    let d = &resp.derived;
    assert_eq!(d.deficits.len(), 1, "starvation surfaces as a deficit row");
    let deficit = &d.deficits[0];
    assert_eq!(deficit.factory, screw_fid);
    assert!((deficit.supplied - 30.0).abs() < 1e-4);
    assert!(
        (deficit.needed - 50.0).abs() < 1e-4,
        "200 screws need 50 rods: {}",
        deficit.needed
    );
}

fn connect_in(s: &mut Session, fid: &str, from: EdgeEnd, to: EdgeEnd, item: &str, tier: u8) {
    s.edit(vec![Command::AddEdge {
        factory: fid.into(),
        from,
        to,
        item: item.into(),
        tier,
    }])
    .unwrap();
}

// ---- deficit-honesty fixtures (rod → screw empire, small variants) ----

fn mk_factory(s: &mut Session, name: &str, x: f64) -> Id {
    s.edit(vec![Command::CreateFactory {
        name: name.into(),
        position: MapPos { x, y: 0.0, z: 0.0 },
        region: "GRASS FIELDS".into(),
    }])
    .unwrap()
    .created[0]
        .clone()
}

fn mk_port(s: &mut Session, fid: &Id, dir: PortDirection, item: &str, ceiling: Option<f64>) -> Id {
    s.edit(vec![Command::AddPort {
        factory: fid.clone(),
        direction: dir,
        item: item.into(),
        rate: 0.0,
        rate_ceiling: ceiling,
        graph_pos: GraphPos { x: 0.0, y: 100.0 },
    }])
    .unwrap()
    .created[0]
        .clone()
}

/// Upstream iron-rod factory (ore in @240 ceiling → smelter → constructor →
/// rods out), same shape as `empire_routes_propagate_supply_and_deficits`.
fn build_rod_factory(s: &mut Session) -> (Id, Id) {
    let fid = mk_factory(s, "ROD WORKS", 0.0);
    let rod_in = mk_port(s, &fid, PortDirection::In, "Desc_OreIron_C", Some(240.0));
    let rod_out = mk_port(s, &fid, PortDirection::Out, "Desc_IronRod_C", None);
    let smelt = add_group(
        s,
        &fid,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        gp(200.0, 100.0),
    );
    let rods = add_group(
        s,
        &fid,
        "Build_ConstructorMk1_C",
        "Recipe_IronRod_C",
        gp(400.0, 100.0),
    );
    let g = EdgeEnd::Group;
    connect_in(
        s,
        &fid,
        EdgeEnd::Port(rod_in),
        g(smelt.clone()),
        "Desc_OreIron_C",
        3,
    );
    connect_in(s, &fid, g(smelt), g(rods.clone()), "Desc_IronIngot_C", 3);
    connect_in(
        s,
        &fid,
        g(rods),
        EdgeEnd::Port(rod_out.clone()),
        "Desc_IronRod_C",
        3,
    );
    (fid, rod_out)
}

/// Downstream screw factory with `n_outs` screw Out ports fed by one group.
fn build_screw_factory(s: &mut Session, n_outs: usize) -> (Id, Id, Vec<Id>, Id) {
    let fid = mk_factory(s, "SCREW WORKS", 500.0);
    let screw_in = mk_port(s, &fid, PortDirection::In, "Desc_IronRod_C", None);
    let outs: Vec<Id> = (0..n_outs)
        .map(|_| mk_port(s, &fid, PortDirection::Out, "Desc_IronScrew_C", None))
        .collect();
    let screws = add_group(
        s,
        &fid,
        "Build_ConstructorMk1_C",
        "Recipe_Screw_C",
        gp(300.0, 100.0),
    );
    connect_in(
        s,
        &fid,
        EdgeEnd::Port(screw_in.clone()),
        EdgeEnd::Group(screws.clone()),
        "Desc_IronRod_C",
        3,
    );
    for out in &outs {
        connect_in(
            s,
            &fid,
            EdgeEnd::Group(screws.clone()),
            EdgeEnd::Port(out.clone()),
            "Desc_IronScrew_C",
            3,
        );
    }
    (fid, screw_in, outs, screws)
}

fn add_rod_route(s: &mut Session, rod_out: &Id, screw_in: &Id) -> Id {
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Belt { tier: 1 },
        from: rod_out.clone(),
        to: screw_in.clone(),
        path: vec![
            MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            MapPos {
                x: 300.0,
                y: 400.0,
                z: 0.0,
            },
        ],
    }])
    .unwrap()
    .created[0]
        .clone()
}

/// M7: total starvation (supply ceiling exactly 0) must surface as a deficit
/// row — the old `max_rate > 0.0` guard dropped the most severe case.
#[test]
fn empire_total_starvation_emits_deficit_row() {
    let mut s = Session::in_memory(None).unwrap();
    let (_rod_fid, rod_out) = build_rod_factory(&mut s);
    let (screw_fid, screw_in, outs, _) = build_screw_factory(&mut s, 1);
    let screw_out = outs[0].clone();
    let route = add_rod_route(&mut s, &rod_out, &screw_in);

    // Healthy first: 80 rods shipped, 200 screws (needs 50 rods) — no clamp.
    s.edit(vec![Command::SetPortRate {
        id: rod_out.clone(),
        rate: 80.0,
    }])
    .unwrap();
    s.edit(vec![Command::SetPortRate {
        id: screw_out.clone(),
        rate: 200.0,
    }])
    .unwrap();
    assert!((s.state.ports[&screw_out].rate - 200.0).abs() < 1e-4);

    // Upstream target drops to exactly 0: supply ceiling 0, total starvation.
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: rod_out.clone(),
            rate: 0.0,
        }])
        .unwrap();
    let d = &resp.derived;
    assert_eq!(d.deficits.len(), 1, "zero supply is still a deficit");
    let row = &d.deficits[0];
    assert_eq!(row.factory, screw_fid);
    assert_eq!(row.port, screw_in);
    assert_eq!(row.route.as_deref(), Some(route.as_str()));
    assert!(
        row.needed.is_finite(),
        "needed must be finite: {}",
        row.needed
    );
    assert!(
        (row.needed - 50.0).abs() < 1e-4,
        "200 screws need 50 rods: {}",
        row.needed
    );
    assert!(row.supplied.abs() < 1e-6, "supplied 0: {}", row.supplied);
    assert!((d.routes[&route].supplied).abs() < 1e-6);
    // The downstream target is never silently rewritten by an upstream dip.
    assert!((s.state.ports[&screw_out].rate - 200.0).abs() < 1e-4);
}

/// M6: an upstream factory in an error state (ports but no groups) must
/// propagate ZERO supply — downstream clamps to 0 and reports a deficit,
/// instead of silently solving as fully supplied.
#[test]
fn empire_errored_upstream_starves_downstream() {
    let mut s = Session::in_memory(None).unwrap();
    // Downstream first, target set while unconstrained (no route yet).
    let (screw_fid, screw_in, outs, _) = build_screw_factory(&mut s, 1);
    let screw_out = outs[0].clone();
    s.edit(vec![Command::SetPortRate {
        id: screw_out.clone(),
        rate: 200.0,
    }])
    .unwrap();
    // Upstream shell: an Out port but no machine groups → error state.
    let rod_fid = mk_factory(&mut s, "ROD SHELL", 0.0);
    let rod_out = mk_port(&mut s, &rod_fid, PortDirection::Out, "Desc_IronRod_C", None);

    let route = add_rod_route(&mut s, &rod_out, &screw_in);
    let resp = s.edit(vec![Command::RenameFactory {
        id: rod_fid.clone(),
        name: "ROD SHELL (WIP)".into(),
    }]);
    let d = match &resp {
        Ok(r) => &r.derived,
        Err(e) => panic!("recompute failed: {e}"),
    };
    let up = &d.factories[&rod_fid];
    assert!(up.solve_error.is_some(), "upstream is in an error state");
    // Downstream honestly clamps to zero supply — not fully supplied.
    let down = &d.factories[&screw_fid];
    assert!(down.solve_error.is_none());
    assert!(
        down.ports[&screw_out].abs() < 1e-6,
        "achieved screws ~0, got {}",
        down.ports[&screw_out]
    );
    assert!((d.routes[&route].supplied).abs() < 1e-6, "route supplies 0");
    assert_eq!(d.deficits.len(), 1, "starvation surfaces as a deficit row");
    let row = &d.deficits[0];
    assert_eq!(row.factory, screw_fid);
    assert!((row.needed - 50.0).abs() < 1e-4, "needed: {}", row.needed);
    assert!(row.supplied.abs() < 1e-6);
    // The canonical target survives untouched.
    assert!((s.state.ports[&screw_out].rate - 200.0).abs() < 1e-4);
}

/// M8 residue: a multi-Out factory under a supply dip degrades through the
/// shortfall channel into exactly one deficit row — no solve_error, real
/// power, and no group write-back of the starved values.
#[test]
fn empire_multi_out_dip_degrades_into_deficits() {
    let mut s = Session::in_memory(None).unwrap();
    let (_rod_fid, rod_out) = build_rod_factory(&mut s);
    let (screw_fid, screw_in, outs, screws) = build_screw_factory(&mut s, 2);
    let route = add_rod_route(&mut s, &rod_out, &screw_in);

    // Healthy: 30 rods shipped; two screw targets of 60 need 30 rods total.
    s.edit(vec![Command::SetPortRate {
        id: rod_out.clone(),
        rate: 30.0,
    }])
    .unwrap();
    for out in &outs {
        s.edit(vec![Command::SetPortRate {
            id: out.clone(),
            rate: 60.0,
        }])
        .unwrap();
    }
    let count_before = s.state.groups[&screws].count;

    // Upstream dips to 15 while the screw factory still wants 120 total.
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: rod_out.clone(),
            rate: 15.0,
        }])
        .unwrap();
    let d = &resp.derived;
    let df = &d.factories[&screw_fid];
    assert!(
        df.solve_error.is_none(),
        "multi-Out dip must degrade, not dead-end: {:?}",
        df.solve_error
    );
    assert!(df.total_power_mw > 0.0, "degraded solve keeps real power");
    assert!(
        df.shortfalls.values().any(|sf| matches!(
            &sf.binding,
            Some(solver::model::Constraint::InputCeiling { port, .. }) if port == &screw_in
        )),
        "shortfall names the starved In port: {:?}",
        df.shortfalls
    );
    assert_eq!(d.deficits.len(), 1, "exactly one row for the route");
    let row = &d.deficits[0];
    assert_eq!(row.factory, screw_fid);
    assert_eq!(row.port, screw_in);
    assert_eq!(row.route.as_deref(), Some(route.as_str()));
    assert!(
        (row.needed - 30.0).abs() < 1e-4,
        "120 screws need 30 rods: {}",
        row.needed
    );
    assert!((row.supplied - 15.0).abs() < 1e-4);
    // Degraded solves are advisory: counts are not rewritten to starved values.
    assert_eq!(s.state.groups[&screws].count, count_before);
    // Targets survive untouched.
    for out in &outs {
        assert!((s.state.ports[out].rate - 60.0).abs() < 1e-4);
    }
}

/// Dedup: when the clamped channel (SetTarget on one Out) and the shortfall
/// channel (the sibling Out) both name the same starved In port, the route
/// still gets exactly ONE deficit row.
#[test]
fn empire_dedup_one_row_per_route_when_both_channels_fire() {
    let mut s = Session::in_memory(None).unwrap();
    let (_rod_fid, rod_out) = build_rod_factory(&mut s);
    let (screw_fid, screw_in, outs, _) = build_screw_factory(&mut s, 2);
    let _route = add_rod_route(&mut s, &rod_out, &screw_in);

    s.edit(vec![Command::SetPortRate {
        id: rod_out.clone(),
        rate: 30.0,
    }])
    .unwrap();
    for out in &outs {
        s.edit(vec![Command::SetPortRate {
            id: out.clone(),
            rate: 60.0,
        }])
        .unwrap();
    }
    // Deep dip (10 rods → 40 screws max), then re-assert the SECOND Out
    // target during the dip: the edited port clamps at a target ceiling
    // binding the In port, while the sibling (still 60) shortfalls on the
    // same In port — both emitter channels are armed simultaneously.
    s.edit(vec![Command::SetPortRate {
        id: rod_out.clone(),
        rate: 10.0,
    }])
    .unwrap();
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: outs[1].clone(),
            rate: 60.0,
        }])
        .unwrap();
    let d = &resp.derived;
    let df = &d.factories[&screw_fid];
    assert!(
        matches!(
            df.target_ceiling.as_ref().map(|c| &c.binding),
            Some(solver::model::Constraint::InputCeiling { port, .. }) if port == &screw_in
        ),
        "clamped channel armed: {:?}",
        df.target_ceiling
    );
    assert!(
        df.shortfalls.values().any(|sf| matches!(
            &sf.binding,
            Some(solver::model::Constraint::InputCeiling { port, .. }) if port == &screw_in
        )),
        "shortfall channel armed: {:?}",
        df.shortfalls
    );
    assert_eq!(
        d.deficits.len(),
        1,
        "one decision per route — never two rows: {:?}",
        d.deficits
    );
    assert_eq!(d.deficits[0].factory, screw_fid);
    assert_eq!(d.deficits[0].port, screw_in);
}

#[test]
fn generator_factories_and_circuits() {
    let mut s = Session::in_memory(None).unwrap();

    // A coal power plant: coal in → generators → MW out. Power is production.
    let plant = s
        .edit(vec![Command::CreateFactory {
            name: "COASTAL COAL".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
        .clone();
    let coal_in = s
        .edit(vec![Command::AddPort {
            factory: plant.clone(),
            direction: PortDirection::In,
            item: "Desc_Coal_C".into(),
            rate: 0.0,
            rate_ceiling: Some(120.0),
            graph_pos: GraphPos { x: 0.0, y: 100.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let mw_out = s
        .edit(vec![Command::AddPort {
            factory: plant.clone(),
            direction: PortDirection::Out,
            item: gamedata::docs::POWER_ITEM.into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: GraphPos { x: 600.0, y: 100.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let burn_recipe = s
        .gamedata
        .recipes
        .values()
        .find(|r| r.produced_in.contains(&"Build_GeneratorCoal_C".to_string()))
        .unwrap()
        .class_name
        .clone();
    let gens = add_group(
        &mut s,
        &plant,
        "Build_GeneratorCoal_C",
        &burn_recipe,
        gp(300.0, 100.0),
    );
    connect_in(
        &mut s,
        &plant,
        EdgeEnd::Port(coal_in),
        EdgeEnd::Group(gens.clone()),
        "Desc_Coal_C",
        3,
    );
    connect_in(
        &mut s,
        &plant,
        EdgeEnd::Group(gens.clone()),
        EdgeEnd::Port(mw_out.clone()),
        gamedata::docs::POWER_ITEM,
        6,
    );

    // Target 300 MW — the MW slider back-solves the fuel chain like items.
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: mw_out.clone(),
            rate: 300.0,
        }])
        .unwrap();
    let df = &resp.derived.factories[&plant];
    assert!((df.ports[&mw_out] - 300.0).abs() < 1e-6);
    assert_eq!(s.state.groups[&gens].count, 4, "4 generators at 75 MW");
    // 300 MW burns 60 coal/min (15/gen)
    let coal_used: f64 = df.groups[&gens].in_rates["Desc_Coal_C"];
    assert!((coal_used - 60.0).abs() < 1e-4, "coal burn: {coal_used}");

    // A consumer factory joined by a power line forms a grid with a margin.
    let consumer = s
        .edit(vec![Command::CreateFactory {
            name: "IRON WORKS".into(),
            position: MapPos {
                x: 800.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
        .clone();
    let ore_in = s
        .edit(vec![Command::AddPort {
            factory: consumer.clone(),
            direction: PortDirection::In,
            item: "Desc_OreIron_C".into(),
            rate: 0.0,
            rate_ceiling: Some(120.0),
            graph_pos: GraphPos { x: 0.0, y: 100.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let ingot_out = s
        .edit(vec![Command::AddPort {
            factory: consumer.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronIngot_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: GraphPos { x: 400.0, y: 100.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let smelt = add_group(
        &mut s,
        &consumer,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        gp(100.0, 100.0),
    );
    connect_in(
        &mut s,
        &consumer,
        EdgeEnd::Port(ore_in),
        EdgeEnd::Group(smelt.clone()),
        "Desc_OreIron_C",
        3,
    );
    connect_in(
        &mut s,
        &consumer,
        EdgeEnd::Group(smelt),
        EdgeEnd::Port(ingot_out.clone()),
        "Desc_IronIngot_C",
        3,
    );
    s.edit(vec![Command::SetPortRate {
        id: ingot_out,
        rate: 30.0,
    }])
    .unwrap();
    let resp = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Power,
            from: plant.clone(),
            to: consumer.clone(),
            path: vec![
                MapPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                MapPos {
                    x: 800.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        }])
        .unwrap();
    let d = &resp.derived;
    assert_eq!(d.circuits.len(), 1, "one grid");
    let grid = &d.circuits[0];
    assert_eq!(grid.members.len(), 2);
    assert!(
        (grid.generation_mw - 300.0).abs() < 1e-4,
        "gen {}",
        grid.generation_mw
    );
    assert!(grid.demand_mw > 0.0, "smelter draws power");
    assert!((d.total_generation_mw - 300.0).abs() < 1e-4);
}

#[test]
fn imported_generator_counts_nameplate_without_fuel() {
    // Power honesty (#58): an imported ◆ coal plant has generators but no traced
    // coal supply, so the fuel-burn recipe solves to 0 POWER_ITEM. Generation
    // must still reflect nameplate × count × clock, else the empire power balance
    // reads a false "NO GEN" for every imported world.
    let mut s = Session::in_memory(None).unwrap();
    // Real generators carry NO recipe in the save (nothing to read), exactly the
    // case that made the solver report 0 generation for imports.
    let gen_mach = |x: f64| app::import::ImportMachine {
        class: "Build_GeneratorCoal_C".into(),
        recipe: None,
        clock: 1.0,
        x,
        y: 0.0,
        z: 0.0,
        ..Default::default()
    };
    // Import 4 coal generators as ◆ BUILT — clustered into one group of 4, with
    // NO coal supply anywhere (the fuel chain is untraced, as in a real import).
    s.import_save(app::import::ImportSnapshot {
        save_name: "IMPORT-GEN".into(),
        machines: vec![
            gen_mach(0.0),
            gen_mach(40.0),
            gen_mach(80.0),
            gen_mach(120.0),
        ],
        ..Default::default()
    })
    .unwrap();
    let gens = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_GeneratorCoal_C")
        .expect("imported generator group");
    assert_eq!(gens.status, Status::Built, "imported as ◆ built");
    assert_eq!(gens.recipe, "", "no recipe was read from the save");
    assert_eq!(gens.count, 4, "4 generators clustered into one group");
    let (gen_factory, gen_id) = (gens.factory.clone(), gens.id.clone());
    // The empire power balance (the summary's source) must reflect nameplate
    // generation — 4 × 75 MW = 300 MW — not the solver's fuel-starved 0.
    let d = s.solve_all_readonly();
    assert!(
        (d.total_generation_mw - 300.0).abs() < 1e-4,
        "empire generation honest without a fuel recipe: {}",
        d.total_generation_mw
    );
    // ...and the PER-GROUP derived output must carry that same nameplate as its
    // POWER_ITEM out-rate, so the factory-graph generator card agrees with the
    // empire instead of reading a false 0 MW. The material solve skips
    // recipe-less generators; the derive re-injects their nameplate here.
    let dg = d
        .factories
        .get(&gen_factory)
        .and_then(|df| df.groups.get(&gen_id))
        .expect("recipe-less generator gets a derived group with its nameplate");
    assert!(
        (dg.out_rates
            .get(gamedata::docs::POWER_ITEM)
            .copied()
            .unwrap_or(0.0)
            - 300.0)
            .abs()
            < 1e-4,
        "generator card reads nameplate MW, not 0: {:?}",
        dg.out_rates
    );
    assert_eq!(dg.power_mw, 0.0, "a generator draws no power");
}

#[test]
fn water_pump_group_produces_water_from_nothing() {
    // A placeable water extractor has no world node (water is drawn from any
    // surface), so it runs its synthesized zero-ingredient extraction recipe.
    // The solver must produce its water with no input, feeding downstream like
    // any source — the whole point of making the pump placeable.
    let mut s = Session::in_memory(None).unwrap();
    let r = s
        .edit(vec![Command::CreateFactory {
            name: "WATER WORKS".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap();
    let fid = r.created[0].clone();

    let pump = add_group(
        &mut s,
        &fid,
        "Build_WaterPump_C",
        "Recipe_Extract_Build_WaterPump",
        gp(200.0, 0.0),
    );
    let r = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::Out,
            item: "Desc_Water_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: gp(600.0, 0.0),
        }])
        .unwrap();
    let out_port = r.created[0].clone();
    connect(
        &mut s,
        &fid,
        EdgeEnd::Group(pump.clone()),
        EdgeEnd::Port(out_port.clone()),
        "Desc_Water_C",
        5,
    );
    s.edit(vec![Command::SetPortRate {
        id: out_port.clone(),
        rate: 100.0,
    }])
    .unwrap();

    let d = s.solve_all_readonly();
    let df = d.factories.get(&fid).expect("factory derived");
    assert!(
        df.solve_error.is_none(),
        "zero-ingredient extraction solves: {:?}",
        df.solve_error
    );
    let dg = df.groups.get(&pump).expect("water pump derived group");
    // Demand-driven to 100/min, within the single pump's 120/min capacity.
    assert!(
        (dg.out_rates.get("Desc_Water_C").copied().unwrap_or(0.0) - 100.0).abs() < 1e-4,
        "pump produces the demanded water with no input: {:?}",
        dg.out_rates
    );
    assert!(dg.in_rates.is_empty(), "extraction consumes nothing");
}

#[test]
fn sync_meta_round_trips_and_survives_new_empire() {
    // Desktop save-sync remembers the picked save (path/name/time). It's a
    // device choice, not plan data, so it must SURVIVE a new empire — otherwise
    // wiping the plan silently forgets the auto-sync target.
    let mut s = Session::in_memory(None).unwrap();
    assert_eq!(s.sync_meta(), None, "no remembered save initially");
    s.set_sync_meta(r#"{"path":"/saves/factory.sav","name":"factory.sav","lastSyncedAt":42}"#)
        .unwrap();
    assert!(s.sync_meta().unwrap().contains("factory.sav"));

    s.new_empire().unwrap();
    assert!(
        s.sync_meta().unwrap().contains("factory.sav"),
        "the remembered save survives new_empire"
    );
}

#[test]
fn starved_fueled_plant_reports_solved_generation_not_nameplate() {
    // The other side of #58's honesty: a plant whose fuel recipe DOES solve
    // must report the solved (starved) output, not nameplate — the empire
    // total has to agree with the per-grid sums, or the power summary claims
    // MW the grid never sees. A ◆ imported plant pins the case: the solver
    // never rewrites its built count, so nameplate stays 4 × 75 MW while the
    // fuel ceiling caps the solve.
    let mut s = Session::in_memory(None).unwrap();
    let burn_recipe = s
        .gamedata
        .recipes
        .values()
        .find(|r| r.produced_in.contains(&"Build_GeneratorCoal_C".to_string()))
        .unwrap()
        .class_name
        .clone();
    let gen_mach = |x: f64| app::import::ImportMachine {
        class: "Build_GeneratorCoal_C".into(),
        recipe: Some(burn_recipe.clone()),
        clock: 1.0,
        x,
        y: 0.0,
        z: 0.0,
        ..Default::default()
    };
    s.import_save(app::import::ImportSnapshot {
        save_name: "IMPORT-FUELED".into(),
        machines: vec![
            gen_mach(0.0),
            gen_mach(40.0),
            gen_mach(80.0),
            gen_mach(120.0),
        ],
        ..Default::default()
    })
    .unwrap();
    let plant = s.state.factories.keys().next().unwrap().clone();
    let gens = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_GeneratorCoal_C")
        .expect("imported generator group")
        .id
        .clone();
    // A consumer on a power line forms the grid whose sum must agree.
    let consumer = s
        .edit(vec![Command::CreateFactory {
            name: "IRON WORKS".into(),
            position: MapPos {
                x: 800.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
        .clone();
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Power,
        from: plant,
        to: consumer,
        path: vec![
            MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            MapPos {
                x: 800.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    }])
    .unwrap();

    // Coal supply collapses to half: 30/min feeds 2 of the 4 generators.
    let coal_in = s
        .state
        .ports
        .values()
        .find(|p| p.item == "Desc_Coal_C")
        .unwrap()
        .id
        .clone();
    let resp = s
        .edit(vec![Command::SetPortCeiling {
            id: coal_in,
            rate_ceiling: Some(30.0),
        }])
        .unwrap();
    let d = &resp.derived;
    let g = &s.state.groups[&gens];
    assert_eq!(g.count, 4, "◆ built count is not the solver's to resize");
    let nameplate = 75.0 * g.effective_count() as f64 * g.effective_clock();
    assert!(
        d.total_generation_mw < nameplate - 1e-6,
        "starved plant reads below nameplate {nameplate}: {}",
        d.total_generation_mw
    );
    assert!(
        (d.total_generation_mw - 150.0).abs() < 1e-4,
        "30 coal/min runs 150 MW worth of generators: {}",
        d.total_generation_mw
    );
    assert_eq!(d.circuits.len(), 1);
    assert!(
        (d.total_generation_mw - d.circuits[0].generation_mw).abs() < 1e-6,
        "empire total agrees with the grid sum: {} vs {}",
        d.total_generation_mw,
        d.circuits[0].generation_mw
    );
}

#[test]
fn imported_generator_nameplate_reads_planned_delta() {
    // The nameplate arm plans with the same effective values the solvers use:
    // a ◆ imported bank with a user-planned expansion/retune reads
    // mw × effective_count × effective_clock, not the built baseline.
    let mut s = Session::in_memory(None).unwrap();
    let gen_mach = |x: f64| app::import::ImportMachine {
        class: "Build_GeneratorCoal_C".into(),
        recipe: None,
        clock: 1.0,
        x,
        y: 0.0,
        z: 0.0,
        ..Default::default()
    };
    s.import_save(app::import::ImportSnapshot {
        save_name: "IMPORT-GEN".into(),
        machines: vec![
            gen_mach(0.0),
            gen_mach(40.0),
            gen_mach(80.0),
            gen_mach(120.0),
        ],
        ..Default::default()
    })
    .unwrap();
    let gid = s
        .state
        .groups
        .values()
        .find(|g| g.machine == "Build_GeneratorCoal_C")
        .expect("imported generator group")
        .id
        .clone();
    s.edit(vec![
        Command::SetGroupCount {
            id: gid.clone(),
            count: 6,
        },
        Command::SetGroupClock {
            id: gid.clone(),
            clock: 0.5,
        },
    ])
    .unwrap();
    let g = &s.state.groups[&gid];
    assert_eq!(
        g.count, 4,
        "◆ baseline untouched — the edit rides the delta"
    );
    let d = s.solve_all_readonly();
    assert!(
        (d.total_generation_mw - 225.0).abs() < 1e-4,
        "75 × 6 × 0.5 = 225 from the planned delta: {}",
        d.total_generation_mw
    );
}

#[cfg(feature = "sqlite")]
#[test]
fn floor_assignment_is_undoable_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.ficsit");
    let gid;
    {
        let mut s = Session::open(&path, None, "fixture").unwrap();
        let (fid, _, _) = build_modular_frame_factory(&mut s);
        gid = s.state.factories[&fid].groups[0].clone();
        s.edit(vec![Command::SetGroupFloor {
            id: gid.clone(),
            floor: 2,
        }])
        .unwrap();
        assert_eq!(s.state.groups[&gid].floor, 2);
        s.undo().unwrap().unwrap();
        assert_eq!(s.state.groups[&gid].floor, 0);
        s.redo().unwrap().unwrap();
    }
    let s = Session::open(&path, None, "fixture").unwrap();
    assert_eq!(s.state.groups[&gid].floor, 2, "floor survives reopen");
}

/// PR 3: NEXT preferences persist through the endpoint, survive reopen, and ride
/// hydrate (`plan.meta.preferences`) — but are NOT undoable and stay OUT of
/// plan_hash (a filter toggle must not staleness-flag proposals or trip merge).
#[cfg(feature = "sqlite")]
#[test]
fn next_preferences_persist_and_ride_hydrate_without_touching_plan_hash() {
    use planner_core::state::NextPreferences;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.ficsit");
    {
        let mut s = Session::open(&path, None, "fixture").unwrap();
        let hash_before = s.plan_hash();
        let can_undo_before = s.undo.can_undo();
        let view = s
            .set_next_preferences(NextPreferences {
                no_trains: true,
                ignore_power: false,
            })
            .unwrap();
        assert!(view.preferences.no_trains && !view.preferences.ignore_power);
        // A preference toggle is not plan geometry: hash is stable, no undo entry.
        assert_eq!(s.plan_hash(), hash_before, "prefs stay out of plan_hash");
        assert_eq!(
            s.undo.can_undo(),
            can_undo_before,
            "prefs must not be undoable"
        );
        // hydrate carries them to the renderer.
        let hydrate = s.hydrate();
        assert_eq!(hydrate["plan"]["meta"]["preferences"]["noTrains"], true);
        assert_eq!(hydrate["plan"]["meta"]["preferences"]["ignorePower"], false);
    }
    // Reopen: preferences survive (persisted with the plan meta row).
    let s = Session::open(&path, None, "fixture").unwrap();
    assert!(
        s.state.meta.preferences.no_trains && !s.state.meta.preferences.ignore_power,
        "preferences survive reopen"
    );
}

/// PR 3: an old plan file (no `preferences` in its meta blob) loads unchanged —
/// serde-default fills the struct, no migration.
#[test]
fn plan_without_preferences_key_loads_with_defaults() {
    use planner_core::state::PlanMeta;
    // A meta blob shaped like a pre-PR-3 file — no `preferences` key at all.
    let legacy = serde_json::json!({
        "schemaVersion": 1,
        "gameBuild": "fixture",
        "name": "OLD WORLD"
    });
    let meta: PlanMeta = serde_json::from_value(legacy).unwrap();
    assert_eq!(meta.name, "OLD WORLD");
    assert!(!meta.preferences.no_trains && !meta.preferences.ignore_power);
}

#[test]
fn failed_multi_command_edit_rolls_back() {
    let mut s = Session::in_memory(None).unwrap();
    let r = s
        .edit(vec![Command::CreateFactory {
            name: "X".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
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

#[test]
fn elevation_flows_into_route_length_and_climb() {
    let mut s = Session::in_memory(None).unwrap();

    // Two pins 300/400 apart on the map, 120m apart vertically → 3-4-(5·1.3)
    let a = s
        .edit(vec![Command::CreateFactory {
            name: "LOWLANDS".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 30.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
        .clone();
    let b = s
        .edit(vec![Command::CreateFactory {
            name: "PLATEAU".into(),
            position: MapPos {
                x: 300.0,
                y: 0.0,
                z: 430.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
        .clone();
    let out = s
        .edit(vec![Command::AddPort {
            factory: a.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronIngot_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: GraphPos { x: 0.0, y: 0.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let inp = s
        .edit(vec![Command::AddPort {
            factory: b.clone(),
            direction: PortDirection::In,
            item: "Desc_IronIngot_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: GraphPos { x: 0.0, y: 0.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let resp = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Belt { tier: 3 },
            from: out,
            to: inp,
            path: vec![
                s.state.factories[&a].position,
                s.state.factories[&b].position,
            ],
        }])
        .unwrap();
    let route = resp.created[0].clone();
    let dr = &resp.derived.routes[&route];
    // 2D distance 300, Δz 400 → 3D length 500; one 400m climb, no descent.
    assert!(
        (dr.length_m - 500.0).abs() < 1e-6,
        "3D length: {}",
        dr.length_m
    );
    assert!((dr.climb_up_m - 400.0).abs() < 1e-6);
    assert!(dr.climb_down_m.abs() < 1e-6);

    // Re-siting the pin (elevation edit = move with new z) refreshes the
    // route's endpoint waypoint — length/climb re-derive from the new site.
    let resp = s
        .edit(vec![Command::MoveFactoryPin {
            id: b.clone(),
            position: MapPos {
                x: 300.0,
                y: 0.0,
                z: 30.0,
            },
        }])
        .unwrap();
    let dr = &resp.derived.routes[&route];
    assert!(
        (dr.length_m - 300.0).abs() < 1e-6,
        "flattened: {}",
        dr.length_m
    );
    assert!(dr.climb_up_m.abs() < 1e-6);

    // Undo restores both the pin and the route waypoint (one entry).
    let resp = s.undo().unwrap().unwrap();
    let dr = &resp.derived.routes[&route];
    assert!((dr.length_m - 500.0).abs() < 1e-6, "undo: {}", dr.length_m);
}

#[test]
fn priority_switches_derive_shed_thresholds() {
    let mut s = Session::in_memory(None).unwrap();
    // three factories on one grid: 150 MW plant + two consumers
    let mk = |s: &mut Session, name: &str, x: f64| -> Id {
        s.edit(vec![Command::CreateFactory {
            name: name.into(),
            position: MapPos { x, y: 0.0, z: 0.0 },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
            .clone()
    };
    let plant = mk(&mut s, "PLANT", 0.0);
    let a = mk(&mut s, "LOAD A", 500.0);
    let b = mk(&mut s, "LOAD B", 1000.0);
    // give the loads real draw: constructor banks idling at a target
    for fid in [&a, &b] {
        let in_p = s
            .edit(vec![Command::AddPort {
                factory: fid.clone(),
                direction: PortDirection::In,
                item: "Desc_IronIngot_C".into(),
                rate: 0.0,
                rate_ceiling: Some(120.0),
                graph_pos: gp(0.0, 100.0),
            }])
            .unwrap()
            .created[0]
            .clone();
        let out_p = s
            .edit(vec![Command::AddPort {
                factory: fid.clone(),
                direction: PortDirection::Out,
                item: "Desc_IronRod_C".into(),
                rate: 0.0,
                rate_ceiling: None,
                graph_pos: gp(600.0, 100.0),
            }])
            .unwrap()
            .created[0]
            .clone();
        let g = add_group(
            &mut s,
            fid,
            "Build_ConstructorMk1_C",
            "Recipe_IronRod_C",
            gp(300.0, 100.0),
        );
        connect(
            &mut s,
            fid,
            EdgeEnd::Port(in_p),
            EdgeEnd::Group(g.clone()),
            "Desc_IronIngot_C",
            2,
        );
        connect(
            &mut s,
            fid,
            EdgeEnd::Group(g),
            EdgeEnd::Port(out_p.clone()),
            "Desc_IronRod_C",
            2,
        );
        s.edit(vec![Command::SetPortRate {
            id: out_p,
            rate: 15.0,
        }])
        .unwrap();
    }
    // plant → A → B power lines
    let line_a = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Power,
            from: plant.clone(),
            to: a.clone(),
            path: vec![
                MapPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                MapPos {
                    x: 500.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        }])
        .unwrap()
        .created[0]
        .clone();
    let line_b = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Power,
            from: a.clone(),
            to: b.clone(),
            path: vec![
                MapPos {
                    x: 500.0,
                    y: 0.0,
                    z: 0.0,
                },
                MapPos {
                    x: 1000.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        }])
        .unwrap()
        .created[0]
        .clone();

    // switches: P3 on the A line, P8 on the far B line (sheds first)
    let sw_a = s
        .edit(vec![Command::AddPrioritySwitch {
            route: line_a.clone(),
            priority: 3,
        }])
        .unwrap()
        .created[0]
        .clone();
    let resp = s
        .edit(vec![Command::AddPrioritySwitch {
            route: line_b,
            priority: 8,
        }])
        .unwrap();

    let grid = &resp.derived.circuits[0];
    assert_eq!(grid.members.len(), 3);
    assert_eq!(grid.switches.len(), 2, "both switches derive");
    // shed order: P8 first at demand == generation, P3 after shedding B's load
    assert_eq!(grid.switches[0].priority, 8);
    assert!((grid.switches[0].sheds_at_mw - grid.generation_mw).abs() < 1e-6);
    assert_eq!(grid.switches[1].priority, 3);
    assert!(
        (grid.switches[1].sheds_at_mw - (grid.generation_mw + grid.switches[0].downstream_mw))
            .abs()
            < 1e-6
    );
    // the P8 switch's load side is LOAD B only; P3 cuts A+B off the plant
    assert!(grid.switches[0].downstream_mw > 0.0);
    assert!(grid.switches[1].downstream_mw >= grid.switches[0].downstream_mw);
    assert!(grid.next_shed.as_deref().unwrap_or("").starts_with("P8"));

    // priority is editable and validated; deleting the line removes the switch
    assert!(s
        .edit(vec![Command::SetSwitchPriority {
            id: sw_a.clone(),
            priority: 9
        }])
        .is_err());
    s.edit(vec![Command::SetSwitchPriority {
        id: sw_a.clone(),
        priority: 1,
    }])
    .unwrap();
    let resp = s.edit(vec![Command::DeleteRoute { id: line_a }]).unwrap();
    assert!(!s.state.switches.contains_key(&sw_a), "cascade delete");
    // plant's line gone: it leaves the grid; A—B remain one circuit
    assert_eq!(resp.derived.circuits.len(), 1);
    assert_eq!(resp.derived.circuits[0].members.len(), 2);
}

#[test]
fn rail_routes_compute_throughput_and_respec() {
    let mut s = Session::in_memory(None).unwrap();
    let mk = |s: &mut Session, name: &str, x: f64| -> Id {
        s.edit(vec![Command::CreateFactory {
            name: name.into(),
            position: MapPos { x, y: 0.0, z: 0.0 },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
            .clone()
    };
    let a = mk(&mut s, "STEEL COAST", 0.0);
    let b = mk(&mut s, "MOTOR WORKS", 3000.0);
    let out = s
        .edit(vec![Command::AddPort {
            factory: a.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronIngot_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: gp(600.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let inp = s
        .edit(vec![Command::AddPort {
            factory: b.clone(),
            direction: PortDirection::In,
            item: "Desc_IronIngot_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: gp(0.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();

    // a 3km haul: rail route with the default consist
    let resp = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Rail {
                spec: RailSpec::default(),
            },
            from: out.clone(),
            to: inp,
            path: vec![
                MapPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                MapPos {
                    x: 3000.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        }])
        .unwrap();
    let route = resp.created[0].clone();
    assert_eq!(
        s.state.ports[&out].bound_route.as_deref(),
        Some(route.as_str()),
        "rail binds ports like belts"
    );
    let dr = &resp.derived.routes[&route];
    let t = dr.transport.as_ref().expect("math block");
    // 2 × 3km × 1.12 at 90 km/h + 50s dwell + 15% headway
    let travel = 2.0 * 3000.0 * 1.12 / (90.0 / 3.6);
    assert!(
        (t.round_trip_s - travel).abs() < 1.0,
        "travel {}",
        t.round_trip_s
    );
    assert!((t.load_unload_s - 50.0).abs() < 1e-9);
    assert!(
        (dr.capacity - t.throughput_per_min).abs() < 1e-9,
        "capacity IS the math"
    );
    assert!(
        t.throughput_per_min > 1000.0,
        "ingot rail moves serious volume"
    );

    // respec: +1 consist doubles throughput; belt→rail swap forbidden on power
    let spec = RailSpec {
        consists: 2,
        ..Default::default()
    };
    let resp = s
        .edit(vec![Command::SetRouteSpec {
            id: route.clone(),
            kind: RouteKind::Rail { spec },
        }])
        .unwrap();
    let t2 = resp.derived.routes[&route]
        .transport
        .as_ref()
        .unwrap()
        .clone();
    assert!((t2.throughput_per_min - 2.0 * t.throughput_per_min).abs() < 1e-6);

    // downgrade to a drone: same binding, new math (batteries line appears)
    let resp = s
        .edit(vec![Command::SetRouteSpec {
            id: route.clone(),
            kind: RouteKind::Drone {
                spec: DroneSpec::default(),
            },
        }])
        .unwrap();
    let t3 = resp.derived.routes[&route]
        .transport
        .as_ref()
        .unwrap()
        .clone();
    assert!(t3.batteries_per_min.is_some());
    // one undo step reverts the respec
    s.undo().unwrap().unwrap();
    assert!(matches!(
        s.state.routes[&route].kind,
        RouteKind::Rail { .. }
    ));
}

// ---- variable-power draw (Particle Accelerator etc.) ----

/// Variable-power recipes carry an average sustained draw override; the
/// session snapshot and the solve must plan with it, not the ~0 the machine
/// would otherwise report from real Docs.json.
#[test]
fn variable_power_recipe_average_drives_group_power() {
    let mut s = Session::in_memory(None).unwrap();
    let r = s
        .edit(vec![Command::CreateFactory {
            name: "QUANTUM WORKS".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap();
    let fid = r.created[0].clone();

    let r = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::In,
            item: "Desc_Coal_C".into(),
            rate: 0.0,
            rate_ceiling: Some(600.0),
            graph_pos: gp(0.0, 200.0),
        }])
        .unwrap();
    let in_port = r.created[0].clone();
    let r = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::Out,
            item: "Desc_Diamond_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: gp(800.0, 200.0),
        }])
        .unwrap();
    let out_port = r.created[0].clone();

    let pa = add_group(
        &mut s,
        &fid,
        "Build_HadronCollider_C",
        "Recipe_Diamond_C",
        gp(400.0, 200.0),
    );
    connect(
        &mut s,
        &fid,
        EdgeEnd::Port(in_port),
        EdgeEnd::Group(pa.clone()),
        "Desc_Coal_C",
        5,
    );
    connect(
        &mut s,
        &fid,
        EdgeEnd::Group(pa.clone()),
        EdgeEnd::Port(out_port.clone()),
        "Desc_Diamond_C",
        5,
    );

    // The snapshot's RecipeSpec carries the recipe average (constant +
    // factor/2 = 500 MW), not the machine estimate path.
    let snap = s.snapshot(&fid).unwrap();
    let gs = snap.groups.iter().find(|g| g.id == pa).unwrap();
    assert_eq!(gs.recipe.power_mw, 500.0);

    // Solve at exactly one machine's worth of demand: 1 diamond / 2 s =
    // 30/min per machine, so a 30/min target lands count 1 @ 100% clock and
    // group power = 500 × 1 × 1.0^1.321928 = 500 MW.
    let r = s
        .edit(vec![Command::SetPortRate {
            id: out_port,
            rate: 30.0,
        }])
        .unwrap();
    let df = &r.derived.factories[&fid];
    let gp_mw = df.groups[&pa].power_mw;
    assert!(
        (gp_mw - 500.0).abs() < 1e-6,
        "variable-power draw reaches group power: {gp_mw}"
    );
    assert!(
        (df.total_power_mw - 500.0).abs() < 1e-6,
        "and the factory total: {}",
        df.total_power_mw
    );
}

// ---- M9: persist failure can never diverge memory from disk ----
// The fault seam (`s.store.faults_mut()`, persist's `fault-injection` feature) fails
// the next N commits/checkpoints before their SQLite transaction opens —
// observationally identical to a rolled-back mid-write failure.

fn create_named_factory(s: &mut Session, name: &str) -> Id {
    s.edit(vec![Command::CreateFactory {
        name: name.into(),
        position: MapPos {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        region: "GRASS FIELDS".into(),
    }])
    .unwrap()
    .created[0]
        .clone()
}

/// Disk and memory agree entity-for-entity right now.
fn assert_disk_matches_memory(s: &Session) {
    let (disk, entries, cursor) = s.store.load().unwrap();
    assert_eq!(disk.project(), s.state.project(), "disk == memory");
    assert_eq!(cursor, s.undo.entries().len(), "cursor == applied depth");
    assert!(entries.len() >= cursor);
}

#[cfg(feature = "sqlite")]
#[test]
fn edit_persist_failure_leaves_no_trace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.ficsit");
    let fid;
    {
        let mut s = Session::open(&path, None, "fixture").unwrap();
        fid = create_named_factory(&mut s, "NORTHERN FORGE");
        let hash_before = s.plan_hash();
        let depth_before = s.undo.entries().len();
        let label_before = s.undo.undo_label().map(String::from);

        // E1 hits a transient persist failure: the error surfaces and the
        // edit leaves no trace anywhere.
        s.store.faults_mut().fail_commits = 1;
        let err = s.edit(vec![Command::RenameFactory {
            id: fid.clone(),
            name: "GHOST".into(),
        }]);
        assert!(err.is_err(), "persist failure must surface");
        assert_eq!(s.state.factories[&fid].name, "NORTHERN FORGE");
        assert_eq!(s.plan_hash(), hash_before, "plan hash unchanged");
        assert_eq!(s.undo.entries().len(), depth_before, "undo depth unchanged");
        assert_eq!(s.undo.undo_label().map(String::from), label_before);
        assert_disk_matches_memory(&s);

        // E2 (the fault was one-shot) succeeds on top of the clean state.
        s.edit(vec![Command::RenameFactory {
            id: fid.clone(),
            name: "IRON WORKS".into(),
        }])
        .unwrap();
        assert_eq!(s.state.factories[&fid].name, "IRON WORKS");
        assert_disk_matches_memory(&s);
    }
    // Reopen: E2 present, E1 absent, and undo applies cleanly — the M9
    // silent-loss + journal-skew scenario is impossible.
    let mut s = Session::open(&path, None, "fixture").unwrap();
    assert_eq!(s.state.factories[&fid].name, "IRON WORKS");
    assert_eq!(s.undo.entries().len(), 2, "create + E2, no ghost entry");
    let r = s.undo().unwrap().unwrap();
    assert!(!r.patches.is_empty());
    assert_eq!(s.state.factories[&fid].name, "NORTHERN FORGE");
}

#[test]
fn edit_persist_failure_preserves_redo_tail() {
    let mut s = Session::in_memory(None).unwrap();
    let fid = create_named_factory(&mut s, "BASE");
    s.edit(vec![Command::RenameFactory {
        id: fid.clone(),
        name: "SECOND".into(),
    }])
    .unwrap();
    s.undo().unwrap().unwrap();
    assert!(s.undo.can_redo(), "redo tail exists");

    // A failed edit must not truncate the redo tail (commit-then-rollback
    // would have destroyed it before the persist error).
    s.store.faults_mut().fail_commits = 1;
    assert!(s
        .edit(vec![Command::RenameFactory {
            id: fid.clone(),
            name: "THIRD".into(),
        }])
        .is_err());
    assert!(s.undo.can_redo(), "redo tail survives in memory");
    let (_, entries, cursor) = s.store.load().unwrap();
    assert_eq!(entries.len(), 2, "redo tail survives on disk");
    assert_eq!(cursor, 1);
    let r = s.redo().unwrap().unwrap();
    assert!(!r.patches.is_empty());
    assert_eq!(s.state.factories[&fid].name, "SECOND");
}

#[test]
fn undo_redo_persist_failure_restores_position() {
    let mut s = Session::in_memory(None).unwrap();
    let fid = create_named_factory(&mut s, "FIRST");
    s.edit(vec![Command::RenameFactory {
        id: fid.clone(),
        name: "SECOND".into(),
    }])
    .unwrap();

    // undo: checkpoint fails → the just-undone entry is re-applied.
    s.store.faults_mut().fail_checkpoints = 1;
    assert!(s.undo().is_err(), "failed checkpoint must surface");
    assert_eq!(s.state.factories[&fid].name, "SECOND", "position restored");
    assert!(!s.undo.can_redo(), "cursor restored");
    assert_eq!(s.undo.entries().len(), 2);
    assert_disk_matches_memory(&s);

    // The fault was one-shot: the same undo now succeeds.
    s.undo().unwrap().unwrap();
    assert_eq!(s.state.factories[&fid].name, "FIRST");
    assert_disk_matches_memory(&s);

    // redo mirror: checkpoint fails → the just-redone entry is un-applied.
    s.store.faults_mut().fail_checkpoints = 1;
    assert!(s.redo().is_err());
    assert_eq!(s.state.factories[&fid].name, "FIRST", "position restored");
    assert!(s.undo.can_redo());
    assert_disk_matches_memory(&s);

    s.redo().unwrap().unwrap();
    assert_eq!(s.state.factories[&fid].name, "SECOND");
    assert_disk_matches_memory(&s);
}

#[test]
fn corrupt_journal_undo_fails_cleanly_and_session_self_heals() {
    use planner_core::patch::PatchOp;
    use planner_core::undo::{UndoEntry, UndoLog};
    let mut s = Session::in_memory(None).unwrap();
    let fid = create_named_factory(&mut s, "FIRST");

    // Simulate a damaged persisted journal: an in-memory log whose top
    // entry's inverse can't apply. Before the fallible-undo fix this was a
    // panic (poisoned mutex); now it surfaces as an error and the session
    // self-heals from disk.
    let corrupt = UndoEntry {
        label: "corrupt".into(),
        forward: vec![],
        inverse: vec![PatchOp::Add {
            path: "/wizzles/x".into(),
            value: serde_json::json!({}),
        }],
    };
    s.undo = UndoLog::hydrate(vec![corrupt]);
    assert!(s.undo().is_err(), "corrupt journal must surface, not panic");

    // Rehydrated from the plan file: state matches disk and the real journal
    // is back, so the same ⌘Z now undoes cleanly.
    assert_disk_matches_memory(&s);
    assert_eq!(s.state.factories[&fid].name, "FIRST");
    s.undo().unwrap().unwrap();
    assert!(!s.state.factories.contains_key(&fid));
    assert_disk_matches_memory(&s);
}

#[test]
fn accept_proposal_persist_failure_rolls_back() {
    use planner_core::proposals::*;
    let mut s = Session::in_memory(None).unwrap();
    let proposal = Proposal {
        id: String::new(),
        source: ProposalSource::GlobalSolver,
        title: "TEST SITE".into(),
        goal: vec![],
        status: ProposalStatus::Draft,
        number: 0,
        snapshot_time: "2026-01-01T00:00:00Z".into(),
        input_hash: s.plan_hash(),
        provenance: "TEST".into(),
        milestone: None,
        items: vec![ProposalItem {
            id: "item-1".into(),
            kind: ProposalItemKind::Create,
            included: true,
            label: "+ PROPOSED SITE — NEW".into(),
            detail: String::new(),
            impact: String::new(),
            commands: vec![Command::CreateFactory {
                name: "PROPOSED SITE".into(),
                position: MapPos {
                    x: 100.0,
                    y: 100.0,
                    z: 0.0,
                },
                region: "GRASS FIELDS".into(),
            }],
            aliases: vec![None],
            depends_on: vec![],
            sync: None,
            conflict: None,
        }],
    };
    let pid = s
        .edit(vec![Command::CreateProposal { proposal }])
        .unwrap()
        .created[0]
        .clone();
    let factories_before = s.state.factories.len();

    s.store.faults_mut().fail_commits = 1;
    assert!(s.accept_proposal(&pid).is_err());
    assert_eq!(
        s.state.proposals[&pid].status,
        ProposalStatus::Draft,
        "proposal still Draft after rollback"
    );
    assert_eq!(
        s.state.factories.len(),
        factories_before,
        "no materialized entities"
    );
    assert_disk_matches_memory(&s);

    // A clean accept then succeeds.
    s.accept_proposal(&pid).unwrap();
    assert_eq!(s.state.proposals[&pid].status, ProposalStatus::Accepted);
    assert_eq!(s.state.factories.len(), factories_before + 1);
    assert_disk_matches_memory(&s);
}

#[test]
fn solver_write_backs_roll_back_with_failed_edit() {
    let mut s = Session::in_memory(None).unwrap();
    let (_fid, out_port, smelt) = build_modular_frame_factory(&mut s);
    assert_eq!(s.state.groups[&smelt].count, 1);
    let rate_before = s.state.ports[&out_port].rate;

    // The failing edit's tx also carries solver write-backs (counts/clocks,
    // route manifests) — they must ride the same rollback.
    s.store.faults_mut().fail_commits = 1;
    assert!(s
        .edit(vec![Command::SetPortRate {
            id: out_port.clone(),
            rate: 2.0,
        }])
        .is_err());
    assert_eq!(s.state.groups[&smelt].count, 1, "write-back reverted");
    assert!((s.state.ports[&out_port].rate - rate_before).abs() < 1e-9);
    assert_disk_matches_memory(&s);

    // Retried, the same edit lands write-backs normally.
    s.edit(vec![Command::SetPortRate {
        id: out_port,
        rate: 2.0,
    }])
    .unwrap();
    assert_eq!(s.state.groups[&smelt].count, 2);
    assert_disk_matches_memory(&s);
}

// ---- W2a refactor/cutover: plan a replacement, accept without touching ◆ ----

/// Import a single-machine ◆ built factory producing iron ingot (net surplus →
/// an Out port), and return its factory id.
fn import_built_ingot(s: &mut Session, x: f64) -> Id {
    let mach = |class: &str, recipe: &str, x: f64| app::import::ImportMachine {
        class: class.into(),
        recipe: Some(recipe.into()),
        clock: 1.0,
        x,
        y: 0.0,
        z: 0.0,
        ..Default::default()
    };
    s.import_save(app::import::ImportSnapshot {
        save_name: "OLD-INGOT".into(),
        machines: vec![mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", x)],
        ..Default::default()
    })
    .unwrap();
    s.state
        .factories
        .values()
        .find(|f| f.status == Status::Built)
        .map(|f| f.id.clone())
        .unwrap()
}

/// plan_replacement drafts a Refactor proposal: source Refactor, a CREATE item
/// carrying the site alias, and a trailing SetFactoryReplaces { $site → old }.
#[test]
fn plan_replacement_builds_refactor_proposal() {
    let mut s = Session::in_memory(None).unwrap();
    let old = import_built_ingot(&mut s, 0.0);
    let old_pos = s.state.factories[&old].position;

    let proposal = s.plan_replacement(old.clone(), None).unwrap();
    assert_eq!(
        proposal.source,
        planner_core::proposals::ProposalSource::Refactor
    );
    // the CREATE item mints the new factory beside the old pin AND appends the
    // SetFactoryReplaces link referencing the site alias + the old id.
    let create = proposal
        .items
        .iter()
        .find(|it| it.kind == planner_core::proposals::ProposalItemKind::Create)
        .expect("CREATE item");
    let link = create
        .commands
        .iter()
        .find_map(|c| match c {
            Command::SetFactoryReplaces { id, replaces } => Some((id.clone(), replaces.clone())),
            _ => None,
        })
        .expect("trailing SetFactoryReplaces");
    assert_eq!(link.0, "$site", "links the freshly-minted site alias");
    assert_eq!(link.1, Some(old.clone()));
    // the new site is placed beside the old pin (x shifted, y shared)
    let new_pos = create
        .commands
        .iter()
        .find_map(|c| match c {
            Command::CreateFactory { position, .. } => Some(*position),
            _ => None,
        })
        .unwrap();
    assert!(new_pos.x > old_pos.x, "sited to the side of the old pin");
    assert_eq!(new_pos.y, old_pos.y);
}

/// Accepting a Refactor proposal is one undo step and NEVER touches the ◆ built
/// layer: the old factory's groups/counts are byte-identical afterward, the new
/// ◇ carries `replaces`, and undo restores the pre-accept state.
#[test]
fn accept_refactor_is_one_undo_step_and_old_built_untouched() {
    let mut s = Session::in_memory(None).unwrap();
    let old = import_built_ingot(&mut s, 0.0);
    // snapshot the ◆ built factory + its groups before the refactor
    let old_before = s.state.factories[&old].clone();
    let groups_before: std::collections::BTreeMap<Id, MachineGroup> = s
        .state
        .groups
        .values()
        .filter(|g| g.factory == old)
        .map(|g| (g.id.clone(), g.clone()))
        .collect();

    let proposal = s.plan_replacement(old.clone(), None).unwrap();
    let pid = s
        .edit(vec![Command::CreateProposal { proposal }])
        .unwrap()
        .created[0]
        .clone();
    let factories_before = s.state.factories.len();
    s.accept_proposal(&pid).unwrap();

    // the old ◆ factory + groups are byte-identical (never a ◆ write)
    assert_eq!(s.state.factories[&old], old_before, "◆ factory untouched");
    for (gid, g) in &groups_before {
        assert_eq!(&s.state.groups[gid], g, "◆ group untouched");
    }
    // a NEW ◇ factory appeared carrying replaces → old
    let new = s
        .state
        .factories
        .values()
        .find(|f| f.replaces.as_deref() == Some(old.as_str()))
        .expect("new ◇ carries replaces");
    assert_eq!(new.status, Status::Planned);
    // a cutover now exists pairing new → old
    assert!(s
        .solve_all_readonly()
        .cutovers
        .iter()
        .any(|c| c.old_factory == old));

    // one undo reverts the ENTIRE accept (◇-only) in a single step
    s.undo().unwrap().unwrap();
    assert_eq!(s.state.factories.len(), factories_before);
    assert!(
        !s.state
            .factories
            .values()
            .any(|f| f.replaces.as_deref() == Some(old.as_str())),
        "replacement gone after undo"
    );
    assert_eq!(s.state.factories[&old], old_before, "◆ still untouched");
}

/// Review minor M8: the clamp write-back must only ever rewrite an OUT port's
/// target. For an In-port SetPortRate under a clamped solve, result.ports
/// carries the solved INTAKE — writing that back would replace the value the
/// same command batch just set with an unrelated flow figure (reachable via the
/// raw command API; no shipped UI edits In-port rates).
#[test]
fn in_port_rate_survives_clamped_solve_write_back() {
    let mut s = Session::in_memory(None).unwrap();
    let fid = s
        .edit(vec![Command::CreateFactory {
            name: "CLAMP WORKS".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
        .clone();
    let inp = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::In,
            item: "Desc_OreIron_C".into(),
            rate: 0.0,
            rate_ceiling: Some(300.0),
            graph_pos: GraphPos { x: 0.0, y: 100.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let out = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronIngot_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: GraphPos { x: 600.0, y: 100.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let g = add_group(
        &mut s,
        &fid,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        gp(300.0, 100.0),
    );
    connect_in(
        &mut s,
        &fid,
        EdgeEnd::Port(inp.clone()),
        EdgeEnd::Group(g.clone()),
        "Desc_OreIron_C",
        6,
    );
    connect_in(
        &mut s,
        &fid,
        EdgeEnd::Group(g),
        EdgeEnd::Port(out.clone()),
        "Desc_IronIngot_C",
        6,
    );

    // Achievable target at the generous ceiling…
    s.edit(vec![Command::SetPortRate {
        id: out.clone(),
        rate: 300.0,
    }])
    .unwrap();
    assert!((s.state.ports[&out].rate - 300.0).abs() < 1e-6);
    // …then the ceiling drops (Recompute trigger — no target write-back), so
    // the stored out target now permanently exceeds the achievable ceiling.
    s.edit(vec![Command::SetPortCeiling {
        id: inp.clone(),
        rate_ceiling: Some(120.0),
    }])
    .unwrap();
    assert!(
        (s.state.ports[&out].rate - 300.0).abs() < 1e-6,
        "stored target kept"
    );

    // Editing the In port's rate under this clamped solve must keep the value
    // the user set — not the solver's intake figure.
    s.edit(vec![Command::SetPortRate {
        id: inp.clone(),
        rate: 100.0,
    }])
    .unwrap();
    assert!(
        (s.state.ports[&inp].rate - 100.0).abs() < 1e-6,
        "In-port rate sticks: {}",
        s.state.ports[&inp].rate
    );
}

// Regression (task #75): a group whose recipe can't be resolved — a power
// generator (imported with an empty recipe) or an unknown/modded recipe class —
// must NOT make the WHOLE factory fail to solve. Before the fix, `snapshot`'s
// `recipes.get(&g.recipe)?` early-returned None on the first such group, so the
// factory reported "missing recipe or machine data" and EVERY machine rendered
// 0/min. A real imported save with a biomass generator inside a production
// factory hit exactly this.
#[test]
fn unresolvable_recipe_group_does_not_fail_the_whole_factory() {
    let mut s = Session::in_memory(None).unwrap();
    let r = s
        .edit(vec![Command::CreateFactory {
            name: "MIXED".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap();
    let fid = r.created[0].clone();

    // A valid production group (fixture recipe) …
    let good = add_group(
        &mut s,
        &fid,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        gp(200.0, 200.0),
    );
    // … and a generator-style group with an unresolvable (empty) recipe, exactly
    // as import creates it. The edit response carries the fresh solve.
    let resp = s
        .edit(vec![Command::AddGroup {
            factory: fid.clone(),
            machine: "Build_GeneratorBiomass_Automated_C".into(),
            recipe: "".into(),
            count: 1,
            clock: 1.0,
            graph_pos: gp(200.0, 400.0),
            floor: 0,
        }])
        .unwrap();
    let df = &resp.derived.factories[&fid];
    assert!(
        df.solve_error.is_none(),
        "one unresolvable-recipe group must not error the whole factory: {:?}",
        df.solve_error
    );

    // The snapshot skips the generator and keeps the valid group.
    let snap = s.snapshot(&fid).expect("factory snapshots");
    assert_eq!(snap.groups.len(), 1, "the unresolvable group is skipped");
    assert_eq!(
        snap.groups[0].id, good,
        "the valid production group survives"
    );
}

// Task #82: skipping an unresolvable group is right, but a genuine unknown
// PRODUCTION recipe (a modded machine, or a Docs.json missing a recipe the save
// uses) must be SURFACED — otherwise the factory silently under-counts. A
// generator (empty recipe) is not a warning; a non-empty unresolvable recipe is.
#[test]
fn unknown_production_recipe_surfaces_a_warning_but_a_generator_does_not() {
    let mut s = Session::in_memory(None).unwrap();
    let r = s
        .edit(vec![Command::CreateFactory {
            name: "MIXED".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap();
    let fid = r.created[0].clone();

    // A valid production group anchors the solve (so the factory isn't empty).
    add_group(
        &mut s,
        &fid,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        gp(200.0, 200.0),
    );

    // A generator (empty recipe) is skipped but is NOT a warning.
    s.edit(vec![Command::AddGroup {
        factory: fid.clone(),
        machine: "Build_GeneratorBiomass_Automated_C".into(),
        recipe: "".into(),
        count: 1,
        clock: 1.0,
        graph_pos: gp(200.0, 300.0),
        floor: 0,
    }])
    .unwrap();
    let d = s.solve_all_readonly();
    assert!(
        d.factories[&fid].warnings.is_empty(),
        "a generator's empty recipe must not warn: {:?}",
        d.factories[&fid].warnings
    );

    // A non-empty recipe absent from the catalog IS surfaced.
    let resp = s
        .edit(vec![Command::AddGroup {
            factory: fid.clone(),
            machine: "Build_SmelterMk1_C".into(),
            recipe: "Recipe_Totally_Unknown_C".into(),
            count: 2,
            clock: 1.0,
            graph_pos: gp(200.0, 400.0),
            floor: 0,
        }])
        .unwrap();
    let df = &resp.derived.factories[&fid];
    assert!(
        df.solve_error.is_none(),
        "an unknown recipe must not error the whole factory"
    );
    assert_eq!(
        df.warnings.len(),
        1,
        "one unknown-recipe production group is surfaced: {:?}",
        df.warnings
    );
    assert!(
        df.warnings[0].contains("catalog"),
        "the warning points at the catalog: {}",
        df.warnings[0]
    );
}

// Task #82 (review follow-up): a factory whose ONLY group is an unknown recipe
// lands in the "no machine groups yet" error path — exactly where the catalog
// pointer helps most — so the warning must survive there too, not be swallowed.
#[test]
fn all_unknown_recipe_factory_still_surfaces_the_catalog_warning() {
    let mut s = Session::in_memory(None).unwrap();
    let r = s
        .edit(vec![Command::CreateFactory {
            name: "MODDED".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap();
    let fid = r.created[0].clone();

    let resp = s
        .edit(vec![Command::AddGroup {
            factory: fid.clone(),
            machine: "Build_SmelterMk1_C".into(),
            recipe: "Recipe_Totally_Unknown_C".into(),
            count: 1,
            clock: 1.0,
            graph_pos: gp(200.0, 200.0),
            floor: 0,
        }])
        .unwrap();
    let df = &resp.derived.factories[&fid];
    assert_eq!(
        df.warnings.len(),
        1,
        "the catalog warning survives the no-groups error path: {:?}",
        df.warnings
    );
    assert!(df.warnings[0].contains("catalog"));
}

// ExpandGroup: a ×N bank materializes into N individual machines wired through
// real splitter/merger junction trees, and undo restores the exact ×N bank.
#[test]
fn expand_bank_materializes_machines_and_junctions_and_undoes() {
    let mut s = Session::in_memory(None).unwrap();
    let r = s
        .edit(vec![Command::CreateFactory {
            name: "IRON".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap();
    let fid = r.created[0].clone();
    let in_port = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::In,
            item: "Desc_OreIron_C".into(),
            rate: 0.0,
            rate_ceiling: Some(240.0),
            graph_pos: gp(0.0, 0.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let out_port = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronIngot_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: gp(600.0, 0.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    // a ×2 smelter bank, wired ore in / ingot out
    let bank = s
        .edit(vec![Command::AddGroup {
            factory: fid.clone(),
            machine: "Build_SmelterMk1_C".into(),
            recipe: "Recipe_IngotIron_C".into(),
            count: 2,
            clock: 1.0,
            graph_pos: gp(300.0, 0.0),
            floor: 0,
        }])
        .unwrap()
        .created[0]
        .clone();
    connect(
        &mut s,
        &fid,
        EdgeEnd::Port(in_port.clone()),
        EdgeEnd::Group(bank.clone()),
        "Desc_OreIron_C",
        1,
    );
    connect(
        &mut s,
        &fid,
        EdgeEnd::Group(bank.clone()),
        EdgeEnd::Port(out_port.clone()),
        "Desc_IronIngot_C",
        1,
    );

    assert_eq!(s.state.groups.len(), 1);
    assert_eq!(s.state.junctions.len(), 0);
    assert_eq!(s.state.edges.len(), 2);

    s.edit(vec![Command::ExpandGroup { id: bank.clone() }])
        .unwrap();

    // the ×2 bank is gone, replaced by 2 count-1 machines
    assert!(!s.state.groups.contains_key(&bank), "bank removed");
    assert_eq!(s.state.groups.len(), 2, "two individual machines");
    assert!(s.state.groups.values().all(|g| g.count == 1));
    // 1 splitter + 1 merger (balanced tree for N=2)
    assert_eq!(s.state.junctions.len(), 2, "one splitter + one merger");
    // in→splitter, splitter→m1, splitter→m2, m1→merger, m2→merger, merger→out
    assert_eq!(s.state.edges.len(), 6, "rewired belt topology");
    // the port belts survived, now landing on junctions (not the deleted bank)
    let in_edge = s
        .state
        .edges
        .values()
        .find(|e| e.from == EdgeEnd::Port(in_port.clone()))
        .unwrap();
    assert!(
        matches!(in_edge.to, EdgeEnd::Junction(_)),
        "input belt feeds a splitter"
    );
    let out_edge = s
        .state
        .edges
        .values()
        .find(|e| e.to == EdgeEnd::Port(out_port.clone()))
        .unwrap();
    assert!(
        matches!(out_edge.from, EdgeEnd::Junction(_)),
        "output belt leaves a merger"
    );
    // the factory's group list tracks exactly the 2 machines
    let f = s.state.factories.get(&fid).unwrap();
    assert_eq!(f.groups.len(), 2);

    // undo restores the exact pre-expand state
    s.undo().unwrap().unwrap();
    assert!(s.state.groups.contains_key(&bank), "bank restored");
    assert_eq!(s.state.groups.len(), 1);
    assert_eq!(s.state.groups[&bank].count, 2);
    assert_eq!(s.state.junctions.len(), 0, "junctions gone");
    assert_eq!(s.state.edges.len(), 2, "original belts restored");
    let in_edge = s
        .state
        .edges
        .values()
        .find(|e| e.from == EdgeEnd::Port(in_port.clone()))
        .unwrap();
    assert_eq!(
        in_edge.to,
        EdgeEnd::Group(bank.clone()),
        "input belt back on the bank"
    );
}

#[test]
fn new_empire_wipes_the_plan_and_journal_but_keeps_the_catalog() {
    let mut s = Session::in_memory(None).unwrap();
    let recipes_before = s.gamedata.recipes.len();
    let nodes_before = s.world.nodes.len();
    assert!(recipes_before > 0, "the fixture catalog is loaded");

    // Build a real plan with an edit history (so undo is non-empty).
    build_modular_frame_factory(&mut s);
    assert!(!s.state.factories.is_empty(), "plan has factories");
    assert!(s.undo.can_undo(), "edits left an undo history");

    // Save-derived facts that new_empire must also clear (a discarded save's
    // milestones/alt-recipe gating must not leak into the fresh empire).
    s.unlocked.insert("Recipe_Alternate_Wire_1_C".into());
    s.purchased_schematics.insert("Schematic_3-1_C".into());

    let resp = s.new_empire().unwrap();
    assert!(s.unlocked.is_empty(), "unlocked recipes cleared");
    assert!(
        s.purchased_schematics.is_empty(),
        "purchased schematics cleared"
    );
    assert!(s.advisor.cards.is_empty(), "advisor state reset");

    // Plan + journal wiped...
    assert!(s.state.factories.is_empty(), "factories wiped");
    assert!(s.state.groups.is_empty(), "groups wiped");
    assert!(s.state.ports.is_empty(), "ports wiped");
    assert!(!s.undo.can_undo(), "undo stack cleared");
    assert!(!s.undo.can_redo(), "redo stack cleared");
    assert!(
        !resp.can_undo && !resp.can_redo,
        "response reflects the empty journal"
    );
    // ...but the catalog + world are KEPT (a new save must not cost the catalog).
    assert_eq!(
        s.gamedata.recipes.len(),
        recipes_before,
        "gamedata catalog kept"
    );
    assert_eq!(s.world.nodes.len(), nodes_before, "world snapshot kept");

    // A fresh edit works on the empty plan (no lingering corruption).
    let fid = s
        .edit(vec![Command::CreateFactory {
            name: "AFTER RESET".into(),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
        .clone();
    assert_eq!(s.state.factories.len(), 1);
    assert_eq!(s.state.factories[&fid].name, "AFTER RESET");
}

// ---------------------------------------------------------------------------
// Audit #124: per-grid generation carries the SAME nameplate fallback as the
// empire total. A recipe-less generator (every imported generator, and
// geothermal by design — no synthesized burn recipe) contributes no solved
// __PowerMW out_rate, so the per-grid sum used to read 0 MW while the status
// bar showed the nameplate — two contradictory numbers for one physical
// plant, and switch-shed thresholds computed off the 0 baseline.
// ---------------------------------------------------------------------------

/// A grid holding two recipe-less geothermal generators (200 MW nameplate
/// each) must attribute 400 MW to the CIRCUIT, matching total_generation_mw.
#[test]
fn recipe_less_generator_grid_reads_nameplate_not_zero() {
    let mut s = Session::in_memory(None).unwrap();
    let geo = mk_factory(&mut s, "GEO FARM", 0.0);
    // Recipe-less (imported-style) generator group, count 2 via two adds of 1?
    // AddGroup helper is count=1; add one group then set its count to 2.
    let g = add_group(
        &mut s,
        &geo,
        "Build_GeneratorGeoThermal_C",
        "",
        GraphPos { x: 0.0, y: 0.0 },
    );
    s.edit(vec![Command::SetGroupCount { id: g, count: 2 }])
        .unwrap();
    let load = mk_factory(&mut s, "LOAD SINK", 800.0);
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Power,
        from: geo.clone(),
        to: load.clone(),
        path: vec![
            MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            MapPos {
                x: 800.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    }])
    .unwrap();

    let d = s.solve_all_readonly();
    assert_eq!(d.circuits.len(), 1, "one grid joining the two factories");
    let c = &d.circuits[0];
    assert!(
        (c.generation_mw - 400.0).abs() < 1e-6,
        "grid card reads the 2 x 200 MW nameplate, got {}",
        c.generation_mw
    );
    // Empire total and per-grid attribution agree by construction.
    assert!(
        (d.total_generation_mw - c.generation_mw).abs() < 1e-6,
        "empire total ({}) must equal the grid's generation ({})",
        d.total_generation_mw,
        c.generation_mw
    );
}

/// The fallback must NOT mask a solved, fuel-starved plant: a coal generator
/// with a resolvable burn recipe and a solved factory keeps its REAL (solved)
/// output in both the per-grid sum and the empire total — nameplate applies
/// only when the recipe can't resolve.
#[test]
fn solved_generator_keeps_real_output_in_grid_sum() {
    let mut s = Session::in_memory(None).unwrap();
    let plant = mk_factory(&mut s, "COAL PLANT", 0.0);
    let coal_in = mk_port(
        &mut s,
        &plant,
        PortDirection::In,
        "Desc_Coal_C",
        Some(480.0),
    );
    let mw_out = mk_port(&mut s, &plant, PortDirection::Out, "__PowerMW", None);
    let gens = add_group(
        &mut s,
        &plant,
        "Build_GeneratorCoal_C",
        "Recipe_Power_Build_GeneratorCoal_Desc_Coal_C",
        GraphPos { x: 0.0, y: 0.0 },
    );
    connect(
        &mut s,
        &plant,
        EdgeEnd::Port(coal_in),
        EdgeEnd::Group(gens.clone()),
        "Desc_Coal_C",
        6,
    );
    connect(
        &mut s,
        &plant,
        EdgeEnd::Group(gens),
        EdgeEnd::Port(mw_out.clone()),
        "__PowerMW",
        6,
    );
    // Drive the plant to 30 MW — well under the 75 MW nameplate.
    s.edit(vec![Command::SetPortRate {
        id: mw_out,
        rate: 30.0,
    }])
    .unwrap();
    let load = mk_factory(&mut s, "TOWN", 800.0);
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Power,
        from: plant.clone(),
        to: load.clone(),
        path: vec![
            MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            MapPos {
                x: 800.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    }])
    .unwrap();

    let d = s.solve_all_readonly();
    assert_eq!(d.circuits.len(), 1);
    let c = &d.circuits[0];
    assert!(
        (c.generation_mw - 30.0).abs() < 1e-6,
        "solved output (30 MW), not the 75 MW nameplate, got {}",
        c.generation_mw
    );
    assert!(
        (d.total_generation_mw - 30.0).abs() < 1e-6,
        "empire total matches the solved grid figure, got {}",
        d.total_generation_mw
    );
}

// ---------------------------------------------------------------------------
// Audit #125: multi-output deficit gating/sizing must use the EDITED output's
// requested rate — never the factory's first Out port. Before the fix, an
// unrelated sibling output's target both fired PHANTOM deficits (sibling.rate
// > the edited output's max_rate while the edited target was fully met) and
// mis-scaled `needed` by the wrong target.
// ---------------------------------------------------------------------------
#[test]
fn multi_output_deficit_scales_by_edited_output_not_first_port() {
    let mut s = Session::in_memory(None).unwrap();

    // Upstream: rod factory shipping 30 rods/min over a route.
    let rod_fid = mk_factory(&mut s, "ROD SOURCE", 0.0);
    let rod_in = mk_port(
        &mut s,
        &rod_fid,
        PortDirection::In,
        "Desc_OreIron_C",
        Some(240.0),
    );
    let rod_out = mk_port(&mut s, &rod_fid, PortDirection::Out, "Desc_IronRod_C", None);
    let smelt = add_group(
        &mut s,
        &rod_fid,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        gp(200.0, 100.0),
    );
    let rods = add_group(
        &mut s,
        &rod_fid,
        "Build_ConstructorMk1_C",
        "Recipe_IronRod_C",
        gp(400.0, 100.0),
    );
    let g = EdgeEnd::Group;
    connect_in(
        &mut s,
        &rod_fid,
        EdgeEnd::Port(rod_in),
        g(smelt.clone()),
        "Desc_OreIron_C",
        6,
    );
    connect_in(
        &mut s,
        &rod_fid,
        g(smelt),
        g(rods.clone()),
        "Desc_IronIngot_C",
        6,
    );
    connect_in(
        &mut s,
        &rod_fid,
        g(rods),
        EdgeEnd::Port(rod_out.clone()),
        "Desc_IronRod_C",
        6,
    );

    // Downstream MIXED WORKS with TWO INDEPENDENT chains:
    //   chain 1 (created FIRST so its Out is the factory's first Out port):
    //     open ore In → smelter → out_ingot        (the unrelated sibling)
    //   chain 2: route-fed rod In → constructor → out_screws (the edited one)
    let mix_fid = mk_factory(&mut s, "MIXED WORKS", 900.0);
    let ore_in = mk_port(&mut s, &mix_fid, PortDirection::In, "Desc_OreIron_C", None);
    let out_ingot = mk_port(
        &mut s,
        &mix_fid,
        PortDirection::Out,
        "Desc_IronIngot_C",
        None,
    );
    let mix_smelt = add_group(
        &mut s,
        &mix_fid,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        gp(200.0, 100.0),
    );
    connect_in(
        &mut s,
        &mix_fid,
        EdgeEnd::Port(ore_in),
        g(mix_smelt.clone()),
        "Desc_OreIron_C",
        6,
    );
    connect_in(
        &mut s,
        &mix_fid,
        g(mix_smelt),
        EdgeEnd::Port(out_ingot.clone()),
        "Desc_IronIngot_C",
        6,
    );
    let rods_in = mk_port(&mut s, &mix_fid, PortDirection::In, "Desc_IronRod_C", None);
    let out_screws = mk_port(
        &mut s,
        &mix_fid,
        PortDirection::Out,
        "Desc_IronScrew_C",
        None,
    );
    let screws = add_group(
        &mut s,
        &mix_fid,
        "Build_ConstructorMk1_C",
        "Recipe_Screw_C",
        gp(400.0, 300.0),
    );
    connect_in(
        &mut s,
        &mix_fid,
        EdgeEnd::Port(rods_in.clone()),
        g(screws.clone()),
        "Desc_IronRod_C",
        6,
    );
    connect_in(
        &mut s,
        &mix_fid,
        g(screws),
        EdgeEnd::Port(out_screws.clone()),
        "Desc_IronScrew_C",
        6,
    );

    // Route: rod OUT → MIXED rod IN, tier 6 (no belt interference).
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Belt { tier: 6 },
        from: rod_out.clone(),
        to: rods_in.clone(),
        path: vec![
            MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            MapPos {
                x: 900.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    }])
    .unwrap();
    s.edit(vec![Command::SetPortRate {
        id: rod_out,
        rate: 30.0,
    }])
    .unwrap();
    // The unrelated sibling output targets 200 ingot/min (feasible, open ore).
    s.edit(vec![Command::SetPortRate {
        id: out_ingot,
        rate: 200.0,
    }])
    .unwrap();

    // PHANTOM check: 100 screws need 25 rods — fully covered by the 30
    // supplied. NO deficit may fire, even though the sibling's 200/min target
    // exceeds the screw chain's 120/min supply ceiling (the old gate compared
    // the FIRST out port's rate against the edited output's max_rate).
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: out_screws.clone(),
            rate: 100.0,
        }])
        .unwrap();
    assert!(
        resp.derived.deficits.is_empty(),
        "met target must not report a deficit; got {:?}",
        resp.derived.deficits
    );

    // SIZING check: 480 screws need 120 rods; only 30 ship → clamp at 120
    // screws. `needed` must be the EDITED output's requirement (120 rods),
    // not first-out scaled (30 x 200/120 = 50).
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: out_screws,
            rate: 480.0,
        }])
        .unwrap();
    let d = &resp.derived;
    assert_eq!(d.deficits.len(), 1, "starved screw chain reports one row");
    let row = &d.deficits[0];
    assert_eq!(row.factory, mix_fid);
    assert_eq!(row.port, rods_in);
    assert!(
        (row.supplied - 30.0).abs() < 1e-4,
        "supplied {}",
        row.supplied
    );
    assert!(
        (row.needed - 120.0).abs() < 1e-4,
        "480 screws need 120 rods (not a first-out-scaled figure): {}",
        row.needed
    );
}

/// PR #49 review: a raw In-port SetPortRate (reachable only via the command
/// API — no shipped UI emits one) must NOT feed its input-unit rate into the
/// clamped-channel deficit sizing. The Out-direction guard makes it fall to
/// the sole-Out-port arm, matching trigger_for_factory's synthesized solve:
/// with the sole Out target fully met by the supply, no deficit row at all.
#[test]
fn in_port_rate_edit_does_not_fabricate_a_deficit() {
    let mut s = Session::in_memory(None).unwrap();
    // Upstream rod source shipping 40 rods/min.
    let rod_fid = mk_factory(&mut s, "ROD SRC 2", 0.0);
    let rod_in = mk_port(
        &mut s,
        &rod_fid,
        PortDirection::In,
        "Desc_OreIron_C",
        Some(240.0),
    );
    let rod_out = mk_port(&mut s, &rod_fid, PortDirection::Out, "Desc_IronRod_C", None);
    let smelt = add_group(
        &mut s,
        &rod_fid,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        gp(200.0, 100.0),
    );
    let rods = add_group(
        &mut s,
        &rod_fid,
        "Build_ConstructorMk1_C",
        "Recipe_IronRod_C",
        gp(400.0, 100.0),
    );
    let g = EdgeEnd::Group;
    connect_in(
        &mut s,
        &rod_fid,
        EdgeEnd::Port(rod_in),
        g(smelt.clone()),
        "Desc_OreIron_C",
        6,
    );
    connect_in(
        &mut s,
        &rod_fid,
        g(smelt),
        g(rods.clone()),
        "Desc_IronIngot_C",
        6,
    );
    connect_in(
        &mut s,
        &rod_fid,
        g(rods),
        EdgeEnd::Port(rod_out.clone()),
        "Desc_IronRod_C",
        6,
    );
    // Downstream single-output screw factory.
    let screw_fid = mk_factory(&mut s, "SCREW SINK", 900.0);
    let screw_in = mk_port(
        &mut s,
        &screw_fid,
        PortDirection::In,
        "Desc_IronRod_C",
        None,
    );
    let screw_out = mk_port(
        &mut s,
        &screw_fid,
        PortDirection::Out,
        "Desc_IronScrew_C",
        None,
    );
    let screws = add_group(
        &mut s,
        &screw_fid,
        "Build_ConstructorMk1_C",
        "Recipe_Screw_C",
        gp(300.0, 100.0),
    );
    connect_in(
        &mut s,
        &screw_fid,
        EdgeEnd::Port(screw_in.clone()),
        g(screws.clone()),
        "Desc_IronRod_C",
        6,
    );
    connect_in(
        &mut s,
        &screw_fid,
        g(screws),
        EdgeEnd::Port(screw_out.clone()),
        "Desc_IronScrew_C",
        6,
    );
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Belt { tier: 6 },
        from: rod_out.clone(),
        to: screw_in.clone(),
        path: vec![
            MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            MapPos {
                x: 900.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    }])
    .unwrap();
    s.edit(vec![Command::SetPortRate {
        id: rod_out,
        rate: 40.0,
    }])
    .unwrap();
    // Out target 160 screws = 40 rods — exactly met by the supply.
    s.edit(vec![Command::SetPortRate {
        id: screw_out,
        rate: 160.0,
    }])
    .unwrap();

    // Raw In-port rate edit with an arbitrary figure: the response must not
    // fabricate a deficit (the sole Out target is fully supplied).
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: screw_in,
            rate: 999.0,
        }])
        .unwrap();
    assert!(
        resp.derived.deficits.is_empty(),
        "met sole-Out target: no deficit despite the In-port 999 edit; got {:?}",
        resp.derived.deficits
    );
}

/// Audit #127 / PR #57: an IDLE solve (nothing consumes a group's output) is
/// absence of demand, not a sizing — the settle write-back must NOT erase the
/// user's authored clock with 0. A group that IS demanded still gets sized.
#[test]
fn idle_settle_preserves_the_authored_clock() {
    let mut s = Session::in_memory(None).unwrap();
    let fid = s
        .edit(vec![Command::CreateFactory {
            name: "IDLE CLOCK".into(),
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
    let in_port = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::In,
            item: "Desc_IronIngot_C".into(),
            rate: 0.0,
            rate_ceiling: Some(200.0),
            graph_pos: gp(0.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let g = s
        .edit(vec![Command::AddGroup {
            factory: fid.clone(),
            machine: "Build_ConstructorMk1_C".into(),
            recipe: "Recipe_IronRod_C".into(),
            count: 1,
            clock: 0.5,
            graph_pos: gp(320.0, 100.0),
            floor: 0,
        }])
        .unwrap()
        .created[0]
        .clone();
    s.edit(vec![Command::AddEdge {
        factory: fid.clone(),
        from: EdgeEnd::Port(in_port),
        to: EdgeEnd::Group(g.clone()),
        item: "Desc_IronIngot_C".into(),
        tier: 3,
    }])
    .unwrap();

    // The rod output has no consumer and no export target: the group solves
    // idle on every one of those settles. The authored clock must survive.
    assert_eq!(s.state.groups[&g].count, 1);
    assert!(
        (s.state.groups[&g].clock - 0.5).abs() < 1e-9,
        "idle settle erased the authored clock: {}",
        s.state.groups[&g].clock
    );

    // Once the output is DEMANDED (export target), the settle sizes the group
    // for real — demand-driven write-back still works.
    let out_port = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronRod_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: gp(680.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    s.edit(vec![Command::AddEdge {
        factory: fid.clone(),
        from: EdgeEnd::Group(g.clone()),
        to: EdgeEnd::Port(out_port.clone()),
        item: "Desc_IronRod_C".into(),
        tier: 3,
    }])
    .unwrap();
    s.edit(vec![Command::SetPortRate {
        id: out_port,
        rate: 15.0,
    }])
    .unwrap();
    assert!(
        (s.state.groups[&g].clock - 1.0).abs() < 1e-6,
        "a demanded group is still sized by the settle: {}",
        s.state.groups[&g].clock
    );
}
