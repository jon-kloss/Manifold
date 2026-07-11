//! Golden solver cases (SDD §11). The Modular Frame chain is fully worked in
//! the HANDOFF: at target T/min — ore 24T, rods 10.5T, screws 18T, plates 9T,
//! reinforced iron plate 1.5T.

use solver::model::*;

fn recipe(
    id: &str,
    machine: &str,
    dur: f64,
    inputs: &[(&str, f64)],
    outputs: &[(&str, f64)],
    mw: f64,
) -> RecipeSpec {
    RecipeSpec {
        id: id.into(),
        machine: machine.into(),
        duration_s: dur,
        inputs: inputs.iter().map(|(i, a)| (i.to_string(), *a)).collect(),
        outputs: outputs.iter().map(|(i, a)| (i.to_string(), *a)).collect(),
        power_mw: mw,
    }
}

fn group(id: &str, r: RecipeSpec) -> GroupSpec {
    GroupSpec {
        id: id.into(),
        recipe: r,
        count: 1,
        clock: 1.0,
    }
}

fn edge(id: &str, from: NodeRef, to: NodeRef, item: &str, capacity: f64) -> EdgeSpec {
    EdgeSpec {
        id: id.into(),
        from,
        to,
        item: item.into(),
        capacity,
    }
}

fn g(id: &str) -> NodeRef {
    NodeRef::Group(id.into())
}

/// The Modular Frame factory: ore in, modular frames out. Standard recipes.
fn modular_frame_snapshot(
    target: f64,
    ore_ceiling: Option<f64>,
    screw_belt_cap: f64,
) -> FactorySnapshot {
    FactorySnapshot {
        groups: vec![
            group(
                "smelt",
                recipe(
                    "Recipe_IngotIron_C",
                    "smelter",
                    2.0,
                    &[("ore", 1.0)],
                    &[("ingot", 1.0)],
                    4.0,
                ),
            ),
            group(
                "rods",
                recipe(
                    "Recipe_IronRod_C",
                    "constructor",
                    4.0,
                    &[("ingot", 1.0)],
                    &[("rod", 1.0)],
                    4.0,
                ),
            ),
            group(
                "screws",
                recipe(
                    "Recipe_Screw_C",
                    "constructor",
                    6.0,
                    &[("rod", 1.0)],
                    &[("screw", 4.0)],
                    4.0,
                ),
            ),
            group(
                "plates",
                recipe(
                    "Recipe_IronPlate_C",
                    "constructor",
                    6.0,
                    &[("ingot", 3.0)],
                    &[("plate", 2.0)],
                    4.0,
                ),
            ),
            group(
                "rip",
                recipe(
                    "Recipe_IronPlateReinforced_C",
                    "assembler",
                    12.0,
                    &[("plate", 6.0), ("screw", 12.0)],
                    &[("rip", 1.0)],
                    15.0,
                ),
            ),
            group(
                "mf",
                recipe(
                    "Recipe_ModularFrame_C",
                    "assembler",
                    60.0,
                    &[("rip", 3.0), ("rod", 12.0)],
                    &[("mf", 2.0)],
                    15.0,
                ),
            ),
        ],
        edges: vec![
            edge(
                "e-ore",
                NodeRef::Input("in-ore".into()),
                g("smelt"),
                "ore",
                780.0,
            ),
            edge("e-ingot-rods", g("smelt"), g("rods"), "ingot", 780.0),
            edge("e-ingot-plates", g("smelt"), g("plates"), "ingot", 780.0),
            edge("e-rod-screws", g("rods"), g("screws"), "rod", 780.0),
            edge("e-rod-mf", g("rods"), g("mf"), "rod", 780.0),
            edge("e-plate-rip", g("plates"), g("rip"), "plate", 780.0),
            edge(
                "e-screw-rip",
                g("screws"),
                g("rip"),
                "screw",
                screw_belt_cap,
            ),
            edge("e-rip-mf", g("rip"), g("mf"), "rip", 780.0),
            edge(
                "e-mf-out",
                g("mf"),
                NodeRef::Output("out-mf".into()),
                "mf",
                780.0,
            ),
        ],
        inputs: vec![InputPortSpec {
            id: "in-ore".into(),
            item: "ore".into(),
            ceiling: ore_ceiling,
        }],
        junctions: vec![],
        outputs: vec![OutputPortSpec {
            id: "out-mf".into(),
            item: "mf".into(),
            rate: target,
        }],
    }
}

/// Same chain, but the rod run feeds screws and final assembly through an
/// explicit splitter, and a merger sits on the RIP line — junctions must be
/// invisible to the math (pure conservation).
fn modular_frame_with_junctions(target: f64) -> FactorySnapshot {
    let mut snap = modular_frame_snapshot(target, None, 780.0);
    snap.junctions = vec!["split-rods".into(), "merge-rip".into()];
    let j = |id: &str| NodeRef::Junction(id.to_string());
    snap.edges
        .retain(|e| e.id != "e-rod-screws" && e.id != "e-rod-mf" && e.id != "e-rip-mf");
    snap.edges.extend([
        edge("e-rod-split", g("rods"), j("split-rods"), "rod", 780.0),
        edge("e-split-screws", j("split-rods"), g("screws"), "rod", 780.0),
        edge("e-split-mf", j("split-rods"), g("mf"), "rod", 780.0),
        edge("e-rip-merge", g("rip"), j("merge-rip"), "rip", 780.0),
        edge("e-merge-mf", j("merge-rip"), g("mf"), "rip", 780.0),
    ]);
    snap
}

fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "{what}: expected {expected}, got {actual}"
    );
}

fn assert_golden_rates(r: &SolveResult, t: f64) {
    assert_close(r.ports["in-ore"], 24.0 * t, "ore intake");
    assert_close(r.groups["rods"].out_rates["rod"], 10.5 * t, "rod rate");
    assert_close(
        r.groups["screws"].out_rates["screw"],
        18.0 * t,
        "screw rate",
    );
    assert_close(r.groups["plates"].out_rates["plate"], 9.0 * t, "plate rate");
    assert_close(r.groups["rip"].out_rates["rip"], 1.5 * t, "RIP rate");
    assert_close(r.ports["out-mf"], t, "target");
    // Mass balance on the shared rod edge split: 4.5T to screws, 6T direct to MF.
    assert_close(r.edges["e-rod-screws"].flow, 4.5 * t, "rods→screws");
    assert_close(r.edges["e-rod-mf"].flow, 6.0 * t, "rods→mf");
}

#[test]
fn t0_modular_frame_golden() {
    let snap = modular_frame_snapshot(0.0, None, 780.0);
    let r = solver::t0::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-mf".into(),
            rate: 2.0,
        },
    )
    .unwrap();
    assert_golden_rates(&r, 2.0);
    assert!(!r.clamped);
    // Counts: relax → ceil → clock redistribution.
    assert_eq!(r.groups["smelt"].count, 2); // 48/30 = 1.6
    assert_close(r.groups["smelt"].clock, 0.8, "smelter clock");
    assert_eq!(r.groups["rods"].count, 2); // 21/15 = 1.4
    assert_close(r.groups["rods"].clock, 0.7, "rod clock");
    assert_eq!(r.groups["mf"].count, 1);
}

#[test]
fn t1_modular_frame_golden() {
    let snap = modular_frame_snapshot(0.0, None, 780.0);
    let r = solver::t1::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-mf".into(),
            rate: 2.0,
        },
    )
    .unwrap();
    assert_golden_rates(&r, 2.0);
    assert!(!r.clamped);
}

#[test]
fn t0_t1_parity_within_epsilon() {
    // SDD §11: T0-WASM vs T1 fixed-point parity within epsilon.
    let snap = modular_frame_snapshot(0.0, Some(240.0), 120.0);
    for target in [0.5, 1.0, 2.0, 3.0] {
        let edit = T0Edit::SetTarget {
            port: "out-mf".into(),
            rate: target,
        };
        let a = solver::t0::solve(&snap, &edit).unwrap();
        let b = solver::t1::solve(&snap, &edit).unwrap();
        for (id, ga) in &a.groups {
            let gb = &b.groups[id];
            for (item, rate) in &ga.out_rates {
                assert!(
                    (rate - gb.out_rates[item]).abs() < 1e-4,
                    "parity {id}/{item}: t0={rate} t1={}",
                    gb.out_rates[item]
                );
            }
            assert_eq!(ga.count, gb.count, "count parity for {id}");
        }
        for (id, ea) in &a.edges {
            assert!(
                (ea.flow - b.edges[id].flow).abs() < 1e-4,
                "edge parity {id}"
            );
        }
    }
}

#[test]
fn t0_hard_stops_at_belt_capacity_and_names_it() {
    // Mk.1 belt (60/min) on the screw run: screws = 18T → ceiling T = 60/18.
    let snap = modular_frame_snapshot(0.0, None, 60.0);
    let r = solver::t0::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-mf".into(),
            rate: 5.0,
        },
    )
    .unwrap();
    assert!(r.clamped, "target beyond belt capacity must clamp");
    let ceiling = r.target_ceiling.clone().expect("ceiling must be reported");
    assert_close(ceiling.max_rate, 60.0 / 18.0, "ceiling rate");
    match ceiling.binding {
        Constraint::BeltCapacity { ref edge, .. } => assert_eq!(edge, "e-screw-rip"),
        ref other => panic!("expected belt binding, got {other:?}"),
    }
    // The plan solved at the clamp, not at the request, and the belt is saturated.
    assert_close(r.ports["out-mf"], 60.0 / 18.0, "clamped target");
    assert_close(
        r.edges["e-screw-rip"].saturation,
        1.0,
        "screw belt saturation",
    );
}

#[test]
fn t1_hard_stops_at_input_ceiling_and_names_it() {
    // Ore capped at 48/min → ceiling T = 2.0.
    let snap = modular_frame_snapshot(0.0, Some(48.0), 780.0);
    let r = solver::t1::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-mf".into(),
            rate: 3.0,
        },
    )
    .unwrap();
    assert!(r.clamped);
    let ceiling = r.target_ceiling.clone().expect("ceiling must be reported");
    assert_close(ceiling.max_rate, 2.0, "ceiling rate");
    match ceiling.binding {
        Constraint::InputCeiling { ref port, .. } => assert_eq!(port, "in-ore"),
        ref other => panic!("expected input-ceiling binding, got {other:?}"),
    }
    assert_golden_rates(&r, 2.0);
}

#[test]
fn mass_balance_property() {
    // Σin = Σout ± sinks at every group, across a sweep of targets.
    let snap = modular_frame_snapshot(0.0, None, 780.0);
    for i in 1..=20 {
        let t = i as f64 * 0.35;
        let r = solver::t0::solve(
            &snap,
            &T0Edit::SetTarget {
                port: "out-mf".into(),
                rate: t,
            },
        )
        .unwrap();
        for gspec in &snap.groups {
            let gr = &r.groups[&gspec.id];
            for (item, _) in &gspec.recipe.inputs {
                let inflow: f64 = snap
                    .edges
                    .iter()
                    .filter(|e| e.to == NodeRef::Group(gspec.id.clone()) && &e.item == item)
                    .map(|e| r.edges[&e.id].flow)
                    .sum();
                assert!(
                    (inflow - gr.in_rates[item]).abs() < 1e-6,
                    "conservation at {}/{item}",
                    gspec.id
                );
            }
        }
    }
}

#[test]
fn junctions_are_conservation_only() {
    // Splitter + merger topology must reproduce the golden numbers exactly,
    // with the splitter trunk carrying the full 10.5T rod flow.
    let snap = modular_frame_with_junctions(0.0);
    for solve in [solver::t0::solve, solver::t1::solve as fn(&_, &_) -> _] {
        let r = solve(
            &snap,
            &T0Edit::SetTarget {
                port: "out-mf".into(),
                rate: 2.0,
            },
        )
        .unwrap();
        assert_close(r.ports["in-ore"], 48.0, "ore intake");
        assert_close(r.groups["screws"].out_rates["screw"], 36.0, "screw rate");
        assert_close(
            r.edges["e-rod-split"].flow,
            21.0,
            "splitter trunk = full rod flow",
        );
        assert_close(
            r.edges["e-split-screws"].flow,
            9.0,
            "split branch to screws",
        );
        assert_close(r.edges["e-split-mf"].flow, 12.0, "split branch to assembly");
        assert_close(r.edges["e-rip-merge"].flow, 3.0, "merger in");
        assert_close(r.edges["e-merge-mf"].flow, 3.0, "merger out");
        assert_close(r.ports["out-mf"], 2.0, "target");
    }
}

#[test]
fn clock_edit_rederives_count() {
    let snap = modular_frame_snapshot(2.0, None, 780.0);
    // Smelter needs 48/min = 1.6 machines; at 50% clock that is 4 machines.
    let r = solver::t0::solve(
        &snap,
        &T0Edit::SetClock {
            group: "smelt".into(),
            clock: 0.5,
        },
    )
    .unwrap();
    assert_eq!(r.groups["smelt"].count, 4);
    assert_close(r.groups["smelt"].clock, 0.5, "explicit clock preserved");
    assert_close(
        r.groups["smelt"].out_rates["ingot"],
        48.0,
        "rate unchanged by clock edit",
    );
}
