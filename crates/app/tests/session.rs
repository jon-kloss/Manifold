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
