//! Phase 3 exit criterion at the core level: "plan a supply chain" produces a
//! reviewable, partially-acceptable proposal — accept materializes ◇ planned
//! entities in ONE undo step; exclusions cascade and recompute consequences.

use std::sync::atomic::AtomicBool;

use app::wizard::{global_solve, WizardGoal, WizardOutcome};
use app::Session;
use planner_core::commands::Command;
use planner_core::entities::*;
use planner_core::proposals::ProposalStatus;

fn gp(x: f64, y: f64) -> GraphPos {
    GraphPos { x, y }
}

/// An empire with ingot surplus and a coal grid — the wizard should reuse
/// both instead of proposing redundant production.
fn build_base(s: &mut Session) -> (Id, Id) {
    let works = s
        .edit(vec![Command::CreateFactory {
            name: "OLD INGOT WORKS".into(),
            position: MapPos {
                x: -1400.0,
                y: 2400.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
        .clone();
    s.edit(vec![Command::ClaimNode {
        factory: works.clone(),
        node: "iron-gf-01".into(),
        extractor: "Build_MinerMk2_C".into(),
        clock: 1.0,
    }])
    .unwrap();
    let ore_in = s
        .edit(vec![Command::AddPort {
            factory: works.clone(),
            direction: PortDirection::In,
            item: "Desc_OreIron_C".into(),
            rate: 0.0,
            rate_ceiling: Some(120.0),
            graph_pos: gp(0.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let ingot_out = s
        .edit(vec![Command::AddPort {
            factory: works.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronIngot_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: gp(600.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let smelt = s
        .edit(vec![Command::AddGroup {
            factory: works.clone(),
            machine: "Build_SmelterMk1_C".into(),
            recipe: "Recipe_IngotIron_C".into(),
            count: 1,
            clock: 1.0,
            graph_pos: gp(300.0, 100.0),
            floor: 0,
        }])
        .unwrap()
        .created[0]
        .clone();
    for (from, to, item) in [
        (
            EdgeEnd::Port(ore_in),
            EdgeEnd::Group(smelt.clone()),
            "Desc_OreIron_C",
        ),
        (
            EdgeEnd::Group(smelt),
            EdgeEnd::Port(ingot_out.clone()),
            "Desc_IronIngot_C",
        ),
    ] {
        s.edit(vec![Command::AddEdge {
            factory: works.clone(),
            from,
            to,
            item: item.into(),
            tier: 3,
        }])
        .unwrap();
    }
    // 60/min surplus, unbound
    s.edit(vec![Command::SetPortRate {
        id: ingot_out,
        rate: 60.0,
    }])
    .unwrap();

    // coal grid: 150 MW
    let plant = s
        .edit(vec![Command::CreateFactory {
            name: "COAL PLANT".into(),
            position: MapPos {
                x: 180.0,
                y: 1050.0,
                z: 0.0,
            },
            region: "GRASS FIELDS".into(),
        }])
        .unwrap()
        .created[0]
        .clone();
    s.edit(vec![Command::ClaimNode {
        factory: plant.clone(),
        node: "coal-gf-01".into(),
        extractor: "Build_MinerMk2_C".into(),
        clock: 1.0,
    }])
    .unwrap();
    let coal_in = s
        .edit(vec![Command::AddPort {
            factory: plant.clone(),
            direction: PortDirection::In,
            item: "Desc_Coal_C".into(),
            rate: 0.0,
            rate_ceiling: Some(120.0),
            graph_pos: gp(0.0, 100.0),
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
            graph_pos: gp(600.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let burn = s
        .gamedata
        .recipes
        .values()
        .find(|r| r.produced_in.contains(&"Build_GeneratorCoal_C".to_string()))
        .unwrap()
        .class_name
        .clone();
    let gens = s
        .edit(vec![Command::AddGroup {
            factory: plant.clone(),
            machine: "Build_GeneratorCoal_C".into(),
            recipe: burn,
            count: 1,
            clock: 1.0,
            graph_pos: gp(300.0, 100.0),
            floor: 0,
        }])
        .unwrap()
        .created[0]
        .clone();
    for (from, to, item) in [
        (
            EdgeEnd::Port(coal_in),
            EdgeEnd::Group(gens.clone()),
            "Desc_Coal_C",
        ),
        (
            EdgeEnd::Group(gens),
            EdgeEnd::Port(mw_out.clone()),
            gamedata::docs::POWER_ITEM,
        ),
    ] {
        s.edit(vec![Command::AddEdge {
            factory: plant.clone(),
            from,
            to,
            item: item.into(),
            tier: 6,
        }])
        .unwrap();
    }
    s.edit(vec![Command::SetPortRate {
        id: mw_out,
        rate: 150.0,
    }])
    .unwrap();
    (works, plant)
}

fn solve(s: &mut Session, goal: WizardGoal) -> WizardOutcome {
    let cancel = AtomicBool::new(false);
    let mut log_lines = Vec::new();
    let out = global_solve(
        &s.state,
        &s.gamedata,
        &s.world,
        &goal,
        s.plan_hash(),
        "2026-07-10T00:00:00Z".into(),
        |phase, line| log_lines.push(format!("{phase}: {line}")),
        &cancel,
    );
    assert!(!log_lines.is_empty(), "solver streams a real log");
    out
}

#[test]
fn wizard_produces_reviewable_partially_acceptable_proposal() {
    let mut s = Session::in_memory(None).unwrap();
    build_base(&mut s);
    let factories_before = s.state.factories.len();

    // goal: 30 iron rods/min — ingots should come from the existing surplus
    let outcome = solve(
        &mut s,
        WizardGoal {
            items: vec![("Desc_IronRod_C".into(), 30.0)],
            constraints: Default::default(),
        },
    );
    let WizardOutcome::Proposal { proposal } = outcome else {
        panic!("expected a proposal, got {outcome:?}");
    };

    // surplus-first: no new smelters, no ore claims — one rod stage + routes
    assert!(
        proposal
            .items
            .iter()
            .any(|i| i.detail.contains("from surplus")),
        "surplus route proposed"
    );
    assert!(
        !proposal
            .items
            .iter()
            .any(|i| i.label.contains("CLAIM iron")),
        "no redundant ore claims: {:?}",
        proposal.items.iter().map(|i| &i.label).collect::<Vec<_>>()
    );
    // A2.4: power is sourced — generator expansion + power line
    assert!(
        proposal.items.iter().any(|i| i.label.contains("EXPAND")),
        "gen expansion"
    );
    assert!(
        proposal.items.iter().any(|i| i.label.contains('⚡')),
        "power line"
    );

    // store it (Draft) — the plan itself is untouched
    let r = s
        .edit(vec![Command::CreateProposal {
            proposal: proposal.clone(),
        }])
        .unwrap();
    let pid = r.created[0].clone();
    assert_eq!(
        s.state.factories.len(),
        factories_before,
        "drafting mutates nothing"
    );
    assert_eq!(s.state.proposals[&pid].number, 1);

    // live consequence with everything included: goal met, power delta positive
    let cons = s.eval_proposal(&pid).unwrap();
    assert!(cons.goal_met, "goal check: {:?}", cons.goal);
    assert!(cons.delta_power_mw > 0.0);
    assert!(cons.machines >= 1);

    // exclude the CREATE item → dependents cascade off, goal no longer met
    let create_item = proposal
        .items
        .iter()
        .find(|i| matches!(i.kind, planner_core::proposals::ProposalItemKind::Create))
        .unwrap()
        .id
        .clone();
    s.edit(vec![Command::ToggleProposalItem {
        proposal: pid.clone(),
        item: create_item.clone(),
        included: false,
    }])
    .unwrap();
    let p = &s.state.proposals[&pid];
    assert!(
        p.items
            .iter()
            .filter(|i| !i.depends_on.is_empty())
            .all(|i| !i.included),
        "dependents cascaded"
    );
    let cons = s.eval_proposal(&pid).unwrap();
    assert!(!cons.goal_met, "excluding the site breaks the goal");

    // re-include by checking the dependent rows — including pulls their
    // dependencies (the CREATE site, the gen expansion) back in with them
    let dependents: Vec<Id> = proposal
        .items
        .iter()
        .filter(|i| !i.depends_on.is_empty())
        .map(|i| i.id.clone())
        .collect();
    for item in dependents {
        s.edit(vec![Command::ToggleProposalItem {
            proposal: pid.clone(),
            item,
            included: true,
        }])
        .unwrap();
    }
    assert!(
        s.state.proposals[&pid].items.iter().all(|i| i.included),
        "deps pulled back in"
    );
    let resp = s.accept_proposal(&pid).unwrap();
    assert_eq!(s.state.proposals[&pid].status, ProposalStatus::Accepted);
    assert_eq!(
        s.state.factories.len(),
        factories_before + 1,
        "new site exists"
    );
    assert!(
        s.state
            .factories
            .values()
            .all(|f| f.status == Status::Planned),
        "planned entities only"
    );
    assert!(
        s.state.routes.values().count() >= 2,
        "surplus belt + power line materialized; routes={:?} items={:?}",
        s.state
            .routes
            .values()
            .map(|r| (&r.kind, &r.endpoints))
            .collect::<Vec<_>>(),
        s.state.proposals[&pid]
            .items
            .iter()
            .map(|i| (&i.label, i.included))
            .collect::<Vec<_>>()
    );
    // the new site actually produces the goal after empire solve
    let rods: f64 = s
        .state
        .ports
        .values()
        .filter(|p| p.direction == PortDirection::Out && p.item == "Desc_IronRod_C")
        .filter_map(|p| {
            resp.derived
                .factories
                .get(&p.factory)
                .and_then(|df| df.ports.get(&p.id))
        })
        .sum();
    assert!((rods - 30.0).abs() < 1e-4, "rods: {rods}");

    // undo of accept removes everything it created and reopens the review
    s.undo().unwrap().unwrap();
    assert_eq!(
        s.state.factories.len(),
        factories_before,
        "undo removes the site"
    );
    assert_ne!(s.state.proposals[&pid].status, ProposalStatus::Accepted);
    s.redo().unwrap().unwrap();
    assert_eq!(s.state.factories.len(), factories_before + 1);
    assert_eq!(s.state.proposals[&pid].status, ProposalStatus::Accepted);
}

#[test]
fn infeasible_returns_best_achievable_and_relaxations() {
    let mut s = Session::in_memory(None).unwrap();
    // no base factories: everything must be new, but zero node budget
    let outcome = solve(
        &mut s,
        WizardGoal {
            items: vec![("Desc_IronRod_C".into(), 30.0)],
            constraints: app::wizard::WizardConstraints {
                node_budget: 0,
                ..Default::default()
            },
        },
    );
    let WizardOutcome::Infeasible(inf) = outcome else {
        panic!("expected infeasible, got {outcome:?}");
    };
    assert!(
        inf.binding.contains("node budget"),
        "binding: {}",
        inf.binding
    );
    assert!(!inf.relaxations.is_empty(), "one-tap relaxations offered");
    assert!(inf.best_rate < 30.0);
}

#[test]
fn plan_hash_flags_staleness() {
    let mut s = Session::in_memory(None).unwrap();
    build_base(&mut s);
    let h1 = s.plan_hash();
    // storing a proposal does NOT change the hash (or it would self-stale)
    let outcome = solve(
        &mut s,
        WizardGoal {
            items: vec![("Desc_IronRod_C".into(), 10.0)],
            constraints: Default::default(),
        },
    );
    let WizardOutcome::Proposal { proposal } = outcome else {
        panic!()
    };
    s.edit(vec![Command::CreateProposal { proposal }]).unwrap();
    assert_eq!(s.plan_hash(), h1, "proposals are excluded from the hash");
    // a real plan edit flips it → STALE badge territory
    s.edit(vec![Command::CreateFactory {
        name: "X".into(),
        position: MapPos {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        region: "GRASS FIELDS".into(),
    }])
    .unwrap();
    assert_ne!(s.plan_hash(), h1);
}

#[test]
fn t2_suggests_cast_screw_mini_proposal() {
    let mut s = Session::in_memory(None).unwrap();
    // factory: ingot in-port → rods → screws (standard recipe, 40/min/machine)
    let f = s
        .edit(vec![Command::CreateFactory {
            name: "SCREW SHOP".into(),
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
    let ingot_in = s
        .edit(vec![Command::AddPort {
            factory: f.clone(),
            direction: PortDirection::In,
            item: "Desc_IronIngot_C".into(),
            rate: 0.0,
            rate_ceiling: Some(120.0),
            graph_pos: gp(0.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let screw_out = s
        .edit(vec![Command::AddPort {
            factory: f.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronScrew_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: gp(900.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let rods = s
        .edit(vec![Command::AddGroup {
            factory: f.clone(),
            machine: "Build_ConstructorMk1_C".into(),
            recipe: "Recipe_IronRod_C".into(),
            count: 1,
            clock: 1.0,
            graph_pos: gp(300.0, 100.0),
            floor: 0,
        }])
        .unwrap()
        .created[0]
        .clone();
    let screws = s
        .edit(vec![Command::AddGroup {
            factory: f.clone(),
            machine: "Build_ConstructorMk1_C".into(),
            recipe: "Recipe_Screw_C".into(),
            count: 1,
            clock: 1.0,
            graph_pos: gp(600.0, 100.0),
            floor: 0,
        }])
        .unwrap()
        .created[0]
        .clone();
    for (from, to, item) in [
        (
            EdgeEnd::Port(ingot_in),
            EdgeEnd::Group(rods.clone()),
            "Desc_IronIngot_C",
        ),
        (
            EdgeEnd::Group(rods.clone()),
            EdgeEnd::Group(screws.clone()),
            "Desc_IronRod_C",
        ),
        (
            EdgeEnd::Group(screws.clone()),
            EdgeEnd::Port(screw_out.clone()),
            "Desc_IronScrew_C",
        ),
    ] {
        s.edit(vec![Command::AddEdge {
            factory: f.clone(),
            from,
            to,
            item: item.into(),
            tier: 2,
        }])
        .unwrap();
    }
    s.edit(vec![Command::SetPortRate {
        id: screw_out.clone(),
        rate: 80.0,
    }])
    .unwrap();
    let screws_before = s.state.groups[&screws].count;
    assert_eq!(screws_before, 2, "80/min on 40/min standard = 2 machines");

    // T2: Cast Screw (50/min per machine, ingots already sourceable) wins
    let proposal = app::wizard::t2_optimize(&s.state, &s.gamedata, &f).expect("a mini-proposal");
    assert!(proposal.title.starts_with("OPTIMIZE"));
    let swap = proposal
        .items
        .iter()
        .find(|i| i.label.contains("Cast Screw"))
        .expect("cast screw swap");
    assert!(
        swap.detail.contains("NOT UNLOCKED"),
        "alternate flagged: {}",
        swap.detail
    );

    // accept applies the swap + rewire and the chain still solves to 80/min
    let r = s
        .edit(vec![Command::CreateProposal {
            proposal: proposal.clone(),
        }])
        .unwrap();
    let pid = r.created[0].clone();
    let resp = s.accept_proposal(&pid).unwrap();
    assert_eq!(
        s.state.groups[&screws].recipe, "Recipe_Alternate_Screw_C",
        "recipe swapped"
    );
    // feed now comes straight from the ingot port; rods keep flowing to nothing
    let screw_rate = resp.derived.factories[&f].ports[&screw_out];
    assert!(
        (screw_rate - 80.0).abs() < 1e-4,
        "target preserved: {screw_rate}"
    );
    // fewer machine-equivalents on the screw stage
    let g = &s.state.groups[&screws];
    assert!(
        (g.count as f64 * g.clock) < screws_before as f64 - 1e-6 + 1.0,
        "cheaper stage: ×{} @ {}",
        g.count,
        g.clock
    );
}
