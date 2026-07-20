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
        driven_cycles: None,
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

/// Two independent chains: in-a → ga → out-x and in-b → gb → out-y. Each
/// recipe converts 1:1 at 30/min per machine. Used for multi-output shortfall
/// cases — the chains share nothing, so shortfall allocation is unambiguous.
fn dual_chain_snapshot(rate_x: f64, rate_y: f64, b_ceiling: Option<f64>) -> FactorySnapshot {
    FactorySnapshot {
        groups: vec![
            group(
                "ga",
                recipe(
                    "Recipe_A_C",
                    "constructor",
                    2.0,
                    &[("a", 1.0)],
                    &[("x", 1.0)],
                    4.0,
                ),
            ),
            group(
                "gb",
                recipe(
                    "Recipe_B_C",
                    "constructor",
                    2.0,
                    &[("b", 1.0)],
                    &[("y", 1.0)],
                    4.0,
                ),
            ),
        ],
        edges: vec![
            edge("e-a", NodeRef::Input("in-a".into()), g("ga"), "a", 780.0),
            edge("e-x", g("ga"), NodeRef::Output("out-x".into()), "x", 780.0),
            edge("e-b", NodeRef::Input("in-b".into()), g("gb"), "b", 780.0),
            edge("e-y", g("gb"), NodeRef::Output("out-y".into()), "y", 780.0),
        ],
        inputs: vec![
            InputPortSpec {
                id: "in-a".into(),
                item: "a".into(),
                ceiling: None,
            },
            InputPortSpec {
                id: "in-b".into(),
                item: "b".into(),
                ceiling: b_ceiling,
            },
        ],
        junctions: vec![],
        outputs: vec![
            OutputPortSpec {
                id: "out-x".into(),
                item: "x".into(),
                rate: rate_x,
            },
            OutputPortSpec {
                id: "out-y".into(),
                item: "y".into(),
                rate: rate_y,
            },
        ],
    }
}

#[test]
fn t1_unwired_output_degrades_not_errors() {
    // SDD §5.2 'no dead ends': an output port with no inbound edge is the
    // routine mid-construction state — degrade, never hard-error.
    let mut snap = modular_frame_snapshot(0.0, None, 780.0);
    snap.edges.retain(|e| e.id != "e-mf-out");
    let r = solver::t1::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-mf".into(),
            rate: 30.0,
        },
    )
    .unwrap();
    // No named capacity ceiling → no clamp: the user's target stays canonical.
    assert!(!r.clamped, "structural shortfall must not clamp");
    assert!(r.target_ceiling.is_none());
    assert_close(r.ports["out-mf"], 0.0, "achieved rate");
    let sf = &r.shortfalls["out-mf"];
    assert_close(sf.requested, 30.0, "requested");
    assert_close(sf.missing, 30.0, "missing");
    match &sf.binding {
        Some(Constraint::Disconnected { node, item }) => {
            assert_eq!(node, "out-mf");
            assert_eq!(item, "mf");
        }
        other => panic!("expected disconnected binding, got {other:?}"),
    }
    // The rest of the chain idles rather than erroring away.
    assert_close(r.groups["mf"].out_rates["mf"], 0.0, "idle assembler");
}

#[test]
fn t1_unwired_group_input_degrades() {
    // Mid-construction case: the smelter's ore feed isn't wired yet, so the
    // whole chain pins to zero — report the gap and name the unwired group.
    let mut snap = modular_frame_snapshot(2.0, None, 780.0);
    snap.edges.retain(|e| e.id != "e-ore");
    let r = solver::t1::solve(&snap, &T0Edit::Recompute).unwrap();
    assert!(!r.clamped);
    assert_close(r.ports["out-mf"], 0.0, "achieved rate");
    let sf = &r.shortfalls["out-mf"];
    assert_close(sf.requested, 2.0, "requested");
    assert_close(sf.missing, 2.0, "missing");
    match &sf.binding {
        Some(Constraint::Disconnected { node, item }) => {
            assert_eq!(node, "smelt");
            assert_eq!(item, "ore");
        }
        other => panic!("expected disconnected binding, got {other:?}"),
    }
}

#[test]
fn t1_multi_output_partial_shortfall_names_ceiling() {
    // Recompute on a two-output factory (no ceiling-pass fallback): the tight
    // input ceiling starves out-y only — out-x stays whole, the shortfall
    // carries the named InputCeiling, and nothing dead-ends.
    let snap = dual_chain_snapshot(30.0, 30.0, Some(10.0));
    let r = solver::t1::solve(&snap, &T0Edit::Recompute).unwrap();
    assert_close(r.ports["out-x"], 30.0, "reachable port stays whole");
    assert_close(r.ports["out-y"], 10.0, "starved port at best achievable");
    assert_eq!(r.shortfalls.len(), 1);
    let sf = &r.shortfalls["out-y"];
    assert_close(sf.requested, 30.0, "requested");
    assert_close(sf.missing, 20.0, "missing");
    match &sf.binding {
        Some(Constraint::InputCeiling { port, ceiling, .. }) => {
            assert_eq!(port, "in-b");
            assert_close(*ceiling, 10.0, "ceiling");
        }
        other => panic!("expected input-ceiling binding, got {other:?}"),
    }
}

#[test]
fn t1_feasible_solves_have_empty_shortfalls() {
    // Regression lock: the shortfall channel stays silent whenever the solve
    // is feasible — including after a clamp settles at the named ceiling.
    let snap = modular_frame_snapshot(0.0, Some(48.0), 780.0);
    let r = solver::t1::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-mf".into(),
            rate: 1.0,
        },
    )
    .unwrap();
    assert!(!r.clamped);
    assert!(
        r.shortfalls.is_empty(),
        "feasible solve: {:?}",
        r.shortfalls
    );
    let r = solver::t1::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-mf".into(),
            rate: 3.0,
        },
    )
    .unwrap();
    assert!(r.clamped, "beyond the ore ceiling");
    assert!(r.shortfalls.is_empty(), "clamped solve: {:?}", r.shortfalls);
}

#[test]
fn t1_shortfall_never_taken_to_save_machines() {
    // Lexicographic guard: at T=20 the chain costs hundreds of machine
    // equivalents; zeroing the target would save all of them, but the
    // shortfall penalty must dominate the machines term.
    let snap = modular_frame_snapshot(20.0, None, 780.0);
    let r = solver::t1::solve(&snap, &T0Edit::Recompute).unwrap();
    assert!(
        r.shortfalls.is_empty(),
        "shortfall taken: {:?}",
        r.shortfalls
    );
    assert_golden_rates(&r, 20.0);
}

#[test]
fn t1_degraded_solve_stays_in_budget() {
    // Three outputs, two degraded (one starved by a ceiling, one unwired):
    // the elastic formulation must stay inside the 50ms T1 budget.
    let mut snap = dual_chain_snapshot(30.0, 30.0, Some(10.0));
    snap.outputs.push(OutputPortSpec {
        id: "out-z".into(),
        item: "z".into(),
        rate: 15.0,
    });
    let r = solver::t1::solve(&snap, &T0Edit::Recompute).unwrap();
    assert_eq!(r.shortfalls.len(), 2);
    match &r.shortfalls["out-z"].binding {
        Some(Constraint::Disconnected { node, .. }) => assert_eq!(node, "out-z"),
        other => panic!("expected disconnected binding, got {other:?}"),
    }
    assert!(
        r.solve_us < 50_000,
        "degraded solve blew the budget: {}us",
        r.solve_us
    );
}

/// One refinery-style group with a TWO-OUTPUT recipe: 3 crude → 2 plastic +
/// 1 residue per 60s cycle (per machine-minute: 3 crude in, 2 plastic +
/// 1 residue out). Group cycles are max(plastic/2, residue) — the piecewise-
/// linear case the old two-point ceiling probe got wrong in both directions.
/// Residue is a fixed sibling target; plastic is the edited port. The single
/// crude input deliberately never mixes open and ceilinged sources on one
/// item, keeping this fixture independent of `pull`'s demand-split weights.
fn refinery_snapshot(
    residue_target: f64,
    crude_ceiling: Option<f64>,
    crude_belt_cap: f64,
) -> FactorySnapshot {
    FactorySnapshot {
        groups: vec![group(
            "refinery",
            recipe(
                "Recipe_Plastic_C",
                "refinery",
                60.0,
                &[("crude", 3.0)],
                &[("plastic", 2.0), ("residue", 1.0)],
                30.0,
            ),
        )],
        edges: vec![
            edge(
                "e-crude",
                NodeRef::Input("in-crude".into()),
                g("refinery"),
                "crude",
                crude_belt_cap,
            ),
            edge(
                "e-plastic",
                g("refinery"),
                NodeRef::Output("out-plastic".into()),
                "plastic",
                780.0,
            ),
            edge(
                "e-residue",
                g("refinery"),
                NodeRef::Output("out-residue".into()),
                "residue",
                780.0,
            ),
        ],
        inputs: vec![InputPortSpec {
            id: "in-crude".into(),
            item: "crude".into(),
            ceiling: crude_ceiling,
        }],
        junctions: vec![],
        outputs: vec![
            OutputPortSpec {
                id: "out-plastic".into(),
                item: "plastic".into(),
                rate: 0.0,
            },
            OutputPortSpec {
                id: "out-residue".into(),
                item: "residue".into(),
                rate: residue_target,
            },
        ],
    }
}

fn set_plastic(rate: f64) -> T0Edit {
    T0Edit::SetTarget {
        port: "out-plastic".into(),
        rate,
    }
}

#[test]
fn t0_multi_output_clamps_at_true_ceiling_and_names_input() {
    // Crude capped at 300/min, 3 crude per cycle → 100 cycles → 200 plastic.
    // On [0,1] the residue target (8) drives cycles, so crude shows zero
    // sensitivity to the plastic target: the old affine probe saw no crude
    // ceiling at all, named the 780 plastic out-belt, and never clamped.
    let snap = refinery_snapshot(8.0, Some(300.0), 780.0);
    let r = solver::t0::solve(&snap, &set_plastic(500.0)).unwrap();
    assert!(r.clamped, "target beyond the crude ceiling must clamp");
    let ceiling = r.target_ceiling.clone().expect("ceiling must be reported");
    assert_close(ceiling.max_rate, 200.0, "true multi-output ceiling");
    match ceiling.binding {
        Constraint::InputCeiling { ref port, .. } => assert_eq!(port, "in-crude"),
        ref other => panic!("expected input-ceiling binding, got {other:?}"),
    }
    assert_close(r.ports["out-plastic"], 200.0, "clamped target");
    assert_close(r.ports["in-crude"], 300.0, "crude saturated at the ceiling");
    for (id, e) in &r.edges {
        assert!(
            e.saturation <= 1.0 + 1e-6,
            "edge {id} over capacity: saturation {}",
            e.saturation
        );
    }
}

#[test]
fn t0_multi_output_belt_binding_beyond_kink() {
    // Open crude input, 240/min crude belt: 3·max(T/2, 8) ≤ 240 → T = 160.
    // The belt's [0,1] sensitivity is zero (residue drives cycles until
    // T=16), so the old probe left a request of 500 unclamped at 312% belt
    // saturation. The binding must name the mid-chain belt, not the ceiling.
    let snap = refinery_snapshot(8.0, None, 240.0);
    let r = solver::t0::solve(&snap, &set_plastic(500.0)).unwrap();
    assert!(r.clamped, "target beyond the crude belt must clamp");
    let ceiling = r.target_ceiling.clone().expect("ceiling must be reported");
    assert_close(ceiling.max_rate, 160.0, "belt-bound ceiling");
    match ceiling.binding {
        Constraint::BeltCapacity { ref edge, .. } => assert_eq!(edge, "e-crude"),
        ref other => panic!("expected belt binding, got {other:?}"),
    }
    assert_close(r.edges["e-crude"].saturation, 1.0, "binding belt saturated");
}

#[test]
fn t0_multi_output_ceiling_reported_unclamped() {
    // A request below the true ceiling must not clamp, but the slider tick
    // still needs the honest hard-stop: max_rate 200 named at the crude
    // ceiling (the old probe reported 780 at the plastic out-belt here).
    let snap = refinery_snapshot(8.0, Some(300.0), 780.0);
    let r = solver::t0::solve(&snap, &set_plastic(150.0)).unwrap();
    assert!(!r.clamped, "request below the ceiling must not clamp");
    assert_close(r.ports["out-plastic"], 150.0, "request honored");
    let ceiling = r.target_ceiling.clone().expect("ceiling must be reported");
    assert_close(ceiling.max_rate, 200.0, "unclamped ceiling still exact");
    match ceiling.binding {
        Constraint::InputCeiling { ref port, .. } => assert_eq!(port, "in-crude"),
        ref other => panic!("expected input-ceiling binding, got {other:?}"),
    }
}

#[test]
fn t0_kink_below_one_no_underestimate() {
    // Residue target 0.25 puts the kink at T=0.5; crude ceiling 1.2 →
    // 3·max(T/2, 0.25) ≤ 1.2 → T = 0.8. The old [0,1] chord mixed the flat
    // and sloped segments (crude 0.75 → 1.5) and crossed at 0.6 — clamping
    // 25% below the true ceiling. Guards the opposite error direction.
    let snap = refinery_snapshot(0.25, Some(1.2), 780.0);
    let r = solver::t0::solve(&snap, &set_plastic(5.0)).unwrap();
    assert!(r.clamped);
    let ceiling = r.target_ceiling.clone().expect("ceiling must be reported");
    assert_close(ceiling.max_rate, 0.8, "kink-below-one ceiling");
    match ceiling.binding {
        Constraint::InputCeiling { ref port, .. } => assert_eq!(port, "in-crude"),
        ref other => panic!("expected input-ceiling binding, got {other:?}"),
    }
    assert_close(r.ports["out-plastic"], 0.8, "clamped target");
}

#[test]
fn t0_zero_slope_on_unit_interval_still_clamps() {
    // Residue target 100 pins cycles at 100 until T=200: crude sits flat at
    // 300/min across the whole [0,1] probe window (per-unit slope exactly
    // zero — the "no candidate → no clamp" mechanism). True ceiling:
    // 1.5T ≤ 400 → T = 800/3.
    let snap = refinery_snapshot(100.0, Some(400.0), 780.0);
    let r = solver::t0::solve(&snap, &set_plastic(500.0)).unwrap();
    assert!(r.clamped, "zero [0,1] slope must still find the clamp");
    let ceiling = r.target_ceiling.clone().expect("ceiling must be reported");
    assert_close(ceiling.max_rate, 800.0 / 3.0, "ceiling beyond the kink");
    match ceiling.binding {
        Constraint::InputCeiling { ref port, .. } => assert_eq!(port, "in-crude"),
        ref other => panic!("expected input-ceiling binding, got {other:?}"),
    }
    assert_close(r.ports["in-crude"], 400.0, "crude tight at its ceiling");
}

#[test]
fn t0_multi_output_property_sweep() {
    // For any requested rate: no cap is ever exceeded, the reported ceiling
    // is request-independent, and a clamped solve is tight at its binding.
    let snap = refinery_snapshot(8.0, Some(300.0), 780.0);
    for i in 0..=12 {
        let rate = i as f64 * 50.0; // 0..600, crossing the 200 ceiling
        let r = solver::t0::solve(&snap, &set_plastic(rate)).unwrap();
        for (id, e) in &r.edges {
            assert!(
                e.saturation <= 1.0 + 1e-6,
                "rate {rate}: edge {id} over capacity ({})",
                e.saturation
            );
        }
        assert!(
            r.ports["in-crude"] <= 300.0 + 1e-6,
            "rate {rate}: crude over ceiling ({})",
            r.ports["in-crude"]
        );
        let ceiling = r.target_ceiling.clone().expect("ceiling must be reported");
        assert_close(ceiling.max_rate, 200.0, "request-independent ceiling");
        match ceiling.binding {
            Constraint::InputCeiling { ref port, .. } => assert_eq!(port, "in-crude"),
            ref other => panic!("expected input-ceiling binding, got {other:?}"),
        }
        if rate > 200.0 {
            assert!(r.clamped, "rate {rate} beyond the ceiling must clamp");
            assert_close(r.ports["in-crude"], 300.0, "tight at the binding");
        } else {
            assert!(!r.clamped, "rate {rate} within the ceiling must not clamp");
            assert_close(r.ports["out-plastic"], rate, "request honored");
        }
    }
}

#[test]
fn t0_t1_ceiling_parity_multi_output() {
    // Both tiers must agree on the hard-stop rate and its name for the
    // multi-output fixture. Only target_ceiling.max_rate and the binding
    // kind/id are compared: the fixture has no starved inputs, keeping this
    // robust to T1's elastic-shortfall handling in either landing order.
    let snap = refinery_snapshot(8.0, Some(300.0), 780.0);
    let edit = set_plastic(500.0);
    let a = solver::t0::solve(&snap, &edit).unwrap();
    let b = solver::t1::solve(&snap, &edit).unwrap();
    let ca = a.target_ceiling.clone().expect("t0 ceiling");
    let cb = b.target_ceiling.clone().expect("t1 ceiling");
    assert!(
        (ca.max_rate - cb.max_rate).abs() < 1e-4,
        "ceiling parity: t0={} t1={}",
        ca.max_rate,
        cb.max_rate
    );
    match (&ca.binding, &cb.binding) {
        (Constraint::InputCeiling { port: pa, .. }, Constraint::InputCeiling { port: pb, .. }) => {
            assert_eq!(pa, "in-crude");
            assert_eq!(pa, pb);
        }
        other => panic!("expected matching input-ceiling bindings, got {other:?}"),
    }
    assert_eq!(a.clamped, b.clamped, "both tiers clamp");
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

// ---- generators (power) --------------------------------------------------
// A driven generator (no power output port wired) runs toward nameplate but is
// fuel-limited and YIELDS to real output targets — never a false or
// target-clobbering number, and its slack never enters shortfalls/ports.

fn coal_gen(driven: Option<f64>) -> GroupSpec {
    // 15 coal/min → 75 MW per machine (nameplate 300 MW / 60 coal at 4×).
    let mut gs = group(
        "gen",
        recipe(
            "Recipe_Power_Coal",
            "coalgen",
            60.0,
            &[("coal", 15.0)],
            &[("power", 75.0)],
            0.0,
        ),
    );
    gs.count = 4;
    gs.driven_cycles = driven;
    gs
}

fn coal_only_snapshot(
    driven: Option<f64>,
    coal_ceiling: Option<f64>,
    wired: bool,
) -> FactorySnapshot {
    let edges = if wired {
        vec![edge(
            "e-coal",
            NodeRef::Input("in-coal".into()),
            g("gen"),
            "coal",
            780.0,
        )]
    } else {
        vec![]
    };
    FactorySnapshot {
        groups: vec![coal_gen(driven)],
        edges,
        inputs: vec![InputPortSpec {
            id: "in-coal".into(),
            item: "coal".into(),
            ceiling: coal_ceiling,
        }],
        junctions: vec![],
        outputs: vec![],
    }
}

#[test]
fn driven_generator_runs_at_nameplate_when_fueled() {
    let snap = coal_only_snapshot(Some(4.0), None, true);
    let r = solver::t1::solve(&snap, &T0Edit::Recompute).unwrap();
    assert_close(
        r.groups["gen"].out_rates["power"],
        300.0,
        "fueled generation",
    );
    assert_close(r.groups["gen"].in_rates["coal"], 60.0, "fuel draw");
    assert!(
        r.shortfalls.is_empty(),
        "generator slack must NOT leak into shortfalls"
    );
}

#[test]
fn driven_generator_scales_down_when_fuel_capped() {
    let snap = coal_only_snapshot(Some(4.0), Some(30.0), true); // 30 coal, needs 60
    let r = solver::t1::solve(&snap, &T0Edit::Recompute).unwrap();
    assert_close(
        r.groups["gen"].out_rates["power"],
        150.0,
        "fuel-limited generation",
    );
    assert!(
        r.shortfalls.is_empty(),
        "fuel-limited generator is not a shortfall"
    );
}

#[test]
fn driven_generator_zero_when_unfueled() {
    let snap = coal_only_snapshot(Some(4.0), None, false);
    let r = solver::t1::solve(&snap, &T0Edit::Recompute).unwrap();
    assert_close(r.groups["gen"].out_rates["power"], 0.0, "no false power");
    assert!(
        r.shortfalls.is_empty(),
        "unfueled generator is not a shortfall"
    );
}

#[test]
fn real_target_wins_the_fuel_fight_against_a_generator() {
    // One capped coal input (60/min) feeds BOTH a coal generator (driven, wants
    // 60 coal for 300 MW) and a "coke" line whose product exits a real OUT port
    // targeted at 60/min (needs all 60 coal). The real target must win; the
    // generator takes the leftover (0), never clobbering the user's target.
    let coke = {
        let mut gs = group(
            "coke",
            recipe(
                "Recipe_Coke",
                "refinery",
                60.0,
                &[("coal", 60.0)],
                &[("coke", 60.0)],
                0.0,
            ),
        );
        gs.count = 1;
        gs
    };
    let snap = FactorySnapshot {
        groups: vec![coal_gen(Some(4.0)), coke],
        edges: vec![
            edge(
                "e-coal-gen",
                NodeRef::Input("in-coal".into()),
                g("gen"),
                "coal",
                780.0,
            ),
            edge(
                "e-coal-coke",
                NodeRef::Input("in-coal".into()),
                g("coke"),
                "coal",
                780.0,
            ),
            edge(
                "e-coke-out",
                g("coke"),
                NodeRef::Output("out-coke".into()),
                "coke",
                780.0,
            ),
        ],
        inputs: vec![InputPortSpec {
            id: "in-coal".into(),
            item: "coal".into(),
            ceiling: Some(60.0),
        }],
        junctions: vec![],
        outputs: vec![OutputPortSpec {
            id: "out-coke".into(),
            item: "coke".into(),
            rate: 60.0,
        }],
    };
    let r = solver::t1::solve(&snap, &T0Edit::Recompute).unwrap();
    assert_close(r.ports["out-coke"], 60.0, "real coke target met in full");
    assert!(r.shortfalls.is_empty(), "no shortfall on the real target");
    assert_close(
        r.groups["gen"].out_rates["power"],
        0.0,
        "generator took only leftover coal",
    );
}

#[test]
fn driven_generator_does_not_clobber_an_edited_targets_ceiling() {
    // Same shared-fuel topology, but the user DRAGS the coke target (the ceiling
    // pass). The generator must yield the contested coal so the edited port's
    // achievable ceiling reads 60 — not ~0 from the generator winning the
    // maximize pass (regression: gen_penalty used to leak into the ceiling pass).
    let coke = {
        let mut gs = group(
            "coke",
            recipe(
                "Recipe_Coke",
                "refinery",
                60.0,
                &[("coal", 60.0)],
                &[("coke", 60.0)],
                0.0,
            ),
        );
        gs.count = 1;
        gs
    };
    let snap = FactorySnapshot {
        groups: vec![coal_gen(Some(4.0)), coke],
        edges: vec![
            edge(
                "e-coal-gen",
                NodeRef::Input("in-coal".into()),
                g("gen"),
                "coal",
                780.0,
            ),
            edge(
                "e-coal-coke",
                NodeRef::Input("in-coal".into()),
                g("coke"),
                "coal",
                780.0,
            ),
            edge(
                "e-coke-out",
                g("coke"),
                NodeRef::Output("out-coke".into()),
                "coke",
                780.0,
            ),
        ],
        inputs: vec![InputPortSpec {
            id: "in-coal".into(),
            item: "coal".into(),
            ceiling: Some(60.0),
        }],
        junctions: vec![],
        outputs: vec![OutputPortSpec {
            id: "out-coke".into(),
            item: "coke".into(),
            rate: 0.0,
        }],
    };
    let r = solver::t1::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-coke".into(),
            rate: 60.0,
        },
    )
    .unwrap();
    assert_close(
        r.ports["out-coke"],
        60.0,
        "edited target reaches its true ceiling",
    );
    assert!(
        !r.clamped,
        "60/min is feasible — must not be clamped to the generator"
    );
    if let Some(tc) = &r.target_ceiling {
        assert_close(tc.max_rate, 60.0, "ceiling is the full coal-limited rate");
    }
}

/// #118 regression topology: MAKE pools one raw from SEVERAL claims — two
/// same-item input ports feeding one group in parallel (directly here; via a
/// merger in the real wiring, which junction conservation makes equivalent).
/// Port A is a capped claim; port B is open supply (no ceiling).
fn pooled_inputs_snapshot(target: f64) -> FactorySnapshot {
    FactorySnapshot {
        groups: vec![group(
            "smelt",
            recipe(
                "Recipe_IngotIron_C",
                "smelter",
                2.0,
                &[("ore", 1.0)],
                &[("ingot", 1.0)],
                4.0,
            ),
        )],
        edges: vec![
            edge(
                "e-a",
                NodeRef::Input("in-a".into()),
                g("smelt"),
                "ore",
                60.0,
            ),
            edge(
                "e-b",
                NodeRef::Input("in-b".into()),
                g("smelt"),
                "ore",
                60.0,
            ),
            edge(
                "e-out",
                g("smelt"),
                NodeRef::Output("out-ingot".into()),
                "ingot",
                780.0,
            ),
        ],
        inputs: vec![
            InputPortSpec {
                id: "in-a".into(),
                item: "ore".into(),
                ceiling: Some(60.0),
            },
            InputPortSpec {
                id: "in-b".into(),
                item: "ore".into(),
                ceiling: None,
            },
        ],
        junctions: vec![],
        outputs: vec![OutputPortSpec {
            id: "out-ingot".into(),
            item: "ingot".into(),
            rate: target,
        }],
    }
}

#[test]
fn t0_weights_open_input_by_its_belt_not_zero() {
    // Before the fix a ceiling-less port weighed 0.0, so the whole 100/min
    // pull routed through capped port A and clamped the preview at 60/min
    // while B sat unused. Weighted min(ceiling, belt) / belt-for-open, the
    // pull splits 50/50 — feasible, both ports working, target met.
    let snap = pooled_inputs_snapshot(0.0);
    let r = solver::t0::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-ingot".into(),
            rate: 100.0,
        },
    )
    .unwrap();
    assert!(
        !r.clamped,
        "100/min fits the pooled supply (60 capped + open)"
    );
    assert_close(r.edges["e-a"].flow, 50.0, "capped-port share");
    assert_close(r.edges["e-b"].flow, 50.0, "open-port share");
    assert_close(r.ports["out-ingot"], 100.0, "target met through the pool");
}

/// Audit #123 regression: a sibling output whose FIXED target already violates
/// its own input ceiling must not zero the EDITED chain's preview. Before the
/// fix, the t=0 base probe saw in-b over its ceiling (out-y=30 > 10), declared
/// the whole edit infeasible, clamped out-x to 0 and named in-b's constraint.
#[test]
fn t0_sibling_infeasibility_does_not_zero_independent_chain() {
    // out-y fixed at 30 with in-b capped at 10 — a standing sibling violation.
    let snap = dual_chain_snapshot(0.0, 30.0, Some(10.0));
    let r = solver::t0::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-x".into(),
            rate: 50.0,
        },
    )
    .unwrap();
    // The independent chain runs at the requested rate…
    assert_close(r.ports["out-x"], 50.0, "edited chain at its requested rate");
    assert!(!r.clamped, "50/min is far below chain A's own belt caps");
    // …and any reported ceiling belongs to chain A: structurally the only
    // caps out-x can push are its own belts, so assert the binding IS one of
    // them (a positive check — an InputCeiling(in-b) binding would fail here).
    if let Some(c) = &r.target_ceiling {
        assert!(c.max_rate > 50.0, "own ceiling is chain A's belt, not 0");
        assert!(
            matches!(&c.binding, Constraint::BeltCapacity { edge, .. } if edge == "e-a" || edge == "e-x"),
            "binding must be chain A's own belt, got {:?}",
            c.binding
        );
    }
    // The sibling's demanded flow is untouched by the edit (T0 shows demand;
    // the violated in-b ceiling surfaces as saturation > 1, and T1 owns the
    // shortfall story on release).
    assert_close(
        r.edges["e-b"].flow,
        30.0,
        "sibling demand unchanged by the edit",
    );
}

/// Same standing sibling violation, but the edited chain's OWN input is now
/// capped tighter than its belts: the clamp lands on in-a's ceiling — an
/// InputCeiling binding naming chain A's port, never the sibling's violated
/// in-b. This makes the anti-blame guard live on the InputCeiling arm (the
/// dual-chain test above can only ever bind on chain A's belts).
#[test]
fn t0_sibling_infeasibility_blames_own_input_ceiling_only() {
    let mut snap = dual_chain_snapshot(0.0, 30.0, Some(10.0));
    snap.inputs[0].ceiling = Some(40.0); // cap chain A's own input below its 780 belts
    let r = solver::t0::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-x".into(),
            rate: 10_000.0,
        },
    )
    .unwrap();
    assert!(r.clamped, "10k/min exceeds in-a's 40 ceiling");
    let c = r.target_ceiling.as_ref().expect("ceiling reported");
    assert_close(c.max_rate, 40.0, "clamped at chain A's own input ceiling");
    match &c.binding {
        Constraint::InputCeiling { port, .. } => {
            assert_eq!(port, "in-a", "own ceiling named, never the sibling's in-b")
        }
        other => panic!("expected InputCeiling binding, got {other:?}"),
    }
}

/// PR #47 review follow-up: `driven_cycles` must be honored by T0 ITSELF, not
/// only T1 — an un-wired generator holds its nameplate cycles in the drag
/// preview (its power output is demanded by nothing, so demand alone would
/// idle it to 0 and every drag frame would read "GENERATES 0 MW").
#[test]
fn t0_driven_generator_holds_nameplate_in_preview() {
    // Dual-chain graph plus an un-wired coal generator: 3 machine-equivalents,
    // burn recipe 15 coal -> 75 "__PowerMW" per cycle-minute (duration 60s).
    let mut snap = dual_chain_snapshot(15.0, 0.0, None);
    snap.groups.push(GroupSpec {
        driven_cycles: Some(3.0),
        ..group(
            "gen",
            recipe(
                "Recipe_Power_C",
                "generator",
                60.0,
                &[("coal", 15.0)],
                &[("__PowerMW", 75.0)],
                0.0,
            ),
        )
    });
    snap.inputs.push(InputPortSpec {
        id: "in-coal".into(),
        item: "coal".into(),
        ceiling: None,
    });
    snap.edges.push(edge(
        "e-coal",
        NodeRef::Input("in-coal".into()),
        g("gen"),
        "coal",
        780.0,
    ));

    // Plain recompute: nameplate generation and fuel draw.
    let r = solver::t0::solve(&snap, &T0Edit::Recompute).unwrap();
    assert_close(
        r.groups["gen"].out_rates["__PowerMW"],
        225.0,
        "3 machine-equivalents x 75 MW nameplate",
    );
    assert_close(r.edges["e-coal"].flow, 45.0, "fuel pulled at nameplate");

    // Mid-drag on the unrelated chain: the generator stays at nameplate and
    // neither clamps nor perturbs the edited port.
    let r = solver::t0::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-x".into(),
            rate: 50.0,
        },
    )
    .unwrap();
    assert_close(
        r.groups["gen"].out_rates["__PowerMW"],
        225.0,
        "preview keeps nameplate mid-drag",
    );
    assert!(
        !r.clamped,
        "generator fuel does not clamp the unrelated edit"
    );
    assert_close(r.ports["out-x"], 50.0, "edited chain unaffected");
}

/// Same graph, sane siblings: the edited chain's OWN ceiling is still found
/// exactly (belt cap 780 on e-a/e-x → ceiling 780) — the relative-feasibility
/// rework must not change the healthy-baseline path.
#[test]
fn t0_own_ceiling_unchanged_when_siblings_healthy() {
    let snap = dual_chain_snapshot(0.0, 5.0, Some(10.0));
    let r = solver::t0::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-x".into(),
            rate: 10_000.0,
        },
    )
    .unwrap();
    assert!(r.clamped, "10k/min exceeds the 780 belt");
    let c = r.target_ceiling.as_ref().expect("ceiling reported");
    assert_close(c.max_rate, 780.0, "belt-cap ceiling exact");
    assert!(
        matches!(&c.binding, Constraint::BeltCapacity { edge, .. } if edge == "e-a" || edge == "e-x"),
        "binding is chain A's own belt, got {:?}",
        c.binding
    );
}

/// A splitter fanning one feed across IDENTICAL parallel groups must share
/// the demand like the real building does — evenly — instead of the
/// degenerate LP vertex loading one branch and idling its siblings (the
/// observed 60/0/0/0 concentration). Pinned at T1: T0's preview already
/// split proportionally, and the two must agree.
fn parallel_concrete_snapshot(counts: &[u32]) -> FactorySnapshot {
    let j = |id: &str| NodeRef::Junction(id.to_string());
    let mut groups = Vec::new();
    let mut edges = vec![edge(
        "e-in",
        NodeRef::Input("in-stone".into()),
        j("split"),
        "stone",
        780.0,
    )];
    for (i, &count) in counts.iter().enumerate() {
        let id = format!("c{}", i + 1);
        let mut gr = group(
            &id,
            recipe(
                "Recipe_Concrete_C",
                "constructor",
                4.0,
                &[("stone", 3.0)],
                &[("concrete", 1.0)],
                4.0,
            ),
        );
        gr.count = count;
        groups.push(gr);
        edges.push(edge(
            &format!("e-s{}", i + 1),
            j("split"),
            g(&id),
            "stone",
            780.0,
        ));
        edges.push(edge(
            &format!("e-m{}", i + 1),
            g(&id),
            j("merge"),
            "concrete",
            780.0,
        ));
    }
    edges.push(edge(
        "e-out",
        j("merge"),
        NodeRef::Output("out-concrete".into()),
        "concrete",
        780.0,
    ));
    FactorySnapshot {
        groups,
        edges,
        inputs: vec![InputPortSpec {
            id: "in-stone".into(),
            item: "stone".into(),
            ceiling: None,
        }],
        junctions: vec!["split".into(), "merge".into()],
        outputs: vec![OutputPortSpec {
            id: "out-concrete".into(),
            item: "concrete".into(),
            rate: 0.0,
        }],
    }
}

#[test]
fn t1_parallel_identical_groups_split_evenly() {
    let snap = parallel_concrete_snapshot(&[1, 1, 1]);
    let r = solver::t1::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-concrete".into(),
            rate: 45.0,
        },
    )
    .unwrap();
    // 1 concrete / 4s = 15/min per machine → one machine-equivalent each.
    for c in ["c1", "c2", "c3"] {
        assert_close(
            r.groups[c].out_rates["concrete"],
            15.0,
            &format!("{c} even share"),
        );
    }
    // Stone: 3 per concrete → each branch belt carries 45/min.
    for e in ["e-s1", "e-s2", "e-s3"] {
        assert_close(r.edges[e].flow, 45.0, &format!("{e} branch feed"));
    }
    assert_close(r.ports["out-concrete"], 45.0, "target met");
}

#[test]
fn t1_parallel_split_weights_by_group_capacity() {
    // A ×3 bank next to a ×1 sibling takes three shares — capacity-weighted
    // fairness, matching what an even per-machine load means physically.
    let snap = parallel_concrete_snapshot(&[1, 3]);
    let r = solver::t1::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-concrete".into(),
            rate: 60.0,
        },
    )
    .unwrap();
    assert_close(r.groups["c1"].out_rates["concrete"], 15.0, "×1 share");
    assert_close(r.groups["c2"].out_rates["concrete"], 45.0, "×3 share");
    assert_close(r.edges["e-s1"].flow, 45.0, "×1 branch stone");
    assert_close(r.edges["e-s2"].flow, 135.0, "×3 branch stone");
}

#[test]
fn t1_parallel_split_bends_to_belt_caps() {
    // A capped branch takes what its belt allows; the surplus lands on the
    // open sibling — fairness never overrides a real constraint.
    let mut snap = parallel_concrete_snapshot(&[1, 1]);
    for e in &mut snap.edges {
        if e.id == "e-s1" {
            e.capacity = 30.0; // stone feed cap → ≤10/min concrete on c1
        }
    }
    let r = solver::t1::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-concrete".into(),
            rate: 40.0,
        },
    )
    .unwrap();
    assert_close(r.groups["c1"].out_rates["concrete"], 10.0, "capped branch");
    assert_close(r.groups["c2"].out_rates["concrete"], 30.0, "open branch");
}

#[test]
fn t1_split_recovers_a_previously_idled_sibling() {
    // Regression: the idle write-back sets a starved group's clock to 0 in
    // the PLAN, and clock-weighted fairness then ejected it from its class —
    // one concentrated solve poisoned every later settle. Weighting by count
    // keeps the sibling in the class, so the next solve redistributes.
    let mut snap = parallel_concrete_snapshot(&[1, 1]);
    snap.groups[1].clock = 0.0; // poisoned by a previous concentrated solve
    let r = solver::t1::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-concrete".into(),
            rate: 30.0,
        },
    )
    .unwrap();
    assert_close(
        r.groups["c1"].out_rates["concrete"],
        15.0,
        "recovered even share c1",
    );
    assert_close(
        r.groups["c2"].out_rates["concrete"],
        15.0,
        "recovered even share c2",
    );
}

#[test]
fn t1_throttled_sibling_does_not_disable_fairness() {
    // Review finding on the first fairness cut (single per-class max-min u):
    // one belt-throttled sibling pinned the class floor and the two healthy
    // branches above it were degenerate again — concentration recurred among
    // them. The concave water-filling reward equalizes the REMAINING pair
    // after the capped branch saturates.
    let mut snap = parallel_concrete_snapshot(&[1, 1, 1]);
    for e in &mut snap.edges {
        if e.id == "e-s1" {
            e.capacity = 9.0; // stone feed cap → ≤3/min concrete on c1
        }
    }
    let r = solver::t1::solve(
        &snap,
        &T0Edit::SetTarget {
            port: "out-concrete".into(),
            rate: 33.0,
        },
    )
    .unwrap();
    assert_close(
        r.groups["c1"].out_rates["concrete"],
        3.0,
        "capped branch saturates",
    );
    assert_close(
        r.groups["c2"].out_rates["concrete"],
        15.0,
        "healthy pair splits evenly (c2)",
    );
    assert_close(
        r.groups["c3"].out_rates["concrete"],
        15.0,
        "healthy pair splits evenly (c3)",
    );
}
