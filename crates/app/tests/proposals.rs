//! Phase 3 exit criterion at the core level: "plan a supply chain" produces a
//! reviewable, partially-acceptable proposal — accept materializes ◇ planned
//! entities in ONE undo step; exclusions cascade and recompute consequences.

use std::sync::atomic::AtomicBool;

use app::wizard::{global_solve, WizardGoal, WizardOutcome};
use app::Session;
use planner_core::commands::Command;
use planner_core::entities::*;
use planner_core::proposals::{
    Milestone, Proposal, ProposalItem, ProposalItemKind, ProposalSource, ProposalStatus,
};

fn gp(x: f64, y: f64) -> GraphPos {
    GraphPos { x, y }
}

fn pos(x: f64, y: f64) -> MapPos {
    MapPos { x, y, z: 0.0 }
}

/// Force a plant's POWER_ITEM out port to a fixed generation target.
fn set_generation(s: &mut Session, plant: &Id, mw: f64) {
    let port = s
        .state
        .ports
        .values()
        .find(|p| {
            p.factory == *plant
                && p.direction == PortDirection::Out
                && p.item == gamedata::docs::POWER_ITEM
        })
        .expect("plant has a power out port")
        .id
        .clone();
    s.edit(vec![Command::SetPortRate { id: port, rate: mw }])
        .unwrap();
}

/// A factory that DRAWS power: `rod_rate`/min of iron rods on Constructor Mk1
/// (4 MW, 15/min each) → a deterministic `rod_rate / 15 * 4` MW of draw.
fn load_factory(s: &mut Session, name: &str, rod_rate: f64) -> Id {
    let f = s
        .edit(vec![Command::CreateFactory {
            name: name.into(),
            position: pos(900.0, 900.0),
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
            rate_ceiling: Some(1000.0),
            graph_pos: gp(0.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let rod_out = s
        .edit(vec![Command::AddPort {
            factory: f.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronRod_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: gp(600.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let ctors = s
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
    for (from, to, item) in [
        (
            EdgeEnd::Port(ingot_in),
            EdgeEnd::Group(ctors.clone()),
            "Desc_IronIngot_C",
        ),
        (
            EdgeEnd::Group(ctors),
            EdgeEnd::Port(rod_out.clone()),
            "Desc_IronRod_C",
        ),
    ] {
        s.edit(vec![Command::AddEdge {
            factory: f.clone(),
            from,
            to,
            item: item.into(),
            tier: 3,
        }])
        .unwrap();
    }
    s.edit(vec![Command::SetPortRate {
        id: rod_out,
        rate: rod_rate,
    }])
    .unwrap();
    f
}

/// A coal generator producing `mw` MW (fuel is drawn from an uncapped in port,
/// no node claim needed). One generator caps at 75 MW.
fn gen_factory(s: &mut Session, name: &str, mw: f64) -> Id {
    let plant = s
        .edit(vec![Command::CreateFactory {
            name: name.into(),
            position: pos(1500.0, 100.0),
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
            rate_ceiling: Some(1000.0),
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
        rate: mw,
    }])
    .unwrap();
    plant
}

fn bare_factory(s: &mut Session, name: &str) -> Id {
    s.edit(vec![Command::CreateFactory {
        name: name.into(),
        position: pos(200.0, 2000.0),
        region: "GRASS FIELDS".into(),
    }])
    .unwrap()
    .created[0]
        .clone()
}

fn power_route_item(from: &Id, to: &Id) -> ProposalItem {
    ProposalItem {
        id: new_id(),
        kind: ProposalItemKind::RouteAdd,
        included: true,
        label: "⚡ power line".into(),
        detail: "grid tie".into(),
        impact: "power".into(),
        commands: vec![Command::AddRoute {
            kind: RouteKind::Power,
            from: from.clone(),
            to: to.clone(),
            path: vec![pos(0.0, 0.0), pos(10.0, 10.0)],
        }],
        aliases: vec![None],
        depends_on: vec![],
        sync: None,
    }
}

/// Store a Draft proposal made of the given items and return its id.
fn store_proposal(s: &mut Session, items: Vec<ProposalItem>) -> Id {
    let proposal = Proposal {
        id: new_id(),
        source: ProposalSource::GlobalSolver,
        title: "TEST POWER".into(),
        goal: vec![],
        status: ProposalStatus::Draft,
        number: 0,
        snapshot_time: "2026-07-10T00:00:00Z".into(),
        input_hash: s.plan_hash(),
        provenance: "test".into(),
        items,
        milestone: None,
    };
    s.edit(vec![Command::CreateProposal { proposal }])
        .unwrap()
        .created[0]
        .clone()
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
        node: "bp_resourcenode496".into(),
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
        node: "bp_resourcenode600".into(),
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
        &s.unlocked,
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
            milestone: None,
            pinned_recipes: Default::default(),
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

/// Regression: an intermediate demanded by two stages (ingots feed the plate
/// stage directly and the screw stage via rods) is popped from the demand
/// queue twice, tapping the SAME surplus port twice. The wizard must merge
/// the takes into one route item — two `AddRoute`s from one port would make
/// accept always fail with "a port is already bound to a route".
#[test]
fn surplus_port_tapped_by_two_stages_yields_one_route() {
    let mut s = Session::in_memory(None).unwrap();
    build_base(&mut s);
    let ingot_port = s
        .state
        .ports
        .values()
        .find(|p| p.direction == PortDirection::Out && p.item == "Desc_IronIngot_C")
        .unwrap()
        .id
        .clone();

    // RIP 2/min: plates need 18 ingots/min, screws→rods need 6 ingots/min —
    // two separate pops against the single 60/min ingot surplus port
    let outcome = solve(
        &mut s,
        WizardGoal {
            items: vec![("Desc_IronPlateReinforced_C".into(), 2.0)],
            constraints: Default::default(),
            milestone: None,
            pinned_recipes: Default::default(),
        },
    );
    let WizardOutcome::Proposal { proposal } = outcome else {
        panic!("expected a proposal, got {outcome:?}");
    };

    let surplus_rows: Vec<&str> = proposal
        .items
        .iter()
        .filter(|i| i.detail.contains("from surplus"))
        .map(|i| i.detail.as_str())
        .collect();
    assert_eq!(
        surplus_rows.len(),
        1,
        "takes from one port aggregate into one route item: {surplus_rows:?}"
    );

    // the whole point: the proposal must be acceptable end-to-end (a second
    // AddRoute from the bound port used to roll the entire accept back)
    let pid = s
        .edit(vec![Command::CreateProposal { proposal }])
        .unwrap()
        .created[0]
        .clone();
    let resp = s
        .accept_proposal(&pid)
        .expect("aggregated surplus proposal accepts");
    assert_eq!(s.state.proposals[&pid].status, ProposalStatus::Accepted);

    let from_ingot: Vec<&Route> = s
        .state
        .routes
        .values()
        .filter(|r| r.endpoints.0 == ingot_port)
        .collect();
    assert_eq!(
        from_ingot.len(),
        1,
        "exactly one route leaves the surplus port"
    );
    assert_eq!(
        s.state.ports[&ingot_port].bound_route.as_ref(),
        Some(&from_ingot[0].id),
        "the surplus port is bound to the aggregated route"
    );

    // and the new site actually produces the goal after the empire solve
    let rip: f64 = s
        .state
        .ports
        .values()
        .filter(|p| p.direction == PortDirection::Out && p.item == "Desc_IronPlateReinforced_C")
        .filter_map(|p| {
            resp.derived
                .factories
                .get(&p.factory)
                .and_then(|df| df.ports.get(&p.id))
        })
        .sum();
    assert!((rip - 2.0).abs() < 1e-4, "rip: {rip}");
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
            milestone: None,
            pinned_recipes: Default::default(),
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

/// A raw/extractable goal ("produce Iron Ore at 120/min") has no production
/// stage. The wizard must build an extraction-and-ship site — claims feed the
/// in port, which feeds the out port — instead of emitting an edge on a
/// `$g.<item>` alias no AddGroup ever creates (which rolled back every accept).
#[test]
fn raw_goal_builds_extraction_and_ship_site_that_accepts() {
    let mut s = Session::in_memory(None).unwrap();
    let (works, _) = build_base(&mut s);

    let outcome = solve(
        &mut s,
        WizardGoal {
            items: vec![("Desc_OreIron_C".into(), 120.0)],
            constraints: Default::default(),
            milestone: None,
            pinned_recipes: Default::default(),
        },
    );
    let WizardOutcome::Proposal { proposal } = outcome else {
        panic!("expected a proposal, got {outcome:?}");
    };

    let create = proposal
        .items
        .iter()
        .find(|i| matches!(i.kind, planner_core::proposals::ProposalItemKind::Create))
        .expect("create item");
    assert!(
        !create
            .commands
            .iter()
            .any(|c| matches!(c, Command::AddGroup { .. })),
        "a raw goal has no production stages"
    );
    assert!(
        create.commands.iter().any(|c| matches!(
            c,
            Command::AddEdge {
                from: EdgeEnd::Port(f),
                to: EdgeEnd::Port(t),
                ..
            } if f == "$in.Desc_OreIron_C" && t == "$site.out"
        )),
        "pass-through edge wires the raw in port to the out port: {:?}",
        create.commands
    );
    assert!(
        proposal
            .items
            .iter()
            .any(|i| matches!(i.kind, planner_core::proposals::ProposalItemKind::Claim)),
        "extraction claims proposed"
    );
    // the goal is delivered to the existing unbound ore In port
    let ore_in = s
        .state
        .ports
        .values()
        .find(|p| {
            p.factory == works && p.direction == PortDirection::In && p.item == "Desc_OreIron_C"
        })
        .unwrap()
        .id
        .clone();
    assert!(
        proposal.items.iter().any(|i| i
            .commands
            .iter()
            .any(|c| matches!(c, Command::AddRoute { to, .. } if to == &ore_in))),
        "route delivers ore to the existing consumer"
    );

    let pid = s
        .edit(vec![Command::CreateProposal { proposal }])
        .unwrap()
        .created[0]
        .clone();
    let cons = s.eval_proposal(&pid).unwrap();
    assert!(
        !cons.warnings.iter().any(|w| w.contains("skipped")),
        "no unresolved-alias skips: {:?}",
        cons.warnings
    );
    assert!(cons.goal_met, "goal check: {:?}", cons.goal);

    let resp = s.accept_proposal(&pid).expect("raw-goal proposal accepts");
    assert_eq!(s.state.proposals[&pid].status, ProposalStatus::Accepted);
    // the group-less pass-through site actually ships the ore (empire solve)
    let ore: f64 = s
        .state
        .ports
        .values()
        .filter(|p| p.direction == PortDirection::Out && p.item == "Desc_OreIron_C")
        .filter_map(|p| {
            resp.derived
                .factories
                .get(&p.factory)
                .and_then(|df| df.ports.get(&p.id))
        })
        .sum();
    assert!((ore - 120.0).abs() < 1e-4, "shipped ore: {ore}");
}

/// A goal only alternate recipes can produce (alternates off) must return
/// Infeasible naming the fix — not a proposal whose `$g.` alias dangles.
#[test]
fn alternate_only_goal_is_infeasible_naming_alternates() {
    let s = Session::in_memory(None).unwrap();
    let mut gd = s.gamedata.clone();
    gd.recipes
        .remove("Recipe_Screw_C")
        .expect("fixture has the standard screw recipe");

    let cancel = AtomicBool::new(false);
    let mut log_lines = Vec::new();
    let outcome = global_solve(
        &s.state,
        &gd,
        &s.world,
        &WizardGoal {
            items: vec![("Desc_IronScrew_C".into(), 40.0)],
            constraints: Default::default(),
            milestone: None,
            pinned_recipes: Default::default(),
        },
        &s.unlocked,
        s.plan_hash(),
        "2026-07-10T00:00:00Z".into(),
        |phase, line| log_lines.push(format!("{phase}: {line}")),
        &cancel,
    );
    let WizardOutcome::Infeasible(inf) = outcome else {
        panic!("expected infeasible, got {outcome:?}");
    };
    assert_eq!(inf.best_rate, 0.0, "nothing achievable without the recipe");
    assert!(
        inf.binding.to_lowercase().contains("alternate"),
        "binding names alternates: {}",
        inf.binding
    );
    assert!(
        inf.relaxations
            .iter()
            .any(|r| r.to_lowercase().contains("alternate")),
        "one-tap relaxation offered: {:?}",
        inf.relaxations
    );
    assert!(
        log_lines.iter().any(|l| l.contains("INFEASIBLE")),
        "log names the dead end: {log_lines:?}"
    );

    // the relaxation is truthful: alternates on → a real proposal
    let mut log_lines = Vec::new();
    let outcome = global_solve(
        &s.state,
        &gd,
        &s.world,
        &WizardGoal {
            items: vec![("Desc_IronScrew_C".into(), 40.0)],
            constraints: app::wizard::WizardConstraints {
                include_alternates: true,
                ..Default::default()
            },
            milestone: None,
            pinned_recipes: Default::default(),
        },
        &s.unlocked,
        s.plan_hash(),
        "2026-07-10T00:00:00Z".into(),
        |phase, line| log_lines.push(format!("{phase}: {line}")),
        &cancel,
    );
    assert!(
        matches!(outcome, WizardOutcome::Proposal { .. }),
        "alternates on solves: {outcome:?}"
    );
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
            milestone: None,
            pinned_recipes: Default::default(),
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
    let proposal =
        app::wizard::t2_optimize(&s.state, &s.gamedata, &s.unlocked, &f).expect("a mini-proposal");
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

    // W2b: once the save has unlocked that alternate, the flag drops — it is a
    // first-class swap, not a locked suggestion (same swap, different framing).
    s.unlocked.insert("Recipe_Alternate_Screw_C".into());
    let unlocked_prop = app::wizard::t2_optimize(&s.state, &s.gamedata, &s.unlocked, &f)
        .expect("still swaps to the cheaper unlocked alt");
    let unlocked_swap = unlocked_prop
        .items
        .iter()
        .find(|i| i.label.contains("Cast Screw"))
        .expect("cast screw swap");
    assert!(
        !unlocked_swap.detail.contains("NOT UNLOCKED"),
        "an unlocked alt is not flagged: {}",
        unlocked_swap.detail
    );
    s.unlocked.clear(); // restore for the accept-path assertions below

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

/// Piece 1 + 3: a proposal that adds a site and ties it to the grid produces a
/// structured per-circuit impact — before→after draw AND generation — that the
/// review banner renders. Power no longer leaks into the `warnings` strip.
#[test]
fn eval_reports_per_circuit_power_impact_for_a_touched_grid() {
    let mut s = Session::in_memory(None).unwrap();
    build_base(&mut s); // 150 MW coal plant, ungridded until the power line ties in
    let outcome = solve(
        &mut s,
        WizardGoal {
            items: vec![("Desc_IronRod_C".into(), 30.0)],
            constraints: Default::default(),
            milestone: None,
            pinned_recipes: Default::default(),
        },
    );
    let WizardOutcome::Proposal { proposal } = outcome else {
        panic!("expected a proposal");
    };
    let pid = s
        .edit(vec![Command::CreateProposal { proposal }])
        .unwrap()
        .created[0]
        .clone();

    let cons = s.eval_proposal(&pid).unwrap();
    assert_eq!(
        cons.circuit_impacts.len(),
        1,
        "the power line forms one grid → one impact: {:?}",
        cons.circuit_impacts
    );
    let ci = &cons.circuit_impacts[0];
    assert!(
        ci.demand_after_mw > ci.demand_before_mw,
        "the site adds draw: {ci:?}"
    );
    assert!(ci.generation_after_mw > 0.0, "generation surfaced: {ci:?}");
    // headroom_after is exactly the shared formula over the after values
    let expected = (ci.generation_after_mw - ci.demand_after_mw) / ci.generation_after_mw;
    assert!((ci.headroom_after - expected).abs() < 1e-9);
    assert!(
        ci.headroom_after >= 0.20,
        "150 MW plant keeps healthy margin: {ci:?}"
    );
    assert_eq!(ci.level, "ok", "healthy margin reads OK: {ci:?}");
    assert!(
        !cons.warnings.iter().any(|w| w.contains("margin")),
        "power moved out of the warning strip: {:?}",
        cons.warnings
    );
}

/// Piece 1: a grid pushed under 5% headroom flags CRIT — the loud consequence
/// the game hides. A 20 MW plant taking on a 40 MW load browns out.
#[test]
fn eval_flags_a_grid_pushed_under_five_percent_as_crit() {
    let mut s = Session::in_memory(None).unwrap();
    let (_, plant) = build_base(&mut s);
    set_generation(&mut s, &plant, 20.0); // throttle the plant to 20 MW
    let load = load_factory(&mut s, "HEAVY LOAD", 150.0); // 10× constructor = 40 MW

    let pid = store_proposal(&mut s, vec![power_route_item(&plant, &load)]);
    let cons = s.eval_proposal(&pid).unwrap();

    assert_eq!(
        cons.circuit_impacts.len(),
        1,
        "one newly-formed grid: {:?}",
        cons.circuit_impacts
    );
    let ci = &cons.circuit_impacts[0];
    assert!(
        ci.demand_after_mw > ci.generation_after_mw,
        "load overdraws the throttled plant: {ci:?}"
    );
    assert!(ci.headroom_after < 0.05, "under the crit floor: {ci:?}");
    assert_eq!(ci.level, "crit", "browned-out grid reads CRIT: {ci:?}");
}

/// Arbiter decision 1 + Piece 1: a proposal touching two grids yields one
/// impact per TOUCHED grid; a grid the proposal never touches is absent. Grids
/// are matched by member-set overlap, not their index-based names.
#[test]
fn multi_grid_proposal_yields_one_impact_per_touched_grid() {
    let mut s = Session::in_memory(None).unwrap();
    let p1 = gen_factory(&mut s, "PLANT ONE", 50.0);
    let l1 = bare_factory(&mut s, "SUB ONE");
    let p2 = gen_factory(&mut s, "PLANT TWO", 50.0);
    let l2 = bare_factory(&mut s, "SUB TWO");
    let p3 = gen_factory(&mut s, "PLANT THREE", 50.0);
    let l3 = bare_factory(&mut s, "SUB THREE");

    // grid one already exists in the plan — the proposal must never touch it
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Power,
        from: p1.clone(),
        to: l1.clone(),
        path: vec![pos(0.0, 0.0), pos(1.0, 1.0)],
    }])
    .unwrap();

    // the proposal ties two NEW grids (plant two + plant three)
    let pid = store_proposal(
        &mut s,
        vec![power_route_item(&p2, &l2), power_route_item(&p3, &l3)],
    );
    let cons = s.eval_proposal(&pid).unwrap();

    assert_eq!(
        cons.circuit_impacts.len(),
        2,
        "one impact per touched grid; the untouched grid one is absent: {:?}",
        cons.circuit_impacts
    );
    for ci in &cons.circuit_impacts {
        assert!(
            ci.generation_after_mw > 0.0 && ci.demand_before_mw == 0.0,
            "each touched grid is newly formed from zero: {ci:?}"
        );
    }
}

/// A total-quantity goal (milestone) rides through the solver untouched into
/// the Proposal, and survives the JSON persist round-trip — the solve itself
/// never read it (the rate still drives the plan).
#[test]
fn wizard_milestone_carries_into_proposal_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.ficsit");

    let pid;
    {
        let mut s = Session::open(&path, None, "fixture").unwrap();
        build_base(&mut s);

        // "2,500 iron rods" — the game's total goal, planned at 30/min
        let outcome = solve(
            &mut s,
            WizardGoal {
                items: vec![("Desc_IronRod_C".into(), 30.0)],
                constraints: Default::default(),
                milestone: Some(Milestone {
                    item: "Desc_IronRod_C".into(),
                    total: 2500.0,
                    rate: 30.0,
                }),
                pinned_recipes: Default::default(),
            },
        );
        let WizardOutcome::Proposal { proposal } = outcome else {
            panic!("expected a proposal, got {outcome:?}");
        };

        // stamped through global_solve, right item/total/rate
        let m = proposal.milestone.as_ref().expect("milestone stamped");
        assert_eq!(m.item, "Desc_IronRod_C");
        assert_eq!(m.total, 2500.0);
        assert_eq!(m.rate, 30.0);

        pid = s
            .edit(vec![Command::CreateProposal { proposal }])
            .unwrap()
            .created[0]
            .clone();
    }

    // reopen from disk: the milestone survives the proposals(id, json) persist
    {
        let s = Session::open(&path, None, "fixture").unwrap();
        let m = s.state.proposals[&pid]
            .milestone
            .as_ref()
            .expect("milestone persists across reopen");
        assert_eq!(m.item, "Desc_IronRod_C");
        assert_eq!(m.total, 2500.0);
        assert_eq!(m.rate, 30.0);
    }
}

/// A power-line proposal item that DELETES an existing route (for split tests).
fn delete_route_item(route: &Id) -> ProposalItem {
    ProposalItem {
        id: new_id(),
        kind: ProposalItemKind::Modify,
        included: true,
        label: "⚡ cut power line".into(),
        detail: "grid split".into(),
        impact: "power".into(),
        commands: vec![Command::DeleteRoute { id: route.clone() }],
        aliases: vec![None],
        depends_on: vec![],
        sync: None,
    }
}

/// T7 (P1 before-attribution) — MERGE: two separate before-grids fold into ONE
/// after-grid. The merged row's `demand_before` must SUM both source grids (no
/// drop): the old per-after single-match attributed only one. A bridging load
/// (created but ungridded until the proposal wires it) makes the merge visible.
#[test]
fn grid_merge_sums_both_before_grids_demand() {
    let mut s = Session::in_memory(None).unwrap();
    let pa = gen_factory(&mut s, "PLANT A", 60.0);
    let la = load_factory(&mut s, "LOAD A", 15.0); // 1 constructor ≈ 4 MW
    let pb = gen_factory(&mut s, "PLANT B", 60.0);
    let lb = load_factory(&mut s, "LOAD B", 15.0);
    // two independent grids already in the plan
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Power,
        from: pa.clone(),
        to: la.clone(),
        path: vec![pos(0.0, 0.0), pos(1.0, 1.0)],
    }])
    .unwrap();
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Power,
        from: pb.clone(),
        to: lb.clone(),
        path: vec![pos(0.0, 0.0), pos(1.0, 1.0)],
    }])
    .unwrap();
    // a bridging load — exists but ungridded (draws 0 until the proposal ties it)
    let lc = load_factory(&mut s, "BRIDGE LOAD", 15.0);

    let before = s.solve_all_readonly();
    assert_eq!(before.circuits.len(), 2, "two separate before grids");
    let sum_before_demand: f64 = before.circuits.iter().map(|c| c.demand_mw).sum();
    let one_grid_demand = before.circuits[0].demand_mw;
    assert!(one_grid_demand > 1e-6, "each grid draws power");

    // the proposal wires LOAD A → BRIDGE → PLANT B, merging both grids into one
    // and adding the bridge's draw.
    let pid = store_proposal(
        &mut s,
        vec![power_route_item(&la, &lc), power_route_item(&lc, &pb)],
    );
    let cons = s.eval_proposal(&pid).unwrap();

    assert_eq!(
        cons.circuit_impacts.len(),
        1,
        "both grids merge into one touched grid: {:?}",
        cons.circuit_impacts
    );
    let ci = &cons.circuit_impacts[0];
    // the merged row's before demand is the SUM of BOTH source grids, not one.
    assert!(
        (ci.demand_before_mw - sum_before_demand).abs() < 1e-6,
        "demand_before sums both grids ({sum_before_demand}), got {}",
        ci.demand_before_mw
    );
    assert!(
        ci.demand_before_mw > one_grid_demand + 1e-6,
        "not a single-grid attribution (would have dropped the other): {ci:?}"
    );
    // and the bridge load pushes after-demand strictly above the merged before.
    assert!(
        ci.demand_after_mw > ci.demand_before_mw + 1e-6,
        "the bridge adds draw: {ci:?}"
    );
}

/// T7 (P1 before-attribution) — SPLIT: one before-grid divides into TWO
/// after-grids when the proposal cuts the bridge route. The before grid's demand
/// must be attributed to exactly ONE child (its primary destination) — NOT
/// double-counted onto both rows — and the sibling reads as newly-formed
/// (before = 0). The old per-after single-match matched the lone before grid to
/// BOTH children, double-counting its demand.
#[test]
fn grid_split_does_not_double_count_before_demand() {
    let mut s = Session::in_memory(None).unwrap();
    let pa = gen_factory(&mut s, "PLANT A", 60.0);
    let la = load_factory(&mut s, "LOAD A", 15.0);
    let pb = gen_factory(&mut s, "PLANT B", 60.0);
    let lb = load_factory(&mut s, "LOAD B", 15.0);
    // one grid: PLANT A—LOAD A and PLANT B—LOAD B, bridged by LOAD A—LOAD B.
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Power,
        from: pa.clone(),
        to: la.clone(),
        path: vec![pos(0.0, 0.0), pos(1.0, 1.0)],
    }])
    .unwrap();
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Power,
        from: pb.clone(),
        to: lb.clone(),
        path: vec![pos(0.0, 0.0), pos(1.0, 1.0)],
    }])
    .unwrap();
    let bridge = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Power,
            from: la.clone(),
            to: lb.clone(),
            path: vec![pos(0.0, 0.0), pos(1.0, 1.0)],
        }])
        .unwrap()
        .created[0]
        .clone();

    let before = s.solve_all_readonly();
    assert_eq!(before.circuits.len(), 1, "one bridged grid before the cut");
    let orig_demand = before.circuits[0].demand_mw;
    assert!(orig_demand > 1e-6, "the grid draws power");

    // the proposal cuts the bridge → two independent after-grids.
    let pid = store_proposal(&mut s, vec![delete_route_item(&bridge)]);
    let cons = s.eval_proposal(&pid).unwrap();

    assert_eq!(
        cons.circuit_impacts.len(),
        2,
        "the cut yields two touched after grids: {:?}",
        cons.circuit_impacts
    );
    // the before grid's demand is counted ONCE across the two rows, not twice.
    let total_before: f64 = cons
        .circuit_impacts
        .iter()
        .map(|c| c.demand_before_mw)
        .sum();
    assert!(
        (total_before - orig_demand).abs() < 1e-6,
        "before demand attributed once ({orig_demand}), not double-counted: got {total_before}"
    );
    // one child inherits the whole before grid; the sibling reads newly-formed.
    assert!(
        cons.circuit_impacts
            .iter()
            .any(|c| (c.demand_before_mw - orig_demand).abs() < 1e-6),
        "one child carries the whole before grid: {:?}",
        cons.circuit_impacts
    );
    assert!(
        cons.circuit_impacts
            .iter()
            .any(|c| c.demand_before_mw == 0.0 && c.generation_before_mw == 0.0),
        "the sibling reads as newly-formed (before = 0): {:?}",
        cons.circuit_impacts
    );
}
