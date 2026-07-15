//! PR 9 opportunity engine: every family fires ONLY on real derived evidence
//! and stays silent without it (honest silence — never a guessed number);
//! deficits are two-gap decomposed (production vs transport) so every card
//! names the TRUE cause; ranking is the documented class-order tuple, capped
//! at 12.

use app::opportunities::{derive_opportunities, Opportunity, OpportunityAction, OpportunityKind};
use app::Session;
use gamedata::docs::Recipe;
use gamedata::worldnodes::{Entrance, WorldNode};
use planner_core::commands::Command;
use planner_core::entities::*;

fn mk_factory(s: &mut Session, name: &str, x: f64, y: f64) -> Id {
    s.edit(vec![Command::CreateFactory {
        name: name.into(),
        position: MapPos { x, y, z: 0.0 },
        region: "GRASS FIELDS".into(),
    }])
    .unwrap()
    .created[0]
        .clone()
}

fn add_group(s: &mut Session, fid: &Id, machine: &str, recipe: &str, count: u32) -> Id {
    s.edit(vec![Command::AddGroup {
        factory: fid.clone(),
        machine: machine.into(),
        recipe: recipe.into(),
        count,
        clock: 1.0,
        graph_pos: GraphPos { x: 0.0, y: 0.0 },
        floor: 0,
    }])
    .unwrap()
    .created[0]
        .clone()
}

fn add_port(s: &mut Session, fid: &Id, dir: PortDirection, item: &str, ceiling: Option<f64>) -> Id {
    s.edit(vec![Command::AddPort {
        factory: fid.clone(),
        direction: dir,
        item: item.into(),
        rate: 0.0,
        rate_ceiling: ceiling,
        graph_pos: GraphPos { x: 0.0, y: 0.0 },
    }])
    .unwrap()
    .created[0]
        .clone()
}

fn belt(s: &mut Session, fid: &Id, from: EdgeEnd, to: EdgeEnd, item: &str) {
    s.edit(vec![Command::AddEdge {
        factory: fid.clone(),
        from,
        to,
        item: item.into(),
        tier: 6,
    }])
    .unwrap();
}

fn set_rate(s: &mut Session, port: &Id, rate: f64) {
    s.edit(vec![Command::SetPortRate {
        id: port.clone(),
        rate,
    }])
    .unwrap();
}

fn next(s: &mut Session) -> Vec<Opportunity> {
    let derived = s.solve_all_readonly();
    let prefs = s.state.meta.preferences.clone();
    derive_opportunities(
        &s.state,
        &s.gamedata,
        &derived,
        &s.world,
        &s.unlocked,
        &s.purchased_schematics,
        &prefs,
    )
}

/// ore in → smelter bank → ingot out at `rate` (a cleanly-solving producer).
fn ingot_factory(
    s: &mut Session,
    name: &str,
    x: f64,
    y: f64,
    smelters: u32,
    rate: f64,
) -> (Id, Id) {
    let fid = mk_factory(s, name, x, y);
    let ore_in = add_port(s, &fid, PortDirection::In, "Desc_OreIron_C", Some(1200.0));
    let out = add_port(s, &fid, PortDirection::Out, "Desc_IronIngot_C", None);
    let bank = add_group(
        s,
        &fid,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        smelters,
    );
    belt(
        s,
        &fid,
        EdgeEnd::Port(ore_in),
        EdgeEnd::Group(bank.clone()),
        "Desc_OreIron_C",
    );
    belt(
        s,
        &fid,
        EdgeEnd::Group(bank),
        EdgeEnd::Port(out.clone()),
        "Desc_IronIngot_C",
    );
    set_rate(s, &out, rate);
    (fid, out)
}

/// ingot in → constructor bank → rod out (targets set by the caller).
fn rod_sink(s: &mut Session, name: &str, x: f64, y: f64, ctors: u32) -> (Id, Id, Id) {
    let fid = mk_factory(s, name, x, y);
    let ingot_in = add_port(s, &fid, PortDirection::In, "Desc_IronIngot_C", None);
    let rod_out = add_port(s, &fid, PortDirection::Out, "Desc_IronRod_C", None);
    let bank = add_group(s, &fid, "Build_ConstructorMk1_C", "Recipe_IronRod_C", ctors);
    belt(
        s,
        &fid,
        EdgeEnd::Port(ingot_in.clone()),
        EdgeEnd::Group(bank.clone()),
        "Desc_IronIngot_C",
    );
    belt(
        s,
        &fid,
        EdgeEnd::Group(bank),
        EdgeEnd::Port(rod_out.clone()),
        "Desc_IronRod_C",
    );
    (fid, ingot_in, rod_out)
}

/// ore in (ceiling) straight to an ore out port — the wizard's extraction-
/// and-ship pass-through shape (group-less, edge-wired; a valid T1 solve).
fn ore_mine(s: &mut Session, name: &str, x: f64, y: f64, rate: f64) -> (Id, Id) {
    let fid = mk_factory(s, name, x, y);
    let ore_in = add_port(s, &fid, PortDirection::In, "Desc_OreIron_C", Some(1200.0));
    let ore_out = add_port(s, &fid, PortDirection::Out, "Desc_OreIron_C", None);
    belt(
        s,
        &fid,
        EdgeEnd::Port(ore_in),
        EdgeEnd::Port(ore_out.clone()),
        "Desc_OreIron_C",
    );
    set_rate(s, &ore_out, rate);
    (fid, ore_out)
}

/// ore in → smelter bank → ingot out, the ore In port UNCEILINGED (a bound
/// route injects its supply as the effective ceiling — the route-binds case).
fn ore_smelter(s: &mut Session, name: &str, x: f64, y: f64, smelters: u32) -> (Id, Id, Id) {
    let fid = mk_factory(s, name, x, y);
    let ore_in = add_port(s, &fid, PortDirection::In, "Desc_OreIron_C", None);
    let out = add_port(s, &fid, PortDirection::Out, "Desc_IronIngot_C", None);
    let bank = add_group(
        s,
        &fid,
        "Build_SmelterMk1_C",
        "Recipe_IngotIron_C",
        smelters,
    );
    belt(
        s,
        &fid,
        EdgeEnd::Port(ore_in.clone()),
        EdgeEnd::Group(bank.clone()),
        "Desc_OreIron_C",
    );
    belt(
        s,
        &fid,
        EdgeEnd::Group(bank),
        EdgeEnd::Port(out.clone()),
        "Desc_IronIngot_C",
    );
    (fid, ore_in, out)
}

fn belt_route(s: &mut Session, from: &Id, to: &Id, tier: u8) -> Id {
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Belt { tier },
        from: from.clone(),
        to: to.clone(),
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
        .clone()
}

/// coal in → coal generator → `rate` MW out (grid generation).
fn coal_plant(s: &mut Session, name: &str, x: f64, y: f64, rate: f64) -> Id {
    let fid = mk_factory(s, name, x, y);
    let coal_in = add_port(s, &fid, PortDirection::In, "Desc_Coal_C", Some(480.0));
    let mw_out = add_port(s, &fid, PortDirection::Out, "__PowerMW", None);
    let gens = add_group(
        s,
        &fid,
        "Build_GeneratorCoal_C",
        "Recipe_Power_Build_GeneratorCoal_Desc_Coal_C",
        4,
    );
    belt(
        s,
        &fid,
        EdgeEnd::Port(coal_in),
        EdgeEnd::Group(gens.clone()),
        "Desc_Coal_C",
    );
    belt(
        s,
        &fid,
        EdgeEnd::Group(gens),
        EdgeEnd::Port(mw_out.clone()),
        "__PowerMW",
    );
    set_rate(s, &mw_out, rate);
    fid
}

fn power_route(s: &mut Session, a: &Id, b: &Id) {
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Power,
        from: a.clone(),
        to: b.clone(),
        path: vec![
            MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            MapPos {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    }])
    .unwrap();
}

/// The H1 test chassis: `smelters` at `up_rate` ingots/min feed `ctors`
/// constructors through a route that starts Mk.4 (so the `rod_rate` target is
/// set while achievable — targets are never rewritten by a tier change), then
/// drops to Mk.1 (60 cap). `dip_to` optionally lowers the upstream target
/// AFTER the drop, steering how the miss decomposes.
fn capped_chain(
    s: &mut Session,
    smelters: u32,
    up_rate: f64,
    ctors: u32,
    rod_rate: f64,
    dip_to: Option<f64>,
) -> Id {
    let (_, ingot_out) = ingot_factory(s, "BIG SMELT", 0.0, 0.0, smelters, up_rate);
    let (_, ingot_in, rod_out) = rod_sink(s, "ROD SINK", 500.0, 0.0, ctors);
    let route = belt_route(s, &ingot_out, &ingot_in, 4);
    set_rate(s, &rod_out, rod_rate);
    s.edit(vec![Command::SetRouteTier {
        id: route.clone(),
        tier: 1,
    }])
    .unwrap();
    if let Some(rate) = dip_to {
        set_rate(s, &ingot_out, rate);
    }
    route
}

fn find_kind(opps: &[Opportunity], kind: OpportunityKind) -> Option<&Opportunity> {
    opps.iter().find(|o| o.kind == kind)
}

fn count_kind(opps: &[Opportunity], kind: OpportunityKind) -> usize {
    opps.iter().filter(|o| o.kind == kind).count()
}

/// An empty plan yields NOTHING — silence, not filler ideas.
#[test]
fn empty_plan_is_silent() {
    let mut s = Session::in_memory(None).unwrap();
    assert!(next(&mut s).is_empty(), "no evidence → no opportunities");
}

/// power_deficit: a grid drawing more than it generates fires class 0 with
/// the derived MW pair as evidence; a healthy grid stays silent. An overdrawn
/// grid is NEVER also a margin warning (S4 — the two bands are exclusive).
#[test]
fn power_deficit_fires_on_overdraw_only() {
    let mut s = Session::in_memory(None).unwrap();
    // 75 MW plant powering a 128 MW load (32 smelters @ 4 MW) → overdrawn.
    let plant = coal_plant(&mut s, "POWER RIDGE", 0.0, 0.0, 75.0);
    let (load, _) = ingot_factory(&mut s, "LOAD BLOCK", 100.0, 0.0, 32, 960.0);
    power_route(&mut s, &plant, &load);

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::PowerDeficit).expect("overdrawn grid fires");
    // S5 literal pins: 128 MW drawn of 75 generated → 53 MW overdraw, whole-MW.
    assert_eq!(o.title, "GRID A is overdrawn by 53 MW");
    assert_eq!(o.evidence, "128 MW demand against 75 MW generated");
    assert_eq!(
        o.action,
        OpportunityAction::OpenAudit {
            tab: "power".into()
        }
    );
    // and it ranks first — class 0 leads everything else present
    assert_eq!(opps[0].kind, OpportunityKind::PowerDeficit);
    // S4: the overdraw never doubles as a thin-margin warning
    assert!(
        !opps.iter().any(|o| o.kind == OpportunityKind::PowerMargin),
        "an overdrawn grid is a deficit, not a margin"
    );

    // shrink the load to 8 smelters at a matching 240/min target (32 MW of
    // 75 — the rate must drop too, or the solver overclocks the smaller bank
    // and its clock-scaled draw keeps the grid overdrawn) → healthy, silent
    let bank = s
        .state
        .groups
        .values()
        .find(|g| g.factory == load && g.machine == "Build_SmelterMk1_C")
        .unwrap()
        .id
        .clone();
    let out = s
        .state
        .ports
        .values()
        .find(|p| p.factory == load && p.direction == PortDirection::Out)
        .unwrap()
        .id
        .clone();
    s.edit(vec![Command::SetGroupCount { id: bank, count: 8 }])
        .unwrap();
    set_rate(&mut s, &out, 240.0);
    assert!(
        !next(&mut s)
            .iter()
            .any(|o| o.kind == OpportunityKind::PowerDeficit),
        "healthy grid must not fire power_deficit"
    );
}

/// L5: a sub-half-MW overdraw renders with one decimal — an overdrawn grid
/// must never read "overdrawn by 0 MW".
#[test]
fn power_deficit_small_overdraw_keeps_a_decimal() {
    let mut s = Session::in_memory(None).unwrap();
    // 63.6 MW plant vs a 64 MW load (16 smelters @ 4 MW) → 0.4 MW overdraw.
    let plant = coal_plant(&mut s, "POWER RIDGE", 0.0, 0.0, 63.6);
    let (load, _) = ingot_factory(&mut s, "LOAD BLOCK", 100.0, 0.0, 16, 480.0);
    power_route(&mut s, &plant, &load);

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::PowerDeficit).expect("0.4 MW overdraw still fires");
    assert!(
        o.title.contains("overdrawn by 0.4 MW"),
        "sub-half-MW overdraw keeps a decimal: {}",
        o.title
    );
}

/// deficit_repair: starved targets group by item empire-wide; the action is a
/// wizard prefill at the ceiled PRODUCTION-gap rate. The tier-4 route here has
/// slack (the upstream dip is a pure production gap), so route_bottleneck_fix
/// must stay silent (S1 — no false transport attribution).
#[test]
fn deficit_repair_groups_by_item_and_prefills_wizard() {
    let mut s = Session::in_memory(None).unwrap();
    // Build the chain SATISFIED first (the downstream target must be set while
    // achievable — an unachievable SetPortRate is clamp-written-back), then
    // dip the upstream to 10/min so the 60-rod target honestly starves.
    let (_, ingot_out) = ingot_factory(&mut s, "OPPORTUNITY BAY", 0.0, 0.0, 4, 60.0);
    let (_, ingot_in, rod_out) = rod_sink(&mut s, "FOUNDRY GAP", 500.0, 0.0, 4);
    belt_route(&mut s, &ingot_out, &ingot_in, 4);
    set_rate(&mut s, &rod_out, 60.0); // satisfiable now
    set_rate(&mut s, &ingot_out, 10.0); // upstream dips → downstream starves

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::DeficitRepair).expect("starved chain fires");
    // S5 literal pin: the full title format, not fragments.
    assert_eq!(o.title, "Iron Ingot is short 50.0/min empire-wide");
    assert_eq!(o.item.as_deref(), Some("Desc_IronIngot_C"));
    assert_eq!(
        o.evidence, "need 60.0/min, supplied 10.0/min across 1 port(s)",
        "a slack route earns no transport suffix"
    );
    match &o.action {
        OpportunityAction::WizardGoal { item, rate } => {
            assert_eq!(item, "Desc_IronIngot_C");
            assert_eq!(*rate, 50.0, "ceil(60 needed − 10 produced)");
        }
        other => panic!("expected WizardGoal, got {other:?}"),
    }
    // deterministic id — stable across recomputes
    assert_eq!(o.id, "deficit_repair:Desc_IronIngot_C");
    // S1: the route has slack — the starve is production-caused, and no
    // route_bottleneck_fix card may claim otherwise.
    assert!(
        !opps
            .iter()
            .any(|o| o.kind == OpportunityKind::RouteBottleneckFix),
        "slack route must not fire route_bottleneck_fix"
    );
}

/// S2: two consumers starved on the SAME item (both over slack routes — pure
/// production gaps) collapse into exactly ONE empire-wide card whose number,
/// evidence row count, and wizard rate are the SUM: gaps 49.5 + 80.0 = 129.5,
/// prefill ceil(129.5) = 130.
#[test]
fn deficit_repair_sums_two_consumers_of_one_item() {
    let mut s = Session::in_memory(None).unwrap();
    // Pair 1: 60-rod sink fed by a producer that will dip to 10.5/min.
    let (_, out_a) = ingot_factory(&mut s, "SMELT A", 0.0, 0.0, 4, 60.0);
    let (_, in_a, rod_a) = rod_sink(&mut s, "SINK A", 500.0, 0.0, 4);
    belt_route(&mut s, &out_a, &in_a, 4);
    set_rate(&mut s, &rod_a, 60.0); // satisfiable now
                                    // Pair 2: 120-rod sink fed by a producer that will dip to 40/min.
    let (_, out_b) = ingot_factory(&mut s, "SMELT B", 0.0, 1000.0, 4, 120.0);
    let (_, in_b, rod_b) = rod_sink(&mut s, "SINK B", 500.0, 1000.0, 8);
    belt_route(&mut s, &out_b, &in_b, 4);
    set_rate(&mut s, &rod_b, 120.0); // satisfiable now
                                     // Both upstreams dip — two pure production gaps on Iron Ingot.
    set_rate(&mut s, &out_a, 10.5); // gap 60 − 10.5 = 49.5
    set_rate(&mut s, &out_b, 40.0); // gap 120 − 40 = 80.0

    let opps = next(&mut s);
    assert_eq!(
        count_kind(&opps, OpportunityKind::DeficitRepair),
        1,
        "one item → one empire-wide card, never one per starved port"
    );
    let o = find_kind(&opps, OpportunityKind::DeficitRepair).unwrap();
    assert_eq!(o.id, "deficit_repair:Desc_IronIngot_C");
    assert_eq!(o.title, "Iron Ingot is short 129.5/min empire-wide");
    assert_eq!(
        o.evidence, "need 180.0/min, supplied 50.5/min across 2 port(s)",
        "both rows aggregate; slack routes earn no transport suffix"
    );
    match &o.action {
        OpportunityAction::WizardGoal { item, rate } => {
            assert_eq!(item, "Desc_IronIngot_C");
            assert_eq!(*rate, 130.0, "exact ceil of the summed gaps (129.5)");
        }
        other => panic!("expected WizardGoal, got {other:?}"),
    }
    // Both routes have slack — no transport story anywhere.
    assert!(
        !opps
            .iter()
            .any(|o| o.kind == OpportunityKind::RouteBottleneckFix),
        "slack routes must not fire route_bottleneck_fix"
    );
}

/// H1 case A (route-capped only): upstream already produces the full need —
/// the deficit card would plan REDUNDANT machines, so it stays silent and the
/// route card leads with the recoverable rate and the SMALLEST sufficient
/// tier (Mk.1 + 180 recoverable needs 240 → Mk.3, never a blind +1).
#[test]
fn route_capped_deficit_yields_route_card_only() {
    let mut s = Session::in_memory(None).unwrap();
    let route = capped_chain(&mut s, 8, 240.0, 16, 240.0, None);

    let opps = next(&mut s);
    assert!(
        !opps
            .iter()
            .any(|o| o.kind == OpportunityKind::DeficitRepair),
        "production covers the need — no deficit card"
    );
    let o = find_kind(&opps, OpportunityKind::RouteBottleneckFix).expect("route card fires");
    // S5 literal pins: 60 flow + 180 recoverable needs 240 → Mk.3 (skip Mk.2).
    assert_eq!(
        o.title,
        "BIG SMELT → ROD SINK caps demand — bump it to Mk.3"
    );
    assert_eq!(
        o.evidence,
        "60.0/60.0 per min at 100% with 180.0/min recoverable through it"
    );
    assert_eq!(o.action, OpportunityAction::SelectRoute { id: route });
    // No class 0/1 present → the route card leads the list.
    assert_eq!(opps[0].kind, OpportunityKind::RouteBottleneckFix);
}

/// H1 case B (mixed): upstream makes 120 of a 240 need over a 60-cap belt —
/// BOTH cards fire, each sized by its OWN gap (deficit 120 production,
/// route 60 recoverable), with the deficit evidence naming the capped share.
#[test]
fn mixed_gap_fires_both_cards_with_own_numbers() {
    let mut s = Session::in_memory(None).unwrap();
    capped_chain(&mut s, 8, 240.0, 16, 240.0, Some(120.0));

    let opps = next(&mut s);
    let d = find_kind(&opps, OpportunityKind::DeficitRepair).expect("production gap fires");
    // S5 literal pins: production gap only (240 needed − 120 produced) in the
    // title; the transport share named — not summed in — as the suffix.
    assert_eq!(d.title, "Iron Ingot is short 120.0/min empire-wide");
    assert_eq!(
        d.evidence,
        "need 240.0/min, supplied 60.0/min across 1 port(s); 60.0/min more capped by full route(s)"
    );
    match &d.action {
        OpportunityAction::WizardGoal { rate, .. } => assert_eq!(*rate, 120.0),
        other => panic!("expected WizardGoal, got {other:?}"),
    }
    let r = find_kind(&opps, OpportunityKind::RouteBottleneckFix).expect("transport gap fires");
    assert_eq!(
        r.evidence,
        "60.0/60.0 per min at 100% with 60.0/min recoverable through it"
    );
    assert!(
        r.title.contains("bump it to Mk.2"),
        "60 flow + 60 recoverable = 120 → Mk.2 suffices: {}",
        r.title
    );
    // class 1 before class 2
    let di = opps.iter().position(|o| o.id == d.id).unwrap();
    let ri = opps.iter().position(|o| o.id == r.id).unwrap();
    assert!(
        di < ri,
        "deficit_repair (class 1) before route fix (class 2)"
    );
}

/// H1 case C (starved at the cap): upstream makes EXACTLY the belt cap — the
/// route recovers nothing by itself (upgrading it moves zero extra items), so
/// the route card is silent and the deficit card carries the whole gap, with
/// the full route named as the next wall.
#[test]
fn starved_at_cap_is_deficit_with_route_mention() {
    let mut s = Session::in_memory(None).unwrap();
    capped_chain(&mut s, 8, 240.0, 16, 240.0, Some(60.0));

    let opps = next(&mut s);
    let d = find_kind(&opps, OpportunityKind::DeficitRepair).expect("real production gap fires");
    // S5 literal pins: the whole gap is production; the full route is
    // mentioned as the next wall, not carded.
    assert_eq!(d.title, "Iron Ingot is short 180.0/min empire-wide");
    assert_eq!(
        d.evidence,
        "need 240.0/min, supplied 60.0/min across 1 port(s); the Mk.1 route is already full — upgrading it is also required once production rises"
    );
    assert!(
        !opps
            .iter()
            .any(|o| o.kind == OpportunityKind::RouteBottleneckFix),
        "zero recoverable → no route card"
    );
}

/// route_bottleneck_fix fires ONLY on a recoverable transport gap; a
/// full-but-satisfied route stays silent (the efficiency grammar — 100%
/// meeting demand is optimal). The exact-fit boundary is kept deliberately:
/// 60 flow + 60 recoverable = 120 lands exactly ON Mk.2's capacity.
#[test]
fn route_bottleneck_fires_only_with_recoverable_gap() {
    let mut s = Session::in_memory(None).unwrap();
    // upstream can push 120/min; the Mk.1 route caps at 60; downstream wants 120.
    let route = capped_chain(&mut s, 4, 120.0, 8, 120.0, None);

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::RouteBottleneckFix)
        .expect("full route with recoverable gap fires");
    assert!(o.title.contains("caps demand"), "{}", o.title);
    assert!(
        o.title.contains("Mk.2"),
        "exact fit: 60 + 60 = 120 = Mk.2 capacity: {}",
        o.title
    );
    assert!(
        o.evidence.contains("60.0/min recoverable through it"),
        "{}",
        o.evidence
    );
    assert_eq!(
        o.action,
        OpportunityAction::SelectRoute { id: route.clone() }
    );
    // Pure transport gap: upstream covers the need → deficit ABSENT, and the
    // route card leads the list (nothing outranks class 2 here).
    assert!(
        !opps
            .iter()
            .any(|o| o.kind == OpportunityKind::DeficitRepair),
        "route-capped miss must not fire deficit_repair"
    );
    assert_eq!(opps[0].kind, OpportunityKind::RouteBottleneckFix);

    // downstream relaxes to 60/min: the route is FULL but satisfied → silent
    let rod_out = s
        .state
        .ports
        .values()
        .find(|p| p.item == "Desc_IronRod_C" && p.direction == PortDirection::Out)
        .unwrap()
        .id
        .clone();
    set_rate(&mut s, &rod_out, 60.0);
    assert!(
        !next(&mut s)
            .iter()
            .any(|o| o.kind == OpportunityKind::RouteBottleneckFix),
        "a full route that meets demand is optimal — no candidate"
    );
    let _ = route;
}

/// M2: when even Mk.6 can't carry flow + recoverable, the fix is a parallel
/// belt, not a tier bump into a wall. A full Mk.5 (780) under a 1 360 need:
/// 780 + 580 recoverable = 1 360 > 1 200.
#[test]
fn route_fix_beyond_mk6_names_parallel_belt() {
    let mut s = Session::in_memory(None).unwrap();
    // Upstream: 46 smelters in two banks (one Mk.6 internal edge can't carry
    // 1 360) shipping 1 360 ingots/min — the upstream WITNESS for the need.
    let up = mk_factory(&mut s, "MEGA SMELT", 0.0, 0.0);
    let ore_in = add_port(
        &mut s,
        &up,
        PortDirection::In,
        "Desc_OreIron_C",
        Some(2000.0),
    );
    let ingot_out = add_port(&mut s, &up, PortDirection::Out, "Desc_IronIngot_C", None);
    for _ in 0..2 {
        let bank = add_group(&mut s, &up, "Build_SmelterMk1_C", "Recipe_IngotIron_C", 23);
        belt(
            &mut s,
            &up,
            EdgeEnd::Port(ore_in.clone()),
            EdgeEnd::Group(bank.clone()),
            "Desc_OreIron_C",
        );
        belt(
            &mut s,
            &up,
            EdgeEnd::Group(bank),
            EdgeEnd::Port(ingot_out.clone()),
            "Desc_IronIngot_C",
        );
    }
    set_rate(&mut s, &ingot_out, 1360.0);
    // Downstream: 92 constructors in two banks wanting 1360 rods/min. The
    // target is set BEFORE the route exists (unbound In ports are
    // unconstrained), so it survives the route's cap. The route lands at
    // Mk.5: its 780 intake stays under every internal Mk.6 edge, so the
    // solver's ceiling binding names the route-injected InputCeiling — the
    // signal the deficit row needs — not an internal belt.
    let down = mk_factory(&mut s, "MEGA RODS", 500.0, 0.0);
    let ingot_in = add_port(&mut s, &down, PortDirection::In, "Desc_IronIngot_C", None);
    let rod_out = add_port(&mut s, &down, PortDirection::Out, "Desc_IronRod_C", None);
    for _ in 0..2 {
        let bank = add_group(
            &mut s,
            &down,
            "Build_ConstructorMk1_C",
            "Recipe_IronRod_C",
            46,
        );
        belt(
            &mut s,
            &down,
            EdgeEnd::Port(ingot_in.clone()),
            EdgeEnd::Group(bank.clone()),
            "Desc_IronIngot_C",
        );
        belt(
            &mut s,
            &down,
            EdgeEnd::Group(bank),
            EdgeEnd::Port(rod_out.clone()),
            "Desc_IronRod_C",
        );
    }
    set_rate(&mut s, &rod_out, 1360.0);
    belt_route(&mut s, &ingot_out, &ingot_in, 5);

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::RouteBottleneckFix).expect("Mk.5 route caps 580");
    assert!(
        o.title
            .contains("caps demand — beyond Mk.6 — add a parallel belt"),
        "780 flow + 580 recoverable exceeds every tier: {}",
        o.title
    );
    assert!(
        o.evidence.contains("580.0/min recoverable through it"),
        "{}",
        o.evidence
    );
    assert!(
        !opps
            .iter()
            .any(|o| o.kind == OpportunityKind::DeficitRepair),
        "upstream produces the full 1360 — transport-only miss"
    );
}

/// M2: a rail bottleneck's fix is the drawer's own stepper ("+1 consist"),
/// never "a second route".
#[test]
fn route_fix_rail_names_consist() {
    let mut s = Session::in_memory(None).unwrap();
    let (_, ingot_out) = ingot_factory(&mut s, "BIG SMELT", 0.0, 0.0, 8, 240.0);
    let (_, ingot_in, rod_out) = rod_sink(&mut s, "ROD SINK", 80000.0, 0.0, 16);
    // Create as a Mk.4 belt over an 80 km path (the 240-rod target must be
    // set while achievable), then swap the kind to a default 1-consist rail:
    // at this length its throughput lands well under 240/min → FULL.
    let route = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Belt { tier: 4 },
            from: ingot_out.clone(),
            to: ingot_in.clone(),
            path: vec![
                MapPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                MapPos {
                    x: 80000.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        }])
        .unwrap()
        .created[0]
        .clone();
    set_rate(&mut s, &rod_out, 240.0);
    s.edit(vec![Command::SetRouteSpec {
        id: route.clone(),
        kind: RouteKind::Rail {
            spec: RailSpec::default(),
        },
    }])
    .unwrap();

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::RouteBottleneckFix).expect("full rail route fires");
    assert!(
        o.title.contains("caps demand — +1 consist"),
        "rail fix names the consist stepper: {}",
        o.title
    );
}

/// power_margin: 0 ≤ headroom < 20% fires class 3; comfortable headroom is
/// silent. (An overdrawn grid is power_deficit, never both.)
#[test]
fn power_margin_fires_in_warn_band_only() {
    let mut s = Session::in_memory(None).unwrap();
    // 75 MW plant, 64 MW load (16 smelters) → 14.7% headroom: warn band.
    let plant = coal_plant(&mut s, "POWER RIDGE", 0.0, 0.0, 75.0);
    let (load, _) = ingot_factory(&mut s, "LOAD BLOCK", 100.0, 0.0, 16, 480.0);
    power_route(&mut s, &plant, &load);

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::PowerMargin).expect("thin-headroom grid fires");
    // S5 literal pins: 11/75 = 14.67% headroom floors to 14 in both strings.
    assert_eq!(o.title, "GRID A has only 14% headroom");
    assert_eq!(o.evidence, "14% headroom (64 of 75 MW drawn)");
    assert!(
        !opps.iter().any(|o| o.kind == OpportunityKind::PowerDeficit),
        "warn band is not an overdraw"
    );

    // 32 MW load (8 smelters at a matching 240/min target) → 57% headroom
    let bank = s
        .state
        .groups
        .values()
        .find(|g| g.factory == load && g.machine == "Build_SmelterMk1_C")
        .unwrap()
        .id
        .clone();
    let out = s
        .state
        .ports
        .values()
        .find(|p| p.factory == load && p.direction == PortDirection::Out)
        .unwrap()
        .id
        .clone();
    s.edit(vec![Command::SetGroupCount { id: bank, count: 8 }])
        .unwrap();
    set_rate(&mut s, &out, 240.0);
    assert!(
        !next(&mut s)
            .iter()
            .any(|o| o.kind == OpportunityKind::PowerMargin),
        "comfortable headroom must not nag"
    );
}

/// L5: the headroom percentage FLOORS in title and evidence — 19.5% must
/// read "19%", never round up toward comfort.
#[test]
fn power_margin_floors_the_percentage() {
    let mut s = Session::in_memory(None).unwrap();
    // generation 64/0.805 ≈ 79.5 MW over a 64 MW load → headroom exactly 19.5%.
    let plant = coal_plant(&mut s, "POWER RIDGE", 0.0, 0.0, 64.0 / 0.805);
    let (load, _) = ingot_factory(&mut s, "LOAD BLOCK", 100.0, 0.0, 16, 480.0);
    power_route(&mut s, &plant, &load);
    let _ = load;

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::PowerMargin).expect("19.5% is warn band");
    assert!(
        o.title.contains("only 19% headroom"),
        "19.5% floors to 19: {}",
        o.title
    );
    assert!(o.evidence.starts_with("19% headroom ("), "{}", o.evidence);
    let _ = plant;
}

/// S3+S6: with TWO thin grids, the THINNER margin ranks first within the
/// class — and the creation order deliberately OPPOSES the rank order (the
/// 14% grid is drawn first, the 4% grid second, so circuit enumeration order
/// alone would get this backwards without the magnitude sort). Both floored
/// percentage strings are pinned.
#[test]
fn power_margin_thinner_grid_leads_despite_creation_order() {
    let mut s = Session::in_memory(None).unwrap();
    // GRID A drawn FIRST: 75 MW over a 64 MW load → 14.67% headroom → "14%".
    let plant_a = coal_plant(&mut s, "THICK PLANT", 40000.0, 40000.0, 75.0);
    let (load_a, _) = ingot_factory(&mut s, "THICK LOAD", 40100.0, 40000.0, 16, 480.0);
    power_route(&mut s, &plant_a, &load_a);
    // GRID B drawn SECOND: 67 MW over 64 → 4.48% headroom → "4%" (thinner).
    let plant_b = coal_plant(&mut s, "THIN PLANT", 44000.0, 40000.0, 67.0);
    let (load_b, _) = ingot_factory(&mut s, "THIN LOAD", 44100.0, 40000.0, 16, 480.0);
    power_route(&mut s, &plant_b, &load_b);

    let opps = next(&mut s);
    let margins: Vec<&Opportunity> = opps
        .iter()
        .filter(|o| o.kind == OpportunityKind::PowerMargin)
        .collect();
    assert_eq!(margins.len(), 2, "both thin grids fire");
    // Thinner margin leads — 4% before 14%, the reverse of creation order.
    assert_eq!(margins[0].title, "GRID B has only 4% headroom");
    assert_eq!(margins[0].evidence, "4% headroom (64 of 67 MW drawn)");
    assert_eq!(margins[1].title, "GRID A has only 14% headroom");
    assert_eq!(margins[1].evidence, "14% headroom (64 of 75 MW drawn)");
    // Neither grid is overdrawn — margin only, no deficit cards.
    assert!(
        !opps.iter().any(|o| o.kind == OpportunityKind::PowerDeficit),
        "thin margins are not overdraws"
    );
}

/// M3: with NO power routes drawn (zero circuits) but empire totals proving
/// an overdraw, the class-0 empire fallback fires — pigeonhole-honest: at
/// least one physical grid must be overdrawn.
#[test]
fn empire_power_fallback_fires_on_overdraw_without_routes() {
    let mut s = Session::in_memory(None).unwrap();
    coal_plant(&mut s, "LONE PLANT", 0.0, 0.0, 10.0);
    ingot_factory(&mut s, "BIG LOAD", 2000.0, 2000.0, 32, 960.0);
    // no power_route — a save-imported base draws none

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::PowerDeficit).expect("empire overdraw fires");
    assert_eq!(o.id, "power_deficit:empire");
    assert_eq!(
        o.title,
        "Plan-wide power demand exceeds generation by 118 MW"
    );
    assert_eq!(
        o.evidence,
        "128 MW demand vs 10 MW generated — no power routes drawn, per-grid balance unknown"
    );
    assert_eq!(
        o.action,
        OpportunityAction::OpenAudit {
            tab: "power".into()
        }
    );
}

/// M3 asymmetry pin: a thin-but-positive EMPIRE margin proves nothing about
/// any physical grid (one grid can be overdrawn while another idles), so
/// without routes the margin family stays SILENT — deliberately.
#[test]
fn empire_power_fallback_silent_on_thin_margin() {
    let mut s = Session::in_memory(None).unwrap();
    coal_plant(&mut s, "LONE PLANT", 0.0, 0.0, 75.0);
    ingot_factory(&mut s, "LOAD BLOCK", 2000.0, 2000.0, 16, 480.0);
    // 64 of 75 MW → 14.7% empire margin, but no routes → no per-grid facts

    let opps = next(&mut s);
    assert!(
        !opps.iter().any(|o| o.kind == OpportunityKind::PowerDeficit),
        "positive margin is not an overdraw"
    );
    assert!(
        !opps.iter().any(|o| o.kind == OpportunityKind::PowerMargin),
        "empire margin proves nothing per-grid — honest silence"
    );
}

/// M3: any drawn circuit disables the empire fallback — per-grid truth wins.
#[test]
fn empire_power_fallback_silent_when_a_circuit_exists() {
    let mut s = Session::in_memory(None).unwrap();
    let plant = coal_plant(&mut s, "SMALL PLANT", 0.0, 0.0, 10.0);
    let (load, _) = ingot_factory(&mut s, "GRID LOAD", 100.0, 0.0, 4, 120.0);
    power_route(&mut s, &plant, &load);
    // a second, UNROUTED load keeps the empire totals overdrawn
    ingot_factory(&mut s, "DARK LOAD", 3000.0, 3000.0, 32, 960.0);

    let opps = next(&mut s);
    assert!(
        opps.iter().any(|o| o.id.starts_with("power_deficit:GRID")),
        "the drawn grid reports itself"
    );
    assert!(
        !opps.iter().any(|o| o.id == "power_deficit:empire"),
        "circuits exist → the empire fallback stands down"
    );
}

/// M3: zero generation is a mid-planning base (machines drawn, no generators
/// yet), not a power emergency — the fallback carves it out.
#[test]
fn empire_power_fallback_silent_at_zero_generation() {
    let mut s = Session::in_memory(None).unwrap();
    ingot_factory(&mut s, "EARLY DRAFT", 0.0, 0.0, 32, 960.0);

    assert!(
        !next(&mut s)
            .iter()
            .any(|o| o.kind == OpportunityKind::PowerDeficit),
        "no generators yet → not an overdraw nag"
    );
}

/// milestone_gap is HONEST-SILENT: gamedata carries no schematic milestone
/// costs (schematics map to recipe unlocks only) and the session persists no
/// purchased-schematic set — so the family emits NOTHING, even on a busy plan
/// with starved targets and unlocked recipes.
#[test]
fn milestone_gap_is_honest_silent_without_costs() {
    let mut s = Session::in_memory(None).unwrap();
    ingot_factory(&mut s, "BUSY", 0.0, 0.0, 4, 120.0);
    s.unlocked.insert("Recipe_IngotIron_C".into());
    assert!(
        s.gamedata.schematics.is_empty(),
        "fixture precondition: no schematic data"
    );
    assert!(
        !next(&mut s)
            .iter()
            .any(|o| o.kind == OpportunityKind::MilestoneGap),
        "no schematic costs anywhere → milestone_gap never guesses"
    );
}

/// alt_adopt: surfaces the TOP altopt opportunity (computation reused, not
/// re-derived) once an alternate is unlocked; silent with nothing unlocked.
/// The evidence carries the whole trade with verbs ("saves N MW").
#[test]
fn alt_adopt_reuses_altopt_top_row() {
    let mut s = Session::in_memory(None).unwrap();
    // a planned 4-smelter ingot line on the standard recipe
    ingot_factory(&mut s, "INGOTS", 0.0, 0.0, 4, 120.0);
    assert!(
        !next(&mut s)
            .iter()
            .any(|o| o.kind == OpportunityKind::AltAdopt),
        "nothing unlocked → silent (fixture reality)"
    );

    // inject a strictly-cheaper unlocked alternate (altopt test pattern)
    let std = s
        .gamedata
        .recipes
        .get("Recipe_IngotIron_C")
        .unwrap()
        .clone();
    let doubled = std
        .products
        .iter()
        .map(|(i, n)| (i.clone(), n * 2.0))
        .collect();
    s.gamedata.recipes.insert(
        "Recipe_Alt_IngotIron_C".into(),
        Recipe {
            class_name: "Recipe_Alt_IngotIron_C".into(),
            display_name: "Pure Iron Ingot".into(),
            products: doubled,
            alternate: true,
            ..std
        },
    );
    s.unlocked.insert("Recipe_Alt_IngotIron_C".into());

    let expected = app::altopt::empire_optimize(&s.state, &s.gamedata, &s.unlocked)
        .into_iter()
        .next()
        .expect("altopt sees the win");
    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::AltAdopt).expect("unlocked cheaper alt fires");
    assert!(
        o.title
            .contains(&format!("saves {} machines", expected.machines_saved)),
        "advertised savings come from altopt verbatim: {}",
        o.title
    );
    assert_eq!(
        o.evidence, "−2 machines · saves 8 MW · on Iron Ingot",
        "the trade line: machines, a power VERB, the product"
    );
    assert_eq!(o.item.as_deref(), Some(expected.product.as_str()));
    assert_eq!(
        o.action,
        OpportunityAction::OpenAudit {
            tab: "optimizer".into()
        }
    );
}

/// M4: a power-costing alternate says "costs N MW" (never an ambiguous "+"),
/// a built line prices its retool hours, and an ingredient the empire neither
/// makes nor imports is named as a NEW chain with its rate. The "Alternate: "
/// display prefix strips in the card only (the chip carries ALT).
#[test]
fn alt_adopt_shows_costs_retool_and_new_chain() {
    let mut s = Session::in_memory(None).unwrap();
    let (fid, _) = ingot_factory(&mut s, "OLD INGOTS", 0.0, 0.0, 4, 120.0);
    // Flip the smelter bank to ◆ Built directly (import is the only command
    // path to Built; state surgery is the established test shortcut). Built
    // groups adopt via plan_replacement, so sourceability doesn't gate them.
    let bank = s
        .state
        .groups
        .values()
        .find(|g| g.factory == fid)
        .unwrap()
        .id
        .clone();
    s.state.groups.get_mut(&bank).unwrap().status = Status::Built;

    let std = s
        .gamedata
        .recipes
        .get("Recipe_IngotIron_C")
        .unwrap()
        .clone();
    let doubled: Vec<(String, f64)> = std
        .products
        .iter()
        .map(|(i, n)| (i.clone(), n * 2.0))
        .collect();
    let mut ingredients = std.ingredients.clone();
    ingredients.push(("Desc_Coal_C".into(), 1.0)); // nobody makes or imports coal here
    s.gamedata.recipes.insert(
        "Recipe_Alt_CokedIron_C".into(),
        Recipe {
            class_name: "Recipe_Alt_CokedIron_C".into(),
            display_name: "Alternate: Coked Iron Ingot".into(),
            products: doubled,
            ingredients,
            alternate: true,
            variable_power_mw: Some(100.0), // 2 × 100 MW vs 4 × 4 MW → costs
            ..std
        },
    );
    s.unlocked.insert("Recipe_Alt_CokedIron_C".into());

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::AltAdopt).expect("net machine win fires");
    assert!(
        o.title.starts_with("Alt Coked Iron Ingot saves 2 machines"),
        "display prefix stripped, no 'Alt Alternate:': {}",
        o.title
    );
    assert_eq!(
        o.evidence,
        "−2 machines · costs 184 MW · ~0.3 h retool · needs new Coal chain (60.0/min) · on Iron Ingot",
        "the honest trade: cost verb, retool hours, new input chain"
    );
}

/// M1: an under-clocked claim on an item NOBODY is short of is deliberate
/// ratio-matching, not an opportunity — silence.
#[test]
fn under_extracted_silent_without_demand() {
    let mut s = Session::in_memory(None).unwrap();
    let fid = mk_factory(&mut s, "MINE HEAD", 0.0, 0.0);
    s.edit(vec![Command::ClaimNode {
        factory: fid,
        node: "bp_resourcenode496".into(),
        extractor: "Build_MinerMk2_C".into(),
        clock: 0.5,
    }])
    .unwrap();

    assert!(
        !next(&mut s)
            .iter()
            .any(|o| o.kind == OpportunityKind::UnderExtracted),
        "no demand for iron ore anywhere → the half-clock claim is a choice, not a gap"
    );
}

/// M1: a claim fires when its item carries an empire-wide PRODUCTION gap; at
/// most one card per item (the largest gain wins); save-only claims (no
/// catalog node → no item, no purity) stay silent even under demand. Also
/// pins the L2 wording: purity+item title, id trailing in the evidence, the
/// lost rate quantified, no duplicated "% clock".
#[test]
fn under_extracted_fires_on_demand_one_card_per_item() {
    let mut s = Session::in_memory(None).unwrap();
    // Ore chain starved by production: the mine ships 30 of the 120 ore the
    // smelter's target needs through a slack Mk.4 route → 90/min production gap.
    let (mine, ore_out) = ore_mine(&mut s, "IRON MINE", -1100.0, -500.0, 120.0);
    let (_, ore_in, ingot_out) = ore_smelter(&mut s, "SMELT ROW", -600.0, -500.0, 4);
    belt_route(&mut s, &ore_out, &ore_in, 4);
    set_rate(&mut s, &ingot_out, 120.0); // satisfiable now
    set_rate(&mut s, &ore_out, 30.0); // the dip → ore production gap 90

    for (node, clock) in [
        ("bp_resourcenode114", 0.5),  // pure iron, gain 120 — the winner
        ("bp_resourcenode115", 0.75), // pure iron, gain 60 — deduped away
        ("save:Persistent_Level:PersistentLevel.Miner_1", 0.25), // save-only
    ] {
        s.edit(vec![Command::ClaimNode {
            factory: mine.clone(),
            node: node.into(),
            extractor: "Build_MinerMk2_C".into(),
            clock,
        }])
        .unwrap();
    }

    let opps = next(&mut s);
    assert_eq!(
        count_kind(&opps, OpportunityKind::UnderExtracted),
        1,
        "one card per item, save-only claims silent"
    );
    let o = find_kind(&opps, OpportunityKind::UnderExtracted).unwrap();
    assert_eq!(o.title, "Pure Iron Ore node is extracting at 50% clock");
    assert_eq!(
        o.evidence,
        "bp_resourcenode114 · claimed by IRON MINE · +120.0/min available at 100%"
    );
    assert_eq!(o.item.as_deref(), Some("Desc_OreIron_C"));
    assert_eq!(o.action, OpportunityAction::SelectFactory { id: mine });
}

/// M1: the other demand channel — the owning factory genuinely BOUND by the
/// claim's own ceiling (output running AT an InputCeiling whose reported
/// figure equals the port's stored ceiling). No empire deficit needed.
#[test]
fn under_extracted_fires_when_claim_ceiling_binds() {
    let mut s = Session::in_memory(None).unwrap();
    let fid = mk_factory(&mut s, "BOUND WORKS", 0.0, 0.0);
    s.edit(vec![Command::ClaimNode {
        factory: fid.clone(),
        node: "bp_resourcenode114".into(), // pure iron
        extractor: "Build_MinerMk2_C".into(),
        clock: 0.5, // 120/min of a possible 240
    }])
    .unwrap();
    // Wizard convention: the In port's ceiling IS the claimed extraction rate.
    let ore_in = add_port(
        &mut s,
        &fid,
        PortDirection::In,
        "Desc_OreIron_C",
        Some(120.0),
    );
    let out = add_port(&mut s, &fid, PortDirection::Out, "Desc_IronIngot_C", None);
    let bank = add_group(&mut s, &fid, "Build_SmelterMk1_C", "Recipe_IngotIron_C", 4);
    belt(
        &mut s,
        &fid,
        EdgeEnd::Port(ore_in),
        EdgeEnd::Group(bank.clone()),
        "Desc_OreIron_C",
    );
    belt(
        &mut s,
        &fid,
        EdgeEnd::Group(bank),
        EdgeEnd::Port(out.clone()),
        "Desc_IronIngot_C",
    );
    set_rate(&mut s, &out, 120.0); // runs exactly AT the claim ceiling

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::UnderExtracted)
        .expect("ceiling-bound factory demands the lost extraction");
    assert!(
        o.evidence.contains("+120.0/min available at 100%"),
        "{}",
        o.evidence
    );
}

/// M1 boundary: an InputCeiling whose reported figure is the ROUTE's injected
/// supply (not the port's stored ceiling) is a transport limit — raising the
/// claim's clock moves nothing, so the family stays silent.
#[test]
fn under_extracted_silent_when_route_supply_binds() {
    let mut s = Session::in_memory(None).unwrap();
    let (_, ore_out) = ore_mine(&mut s, "FAR MINE", 0.0, 0.0, 240.0);
    let (smelt, ore_in, ingot_out) = ore_smelter(&mut s, "HUNGRY SMELT", 500.0, 0.0, 16);
    let route = belt_route(&mut s, &ore_out, &ore_in, 4);
    set_rate(&mut s, &ingot_out, 240.0); // satisfiable over Mk.4
    s.edit(vec![Command::SetRouteTier {
        id: route,
        tier: 1, // now the BELT caps ore at 60 — pure transport gap
    }])
    .unwrap();
    s.edit(vec![Command::ClaimNode {
        factory: smelt,
        node: "bp_resourcenode114".into(),
        extractor: "Build_MinerMk2_C".into(),
        clock: 0.5,
    }])
    .unwrap();

    let opps = next(&mut s);
    assert!(
        opps.iter()
            .any(|o| o.kind == OpportunityKind::RouteBottleneckFix),
        "the belt is the story"
    );
    assert!(
        !opps
            .iter()
            .any(|o| o.kind == OpportunityKind::UnderExtracted),
        "route-injected ceiling is not the claim's — clocking up moves nothing"
    );
}

/// untapped_node: unclaimed pure nodes near a factory surface nearest-first
/// (distance ASC), DEDUPED to one card per item (L1 — three coal pins in one
/// seam are one idea), capped at 3; claiming one removes it; with no
/// factories at all the family is silent (no anchor — honest).
#[test]
fn untapped_node_nearest_pure_unclaimed() {
    let mut s = Session::in_memory(None).unwrap();
    assert!(
        next(&mut s).is_empty(),
        "no factories → no untapped candidates"
    );

    // park a factory on a known pure-node cluster (coal around −1100, −500)
    let fid = mk_factory(&mut s, "PROSPECT CAMP", -1100.0, -500.0);
    let opps = next(&mut s);
    let untapped: Vec<&Opportunity> = opps
        .iter()
        .filter(|o| o.kind == OpportunityKind::UntappedNode)
        .collect();
    assert_eq!(untapped.len(), 3, "nearest 3 items");
    for o in &untapped {
        assert!(o.title.starts_with("Pure "), "{}", o.title);
        assert!(o.evidence.contains("m from"), "{}", o.evidence);
        // the raw id trails the evidence, never leads it (L2)
        let node = o.id.strip_prefix("untapped_node:").unwrap();
        assert!(o.evidence.ends_with(node), "{}", o.evidence);
    }
    // L1: one card per item — the coal seam's three pins collapse to one
    let items: std::collections::BTreeSet<&str> =
        untapped.iter().filter_map(|o| o.item.as_deref()).collect();
    assert_eq!(items.len(), untapped.len(), "items must be distinct");
    // distance ASC: evidence distances are non-decreasing
    let dist = |o: &Opportunity| -> f64 {
        o.evidence
            .split('~')
            .nth(1)
            .and_then(|t| t.split(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap()
    };
    for w in untapped.windows(2) {
        assert!(dist(w[0]) <= dist(w[1]), "nearest first");
    }

    // claim the nearest → it drops off the list
    let first_node = untapped[0]
        .id
        .strip_prefix("untapped_node:")
        .unwrap()
        .to_string();
    s.edit(vec![Command::ClaimNode {
        factory: fid,
        node: first_node.clone(),
        extractor: "Build_MinerMk2_C".into(),
        clock: 1.0,
    }])
    .unwrap();
    assert!(
        !next(&mut s)
            .iter()
            .any(|o| o.id == format!("untapped_node:{first_node}")),
        "claimed nodes are no longer untapped"
    );
}

/// L3: a cave node's ENTRANCE always anchors its distance — a plan-local
/// position override corrects the node marker, not the way in. If the
/// override won here (9 500 m) the node would vanish from the radius
/// entirely; the entrance keeps it at ~2 000 m.
#[test]
fn untapped_node_entrance_wins_over_override() {
    let mut s = Session::in_memory(None).unwrap();
    // an empty corner of the coordinate space — no catalog nodes in range
    mk_factory(&mut s, "DEEP CAMP", 50000.0, 50000.0);
    s.world.nodes.push(WorldNode {
        id: "test_cave_iron".into(),
        item: "Desc_OreIron_C".into(),
        purity: "pure".into(),
        x: 59000.0,
        y: 50000.0,
        z: 0.0,
        zone: "cave".into(),
        entrance: Some(Entrance {
            x: 52000.0,
            y: 50000.0,
            z: 0.0,
        }),
        region: "grass-fields".into(),
    });
    s.edit(vec![Command::SetNodeOverride {
        id: "test_cave_iron".into(),
        node_override: Some(NodeOverride {
            id: "test_cave_iron".into(),
            pos: Some(MapPos {
                x: 59500.0,
                y: 50000.0,
                z: 0.0,
            }),
            save_actor: None,
        }),
    }])
    .unwrap();

    let opps = next(&mut s);
    let o = opps
        .iter()
        .find(|o| o.id == "untapped_node:test_cave_iron")
        .expect("entrance at 2 km keeps the cave node in range");
    assert!(
        o.evidence.starts_with("~2000 m from DEEP CAMP"),
        "entrance anchors the distance: {}",
        o.evidence
    );
}

/// S7: the top untapped card matches an INDEPENDENT nearest-pure computation
/// over the raw catalog — same anchor, same hypot, same (distance, id)
/// tie-break — so the engine can't drift from the data it claims to read.
#[test]
fn untapped_node_matches_independent_nearest_computation() {
    let mut s = Session::in_memory(None).unwrap();
    let (fx, fy) = (-1100.0, -500.0);
    mk_factory(&mut s, "PROSPECT CAMP", fx, fy);

    // Independent oracle: global nearest pure node within the 2 500 m radius,
    // entrance-anchored when one exists, ties broken by id ascending. (No
    // claims and no overrides exist in this plan.)
    let mut oracle: Option<(f64, String)> = None;
    for n in &s.world.nodes {
        if n.purity != "pure" {
            continue;
        }
        let (nx, ny) = match &n.entrance {
            Some(e) => (e.x, e.y),
            None => (n.x, n.y),
        };
        let dist = (fx - nx).hypot(fy - ny);
        if dist > 2500.0 {
            continue;
        }
        let better = oracle
            .as_ref()
            .is_none_or(|(d, id)| (dist, n.id.as_str()) < (*d, id.as_str()));
        if better {
            oracle = Some((dist, n.id.clone()));
        }
    }
    let (dist, id) = oracle.expect("the camp sits on a known pure cluster");

    let opps = next(&mut s);
    let untapped: Vec<&Opportunity> = opps
        .iter()
        .filter(|o| o.kind == OpportunityKind::UntappedNode)
        .collect();
    assert!(!untapped.is_empty(), "pure cluster in range must surface");
    assert_eq!(untapped[0].id, format!("untapped_node:{id}"));
    assert_eq!(
        untapped[0].evidence,
        format!("~{dist:.0} m from PROSPECT CAMP · pure · {id}")
    );
}

/// S8: the 2 500 m radius boundary is INCLUSIVE — a pure node exactly on the
/// line surfaces; one 100 m past it does not. (Catalog replaced wholesale —
/// the established license for boundary-exact world fixtures.)
#[test]
fn untapped_node_radius_boundary_is_inclusive() {
    let mut s = Session::in_memory(None).unwrap();
    mk_factory(&mut s, "LONE CAMP", 0.0, 0.0);
    let node = |id: &str, item: &str, x: f64| WorldNode {
        id: id.into(),
        item: item.into(),
        purity: "pure".into(),
        x,
        y: 0.0,
        z: 0.0,
        zone: "surface".into(),
        entrance: None,
        region: "grass-fields".into(),
    };
    s.world.nodes = vec![
        node("edge_2400", "Desc_OreIron_C", 2400.0),
        node("edge_2500", "Desc_OreCopper_C", 2500.0),
        node("edge_2600", "Desc_Coal_C", 2600.0),
    ];

    let opps = next(&mut s);
    let ids: Vec<&str> = opps
        .iter()
        .filter(|o| o.kind == OpportunityKind::UntappedNode)
        .map(|o| o.id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["untapped_node:edge_2400", "untapped_node:edge_2500"],
        "2500 m is in (<= boundary), 2600 m is out"
    );
}

/// S9: every surfaced untapped id resolves in the catalog to purity "pure" —
/// an inverted filter would surface normal nodes mislabeled "Pure". A normal
/// node parked 50 m from the camp is the bait: nearer than every pure
/// candidate, it would LEAD the list if the filter flipped.
#[test]
fn untapped_node_surfaces_only_catalog_pure_nodes() {
    let mut s = Session::in_memory(None).unwrap();
    mk_factory(&mut s, "PURITY CAMP", -1100.0, -500.0);
    s.world.nodes.push(WorldNode {
        id: "test_normal_bait".into(),
        item: "Desc_OreCopper_C".into(),
        purity: "normal".into(),
        x: -1050.0,
        y: -500.0,
        z: 0.0,
        zone: "surface".into(),
        entrance: None,
        region: "grass-fields".into(),
    });

    let opps = next(&mut s);
    let untapped: Vec<&Opportunity> = opps
        .iter()
        .filter(|o| o.kind == OpportunityKind::UntappedNode)
        .collect();
    assert!(!untapped.is_empty(), "the pure cluster still surfaces");
    for o in &untapped {
        let nid = o.id.strip_prefix("untapped_node:").unwrap();
        assert_ne!(nid, "test_normal_bait", "normal nodes never surface");
        let n = s
            .world
            .nodes
            .iter()
            .find(|n| n.id == nid)
            .expect("surfaced id resolves in the catalog");
        assert_eq!(n.purity, "pure", "surfaced {nid} must be catalog-pure");
    }
}

/// Ranking: class order is broken → savings → growth, and the list caps at 12
/// even when more candidates exist.
#[test]
fn ranking_class_order_and_cap() {
    let mut s = Session::in_memory(None).unwrap();
    // class 1 evidence: an ore chain starved by production (slack route), on
    // a DEMANDED item so the class-6 claim below survives the M1 gate
    let (mine, ore_out) = ore_mine(&mut s, "SHORT SUPPLY", -1100.0, -500.0, 120.0);
    let (_, ore_in, ingot_out) = ore_smelter(&mut s, "WANTS MORE", -600.0, -500.0, 4);
    belt_route(&mut s, &ore_out, &ore_in, 4);
    set_rate(&mut s, &ingot_out, 120.0); // satisfiable now
    set_rate(&mut s, &ore_out, 30.0); // upstream dips → ore deficit
                                      // class 6 evidence: an under-clocked claim on the demanded ore
    s.edit(vec![Command::ClaimNode {
        factory: mine,
        node: "bp_resourcenode114".into(),
        extractor: "Build_MinerMk2_C".into(),
        clock: 0.5,
    }])
    .unwrap();
    // class 0 pressure: nine separate overdrawn grids (10 MW plants under
    // 16 MW loads), parked far from every catalog node
    for i in 0..9 {
        let x = 40000.0 + (i as f64) * 2000.0;
        let plant = coal_plant(&mut s, &format!("PLANT {i}"), x, 40000.0, 10.0);
        let (load, _) = ingot_factory(&mut s, &format!("LOAD {i}"), x, 41000.0, 4, 120.0);
        power_route(&mut s, &plant, &load);
    }

    let opps = next(&mut s);
    assert_eq!(opps.len(), 12, "capped at 12");
    // classes never regress along the list
    let class = |k: OpportunityKind| -> u8 {
        match k {
            OpportunityKind::PowerDeficit => 0,
            OpportunityKind::DeficitRepair => 1,
            OpportunityKind::RouteBottleneckFix => 2,
            OpportunityKind::PowerMargin => 3,
            OpportunityKind::MilestoneGap => 4,
            OpportunityKind::AltAdopt => 5,
            OpportunityKind::UnderExtracted => 6,
            OpportunityKind::UntappedNode => 7,
        }
    };
    for w in opps.windows(2) {
        assert!(
            class(w[0].kind) <= class(w[1].kind),
            "class order must be monotone: {:?} then {:?}",
            w[0].kind,
            w[1].kind
        );
    }
    // broken leads: the overdrawn grids (class 0) head the list
    assert_eq!(opps[0].kind, OpportunityKind::PowerDeficit);
    assert_eq!(count_kind(&opps, OpportunityKind::PowerDeficit), 9);
    // the demanded-item claim survives the gate and the cap
    let clock_card = find_kind(&opps, OpportunityKind::UnderExtracted)
        .expect("demanded under-clocked claim in the top 12");
    assert!(
        clock_card.title.contains("50% clock"),
        "{}",
        clock_card.title
    );
}

/// S11: the wire shape is a CONTRACT — the renderer destructures these exact
/// keys. One Opportunity per action variant, compared as whole JSON values:
/// camelCase field keys, tagged actions, `item` key ABSENT when None
/// (skip_serializing_if), and all eight snake_case kind strings.
#[test]
fn opportunity_serde_shape_is_pinned() {
    use serde_json::{json, to_value};

    // All eight kind strings, in class order.
    for (kind, wire) in [
        (OpportunityKind::PowerDeficit, "power_deficit"),
        (OpportunityKind::DeficitRepair, "deficit_repair"),
        (OpportunityKind::RouteBottleneckFix, "route_bottleneck_fix"),
        (OpportunityKind::PowerMargin, "power_margin"),
        (OpportunityKind::MilestoneGap, "milestone_gap"),
        (OpportunityKind::AltAdopt, "alt_adopt"),
        (OpportunityKind::UnderExtracted, "under_extracted"),
        (OpportunityKind::UntappedNode, "untapped_node"),
    ] {
        assert_eq!(to_value(kind).unwrap(), json!(wire));
    }

    // wizardGoal — `item` PRESENT (Some).
    let o = Opportunity {
        id: "deficit_repair:Desc_IronIngot_C".into(),
        kind: OpportunityKind::DeficitRepair,
        title: "Iron Ingot is short 50.0/min empire-wide".into(),
        evidence: "need 60.0/min, supplied 10.0/min across 1 port(s)".into(),
        item: Some("Desc_IronIngot_C".into()),
        action: OpportunityAction::WizardGoal {
            item: "Desc_IronIngot_C".into(),
            rate: 50.0,
        },
    };
    assert_eq!(
        to_value(&o).unwrap(),
        json!({
            "id": "deficit_repair:Desc_IronIngot_C",
            "kind": "deficit_repair",
            "title": "Iron Ingot is short 50.0/min empire-wide",
            "evidence": "need 60.0/min, supplied 10.0/min across 1 port(s)",
            "item": "Desc_IronIngot_C",
            "action": { "kind": "wizardGoal", "item": "Desc_IronIngot_C", "rate": 50.0 },
        })
    );

    // selectRoute — `item` key ABSENT when None (whole-value equality is the
    // absence proof: an extra "item": null would fail the compare).
    let o = Opportunity {
        id: "route_bottleneck_fix:R1".into(),
        kind: OpportunityKind::RouteBottleneckFix,
        title: "A → B caps demand — bump it to Mk.2".into(),
        evidence: "60.0/60.0 per min at 100% with 60.0/min recoverable through it".into(),
        item: None,
        action: OpportunityAction::SelectRoute { id: "R1".into() },
    };
    assert_eq!(
        to_value(&o).unwrap(),
        json!({
            "id": "route_bottleneck_fix:R1",
            "kind": "route_bottleneck_fix",
            "title": "A → B caps demand — bump it to Mk.2",
            "evidence": "60.0/60.0 per min at 100% with 60.0/min recoverable through it",
            "action": { "kind": "selectRoute", "id": "R1" },
        })
    );

    // selectNode
    let o = Opportunity {
        id: "untapped_node:bp_resourcenode114".into(),
        kind: OpportunityKind::UntappedNode,
        title: "Pure Iron Ore node near CAMP, unclaimed".into(),
        evidence: "~900 m from CAMP · pure · bp_resourcenode114".into(),
        item: Some("Desc_OreIron_C".into()),
        action: OpportunityAction::SelectNode {
            id: "bp_resourcenode114".into(),
        },
    };
    assert_eq!(
        to_value(&o).unwrap(),
        json!({
            "id": "untapped_node:bp_resourcenode114",
            "kind": "untapped_node",
            "title": "Pure Iron Ore node near CAMP, unclaimed",
            "evidence": "~900 m from CAMP · pure · bp_resourcenode114",
            "item": "Desc_OreIron_C",
            "action": { "kind": "selectNode", "id": "bp_resourcenode114" },
        })
    );

    // selectFactory
    let o = Opportunity {
        id: "under_extracted:C1".into(),
        kind: OpportunityKind::UnderExtracted,
        title: "Pure Iron Ore node is extracting at 50% clock".into(),
        evidence: "bp_resourcenode114 · claimed by MINE · +120.0/min available at 100%".into(),
        item: Some("Desc_OreIron_C".into()),
        action: OpportunityAction::SelectFactory { id: "F1".into() },
    };
    assert_eq!(
        to_value(&o).unwrap(),
        json!({
            "id": "under_extracted:C1",
            "kind": "under_extracted",
            "title": "Pure Iron Ore node is extracting at 50% clock",
            "evidence": "bp_resourcenode114 · claimed by MINE · +120.0/min available at 100%",
            "item": "Desc_OreIron_C",
            "action": { "kind": "selectFactory", "id": "F1" },
        })
    );

    // openAudit
    let o = Opportunity {
        id: "power_deficit:GRID A".into(),
        kind: OpportunityKind::PowerDeficit,
        title: "GRID A is overdrawn by 53 MW".into(),
        evidence: "128 MW demand against 75 MW generated".into(),
        item: None,
        action: OpportunityAction::OpenAudit {
            tab: "power".into(),
        },
    };
    assert_eq!(
        to_value(&o).unwrap(),
        json!({
            "id": "power_deficit:GRID A",
            "kind": "power_deficit",
            "title": "GRID A is overdrawn by 53 MW",
            "evidence": "128 MW demand against 75 MW generated",
            "action": { "kind": "openAudit", "tab": "power" },
        })
    );
}

/// S12: one composite plan that fires ALL EIGHT families at once: an overdrawn
/// grid, a thin grid, a slack-route ore deficit, a full route capping a
/// DIFFERENT item, an injected unpurchased milestone, an injected cheaper
/// alternate, a demanded under-clocked claim, and a factory parked on a pure
/// cluster. The overdraw leads and the class order never regresses along the
/// list (milestone_gap ranks at class 4, between power_margin and alt_adopt).
#[test]
fn composite_plan_fires_all_eight_families_in_class_order() {
    let mut s = Session::in_memory(None).unwrap();
    // Class 1 (+6, +7): ore chain starved by PRODUCTION near the pure
    // cluster — mine dips to 30 of the 120 the smelter's target needs over a
    // slack Mk.4 route (90/min ore production gap); the mine's half-clock
    // claim on a pure iron node is therefore demanded, and the mine itself
    // anchors untapped pure nodes.
    let (mine, ore_out) = ore_mine(&mut s, "SHORT SUPPLY", -1100.0, -500.0, 120.0);
    let (_, ore_in, ingot_out) = ore_smelter(&mut s, "WANTS MORE", -600.0, -500.0, 4);
    belt_route(&mut s, &ore_out, &ore_in, 4);
    set_rate(&mut s, &ingot_out, 120.0); // satisfiable now
    set_rate(&mut s, &ore_out, 30.0); // upstream dips → ore deficit
    s.edit(vec![Command::ClaimNode {
        factory: mine,
        node: "bp_resourcenode114".into(),
        extractor: "Build_MinerMk2_C".into(),
        clock: 0.5,
    }])
    .unwrap();
    // Class 2 on a DIFFERENT item: iron ingots produced in full (240) but
    // squeezed through a Mk.1 belt — transport-only, so no ingot deficit.
    capped_chain(&mut s, 8, 240.0, 16, 240.0, None);
    // Class 0: a 10 MW plant under a 16 MW load — overdrawn grid.
    let p0 = coal_plant(&mut s, "OVER PLANT", 40000.0, 40000.0, 10.0);
    let (l0, _) = ingot_factory(&mut s, "OVER LOAD", 40100.0, 40000.0, 4, 120.0);
    power_route(&mut s, &p0, &l0);
    // Class 3: a 75 MW plant under a 64 MW load — 14% headroom.
    let p1 = coal_plant(&mut s, "THIN PLANT", 44000.0, 40000.0, 75.0);
    let (l1, _) = ingot_factory(&mut s, "THIN LOAD", 44100.0, 40000.0, 16, 480.0);
    power_route(&mut s, &p1, &l1);
    // Class 5: inject + unlock a strictly-cheaper alternate ingot recipe.
    let std = s
        .gamedata
        .recipes
        .get("Recipe_IngotIron_C")
        .unwrap()
        .clone();
    let doubled = std
        .products
        .iter()
        .map(|(i, n)| (i.clone(), n * 2.0))
        .collect();
    s.gamedata.recipes.insert(
        "Recipe_Alt_IngotIron_C".into(),
        Recipe {
            class_name: "Recipe_Alt_IngotIron_C".into(),
            display_name: "Pure Iron Ingot".into(),
            products: doubled,
            alternate: true,
            ..std
        },
    );
    s.unlocked.insert("Recipe_Alt_IngotIron_C".into());
    // Class 4: an unpurchased milestone whose cost (iron plate) the empire makes
    // none of → a full-quantity gap fires the milestone card.
    s.gamedata.milestones.insert(
        "Schematic_3-1_C".into(),
        gamedata::docs::Milestone {
            display_name: "Coal Power".into(),
            tier: 3,
            cost: vec![("Desc_IronPlate_C".into(), 60.0)],
        },
    );

    let opps = next(&mut s);
    use OpportunityKind::*;
    for kind in [
        PowerDeficit,
        DeficitRepair,
        RouteBottleneckFix,
        PowerMargin,
        MilestoneGap,
        AltAdopt,
        UnderExtracted,
        UntappedNode,
    ] {
        assert!(
            find_kind(&opps, kind).is_some(),
            "{kind:?} must fire in the composite plan"
        );
    }
    // The broken grid leads everything.
    assert_eq!(opps[0].kind, PowerDeficit);
    // Windows-monotone class order across the whole list.
    let class = |k: OpportunityKind| -> u8 {
        match k {
            PowerDeficit => 0,
            DeficitRepair => 1,
            RouteBottleneckFix => 2,
            PowerMargin => 3,
            MilestoneGap => 4,
            AltAdopt => 5,
            UnderExtracted => 6,
            UntappedNode => 7,
        }
    };
    for w in opps.windows(2) {
        assert!(
            class(w[0].kind) <= class(w[1].kind),
            "class order must be monotone: {:?} then {:?}",
            w[0].kind,
            w[1].kind
        );
    }
}

// ---------- PR 3 preferences: hide suggestions, never facts ----------

/// `no_trains` fully suppresses a RAIL route-fix card (the "+1 consist"
/// suggestion) in the MIXED-gap case — where a `deficit_repair` already covers
/// the item's production gap and alludes to the route, so re-emitting it would
/// be redundant. (The complementary transport-ONLY case is re-emitted under a
/// non-train framing — see `no_trains_reemits_transport_only_rail_route_…`.)
#[test]
fn no_trains_suppresses_rail_route_card_when_deficit_repair_covers_it() {
    let mut s = Session::in_memory(None).unwrap();
    let (_, ingot_out) = ingot_factory(&mut s, "BIG SMELT", 0.0, 0.0, 8, 240.0);
    let (_, ingot_in, rod_out) = rod_sink(&mut s, "ROD SINK", 80000.0, 0.0, 16);
    let route = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Belt { tier: 4 },
            from: ingot_out.clone(),
            to: ingot_in.clone(),
            path: vec![
                MapPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                MapPos {
                    x: 80000.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        }])
        .unwrap()
        .created[0]
        .clone();
    set_rate(&mut s, &rod_out, 240.0);
    s.edit(vec![Command::SetRouteSpec {
        id: route,
        kind: RouteKind::Rail {
            spec: RailSpec::default(),
        },
    }])
    .unwrap();
    // A SEPARATE slack-route ingot chain with a pure PRODUCTION gap → an
    // empire-wide Iron Ingot `deficit_repair` now COVERS the item the rail
    // carries. That makes the rail route the MIXED case: under `no_trains` it
    // stays fully suppressed (the deficit card alludes to it), never re-emitted.
    let (_, gap_out) = ingot_factory(&mut s, "SLACK SMELT", 0.0, 6000.0, 4, 60.0);
    let (_, gap_in, gap_rod) = rod_sink(&mut s, "SLACK RODS", 500.0, 6000.0, 4);
    belt_route(&mut s, &gap_out, &gap_in, 4); // slack Mk.4 → no route card
    set_rate(&mut s, &gap_rod, 60.0); // satisfiable now
    set_rate(&mut s, &gap_out, 10.0); // dip → pure Iron Ingot production gap

    // Default (no preference): the rail route-fix fires (as "+1 consist").
    let before = next(&mut s);
    assert!(
        find_kind(&before, OpportunityKind::RouteBottleneckFix)
            .is_some_and(|o| o.title.contains("consist")),
        "rail route card fires as a consist suggestion without the preference"
    );
    assert!(
        find_kind(&before, OpportunityKind::DeficitRepair).is_some(),
        "the slack chain fires a deficit_repair covering Iron Ingot"
    );
    // no_trains: the covered rail suggestion is suppressed ENTIRELY (the only
    // remaining deficit story is the slack chain, whose route has no card).
    s.state.meta.preferences.no_trains = true;
    assert!(
        find_kind(&next(&mut s), OpportunityKind::RouteBottleneckFix).is_none(),
        "no_trains must suppress the covered rail route-fix suggestion"
    );
}

/// TA-#2: `no_trains` touches ONLY rail cards — a BELT `route_bottleneck_fix`
/// still fires (kills an over-broad `if prefs.no_trains { continue }` mutation).
#[test]
fn no_trains_keeps_belt_route_card() {
    let mut s = Session::in_memory(None).unwrap();
    // A Mk.1 belt capping a recoverable transport gap → a belt route card.
    let route = capped_chain(&mut s, 4, 120.0, 8, 120.0, None);
    assert!(
        find_kind(&next(&mut s), OpportunityKind::RouteBottleneckFix).is_some(),
        "belt route-fix fires by default"
    );
    s.state.meta.preferences.no_trains = true;
    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::RouteBottleneckFix)
        .expect("no_trains must NOT suppress a belt route-fix card");
    assert_eq!(o.action, OpportunityAction::SelectRoute { id: route });
    // The belt fix is a tier bump, never a train wording.
    assert!(
        o.title.contains("Mk."),
        "belt fix names a tier: {}",
        o.title
    );
}

/// M4: `no_trains` must not silently DROP the only advice about an EXISTING
/// overloaded rail route. A transport-ONLY starve (upstream produces the full
/// need, the rail consist caps flow, no deficit_repair covers it) is RE-EMITTED
/// under a non-train framing — same route + evidence, but suggesting a
/// belt/truck alternative instead of "+1 consist".
#[test]
fn no_trains_reemits_transport_only_rail_route_without_train_wording() {
    let mut s = Session::in_memory(None).unwrap();
    let (_, ingot_out) = ingot_factory(&mut s, "BIG SMELT", 0.0, 0.0, 8, 240.0);
    let (_, ingot_in, rod_out) = rod_sink(&mut s, "ROD SINK", 80000.0, 0.0, 16);
    // Create as a Mk.4 belt over an 80 km path (the 240-rod target must be set
    // while achievable), then swap to a default 1-consist rail → FULL, with the
    // upstream producing the whole 240 (transport-only, no production gap).
    let route = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Belt { tier: 4 },
            from: ingot_out.clone(),
            to: ingot_in.clone(),
            path: vec![
                MapPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                MapPos {
                    x: 80000.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        }])
        .unwrap()
        .created[0]
        .clone();
    set_rate(&mut s, &rod_out, 240.0);
    s.edit(vec![Command::SetRouteSpec {
        id: route.clone(),
        kind: RouteKind::Rail {
            spec: RailSpec::default(),
        },
    }])
    .unwrap();

    // Precondition: transport-only, so no deficit_repair exists to allude to it.
    s.state.meta.preferences.no_trains = true;
    let opps = next(&mut s);
    assert!(
        !opps
            .iter()
            .any(|o| o.kind == OpportunityKind::DeficitRepair),
        "upstream produces the whole need — a transport-only starve"
    );
    let o = find_kind(&opps, OpportunityKind::RouteBottleneckFix)
        .expect("the existing overloaded rail route is re-emitted, not dropped");
    assert!(
        !o.title.contains("consist"),
        "re-emit must NOT suggest a train: {}",
        o.title
    );
    assert!(
        o.title.contains("belt") || o.title.contains("truck"),
        "re-emit names a belt/truck alternative: {}",
        o.title
    );
    assert!(o.title.contains("caps demand"), "{}", o.title);
    // Same action + recoverable evidence as the suppressed rail card.
    assert_eq!(o.action, OpportunityAction::SelectRoute { id: route });
    assert!(
        o.evidence.contains("recoverable through it"),
        "{}",
        o.evidence
    );
}

/// `ignore_power` HIDES the advisory `power_margin` card entirely, but only
/// REAL-cross-class-DEMOTES the `power_deficit` FACT (below the actionable
/// repairs the player chose to act on) and appends an honest note — never
/// removes it.
#[test]
fn ignore_power_hides_margin_but_demotes_deficit_with_note() {
    // Thin grid alone: power_margin present by default, hidden under ignore_power.
    let mut thin = Session::in_memory(None).unwrap();
    let plant = coal_plant(&mut thin, "POWER RIDGE", 0.0, 0.0, 75.0);
    let (load, _) = ingot_factory(&mut thin, "LOAD BLOCK", 100.0, 0.0, 16, 480.0);
    power_route(&mut thin, &plant, &load);
    assert!(
        find_kind(&next(&mut thin), OpportunityKind::PowerMargin).is_some(),
        "thin grid fires power_margin by default"
    );
    thin.state.meta.preferences.ignore_power = true;
    assert!(
        find_kind(&next(&mut thin), OpportunityKind::PowerMargin).is_none(),
        "ignore_power must HIDE the advisory power_margin card"
    );

    // Overdrawn grid AND an actionable production starve on a DIFFERENT chain:
    // the FACT survives ignore_power — demoted BELOW the repair + noted.
    let mut over = Session::in_memory(None).unwrap();
    let p = coal_plant(&mut over, "OVER PLANT", 0.0, 0.0, 75.0);
    let (l, _) = ingot_factory(&mut over, "OVER LOAD", 100.0, 0.0, 32, 960.0);
    power_route(&mut over, &p, &l);
    // A separate starved chain (far from the grid) → a deficit_repair (class 1)
    // card the player CAN act on: 4 smelters dipped to 10/min under a 60-rod
    // sink over a slack Mk.4 route → 50/min Iron Ingot production gap.
    let (_, ingot_out) = ingot_factory(&mut over, "SMELT GAP", 5000.0, 5000.0, 4, 60.0);
    let (_, ingot_in, rod_out) = rod_sink(&mut over, "ROD GAP", 5500.0, 5000.0, 4);
    belt_route(&mut over, &ingot_out, &ingot_in, 4);
    set_rate(&mut over, &rod_out, 60.0); // satisfiable now
    set_rate(&mut over, &ingot_out, 10.0); // upstream dips → downstream starves

    let before = next(&mut over);
    let d = find_kind(&before, OpportunityKind::PowerDeficit).expect("overdraw fires by default");
    assert!(!d.evidence.contains("ignored by preference"));
    // By default the overdraw is class 0 and leads everything.
    assert_eq!(before[0].kind, OpportunityKind::PowerDeficit);

    over.state.meta.preferences.ignore_power = true;
    let after = next(&mut over);
    let d = find_kind(&after, OpportunityKind::PowerDeficit)
        .expect("the overdraw FACT is never removed by ignore_power");
    assert!(
        d.evidence.contains("power ignored by preference")
            && d.evidence.contains("still overdrawn"),
        "demoted deficit carries the honest note: {}",
        d.evidence
    );
    // The title (the actual overdraw figure) is unchanged — the fact is intact.
    assert!(d.title.contains("overdrawn by"), "{}", d.title);
    // REAL demotion: the deficit_repair the player chose to act on now ranks
    // ABOVE the demoted overdraw (a mutation leaving power at class 0 fails).
    let di = after
        .iter()
        .position(|o| o.kind == OpportunityKind::DeficitRepair)
        .expect("the starved chain fires a deficit_repair card");
    let pi = after
        .iter()
        .position(|o| o.kind == OpportunityKind::PowerDeficit)
        .expect("the overdraw is still listed");
    assert!(
        di < pi,
        "ignore_power sinks the overdraw BELOW the actionable repair: deficit@{di} power@{pi}"
    );
}

// ---------- PR 4 milestone_gap: the next HUB milestone as a rate ----------

/// milestone_gap fires on the lowest unpurchased milestone, sizing the gap as
/// cost − 60·production against the empire's current OUTPUT rate, and pins the
/// card's title, evidence, and wizard rate (the remainder in ~1 h, ceiled).
#[test]
fn milestone_gap_fires_and_pins_the_card() {
    let mut s = Session::in_memory(None).unwrap();
    // Empire makes 30 iron ingots/min → 1800 in an hour.
    ingot_factory(&mut s, "SMELT", 0.0, 0.0, 1, 30.0);
    s.gamedata.milestones.insert(
        "Schematic_3-1_C".into(),
        gamedata::docs::Milestone {
            display_name: "Coal Power".into(),
            tier: 3,
            cost: vec![("Desc_IronIngot_C".into(), 5000.0)],
        },
    );

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::MilestoneGap).expect("unpurchased milestone fires");
    assert_eq!(o.id, "milestone_gap:Schematic_3-1_C");
    assert_eq!(o.title, "Advance to Coal Power (Tier 3)");
    // 5000 needed − 30/min·60 = 3200 short of a 1-hour build. Single-item
    // milestone → no "+N more" bill; B3 gross-production disclosure appended.
    assert_eq!(
        o.evidence,
        "needs 5000 Iron Ingot; empire makes 30/min — 3200 short of a 1-hour build · based on current production; stockpiles not counted"
    );
    assert_eq!(o.item.as_deref(), Some("Desc_IronIngot_C"));
    // Produce the remainder in ~1 h, ceiled: ceil(3200/60) = 54/min.
    assert_eq!(
        o.action,
        OpportunityAction::WizardGoal {
            item: "Desc_IronIngot_C".into(),
            rate: 54.0,
        }
    );
}

/// SILENT when the only milestone is already purchased — nothing left to plan.
#[test]
fn milestone_gap_silent_when_purchased() {
    let mut s = Session::in_memory(None).unwrap();
    ingot_factory(&mut s, "SMELT", 0.0, 0.0, 1, 30.0);
    s.gamedata.milestones.insert(
        "Schematic_3-1_C".into(),
        gamedata::docs::Milestone {
            display_name: "Coal Power".into(),
            tier: 3,
            cost: vec![("Desc_IronIngot_C".into(), 5000.0)],
        },
    );
    s.purchased_schematics.insert("Schematic_3-1_C".into());
    assert_eq!(
        count_kind(&next(&mut s), OpportunityKind::MilestoneGap),
        0,
        "a purchased milestone never nags"
    );
}

/// SILENT when the empire already out-produces every cost within an hour — the
/// gap is ≤ 0, so there is nothing to plan (honest silence, never a 0-card).
#[test]
fn milestone_gap_silent_when_empire_out_produces_cost() {
    let mut s = Session::in_memory(None).unwrap();
    // 100 ingots/min → 6000 in an hour, more than the whole cost.
    ingot_factory(&mut s, "SMELT", 0.0, 0.0, 4, 100.0);
    s.gamedata.milestones.insert(
        "Schematic_3-1_C".into(),
        gamedata::docs::Milestone {
            display_name: "Coal Power".into(),
            tier: 3,
            cost: vec![("Desc_IronIngot_C".into(), 5000.0)],
        },
    );
    assert_eq!(
        count_kind(&next(&mut s), OpportunityKind::MilestoneGap),
        0,
        "already buildable within an hour → silent"
    );
}

/// Across a MULTI-ITEM cost the largest-gap item wins — even when its absolute
/// quantity is the smaller of the two (it's the SHORTFALL that ranks, not the
/// cost). The empire makes ingots but no plate, so plate is the wall.
#[test]
fn milestone_gap_picks_the_largest_gap_item() {
    let mut s = Session::in_memory(None).unwrap();
    // 30 ingots/min → 1800/h; zero iron plate produced anywhere.
    ingot_factory(&mut s, "SMELT", 0.0, 0.0, 1, 30.0);
    s.gamedata.milestones.insert(
        "Schematic_3-1_C".into(),
        gamedata::docs::Milestone {
            display_name: "Coal Power".into(),
            tier: 3,
            // ingot gap = 1850 − 1800 = 50; plate gap = 500 − 0 = 500 (wins).
            cost: vec![
                ("Desc_IronIngot_C".into(), 1850.0),
                ("Desc_IronPlate_C".into(), 500.0),
            ],
        },
    );

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::MilestoneGap).expect("fires");
    assert_eq!(o.item.as_deref(), Some("Desc_IronPlate_C"));
    // Two-item cost → the B4 "+1 more" bill; B3 disclosure appended.
    assert_eq!(
        o.evidence,
        "needs 500 Iron Plate; empire makes 0/min — 500 short of a 1-hour build · +1 more in this milestone · based on current production; stockpiles not counted"
    );
    assert_eq!(
        o.action,
        OpportunityAction::WizardGoal {
            item: "Desc_IronPlate_C".into(),
            rate: 9.0, // ceil(500/60)
        }
    );
}

/// B2 (frontier-anchored selection, replaces the old lowest-across-the-tree
/// TA-H1 pin): the next milestone is the lowest UNPURCHASED one AT the FRONTIER
/// tier (= the highest purchased tier), never a low tier the player already
/// SKIPPED. The ids are constructed so tier order and class-name order DIVERGE
/// — the frontier winner is NOT the globally-lowest class name — so a naive
/// class-name-only sort would pick the wrong (skipped-low) milestone. Also pins
/// the nothing-purchased → lowest-overall fallback and the frontier-cleared →
/// silent branch.
#[test]
fn milestone_gap_anchors_to_the_frontier_tier() {
    let mut s = Session::in_memory(None).unwrap();
    // Empire makes no iron plate → every milestone below has a real full gap.
    ingot_factory(&mut s, "SMELT", 0.0, 0.0, 1, 30.0);
    let plate_cost = || vec![("Desc_IronPlate_C".to_string(), 500.0)];
    // A SKIPPED-low unpurchased tier-2 milestone whose class name "Schematic_A…"
    // sorts FIRST globally — the trap a class-name-only sort falls into.
    s.gamedata.milestones.insert(
        "Schematic_A_2_C".into(),
        gamedata::docs::Milestone {
            display_name: "Skipped Low".into(),
            tier: 2,
            cost: plate_cost(),
        },
    );
    // Two PURCHASED milestones establish the frontier at tier 5.
    s.gamedata.milestones.insert(
        "Schematic_C_3_C".into(),
        gamedata::docs::Milestone {
            display_name: "Bought Tier 3".into(),
            tier: 3,
            cost: plate_cost(),
        },
    );
    s.gamedata.milestones.insert(
        "Schematic_D_5_C".into(),
        gamedata::docs::Milestone {
            display_name: "Bought Tier 5".into(),
            tier: 5,
            cost: plate_cost(),
        },
    );
    // Two UNPURCHASED milestones AT the frontier tier 5; "Schematic_B…" < "…E…"
    // by class name, so B wins the frontier — but B is NOT the globally-lowest
    // class name (A is), so tier and class-name order genuinely diverge.
    s.gamedata.milestones.insert(
        "Schematic_E_5_C".into(),
        gamedata::docs::Milestone {
            display_name: "Frontier E".into(),
            tier: 5,
            cost: plate_cost(),
        },
    );
    s.gamedata.milestones.insert(
        "Schematic_B_5_C".into(),
        gamedata::docs::Milestone {
            display_name: "Frontier B".into(),
            tier: 5,
            cost: plate_cost(),
        },
    );

    // (1) Nothing purchased yet → fall back to the lowest-overall milestone
    // (tier 2, the genuine first step of a fresh import).
    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::MilestoneGap).expect("fresh import fires tier-1-ish");
    assert_eq!(o.title, "Advance to Skipped Low (Tier 2)");

    // (2) Purchase the two frontier-establishing milestones → frontier = tier 5.
    // The card names the FRONTIER-tier milestone (Frontier B), NOT the skipped
    // low tier-2 one, and NOT Frontier E (B < E by class name).
    s.purchased_schematics.insert("Schematic_C_3_C".into());
    s.purchased_schematics.insert("Schematic_D_5_C".into());
    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::MilestoneGap).expect("frontier tier fires");
    assert_eq!(
        o.title, "Advance to Frontier B (Tier 5)",
        "frontier-tier pick by class name, never the skipped low milestone"
    );
    assert_eq!(o.id, "milestone_gap:Schematic_B_5_C");
    assert_eq!(count_kind(&opps, OpportunityKind::MilestoneGap), 1);

    // (3) Clear the frontier tier (buy both remaining tier-5 milestones) → the
    // next tier is phase-gated and invisible to us, so we stay SILENT even
    // though the skipped tier-2 milestone is still unpurchased below.
    s.purchased_schematics.insert("Schematic_B_5_C".into());
    s.purchased_schematics.insert("Schematic_E_5_C".into());
    assert_eq!(
        count_kind(&next(&mut s), OpportunityKind::MilestoneGap),
        0,
        "frontier cleared → honest silence, never back-fill the skipped low tier"
    );
}

/// B1 (engine belt-and-suspenders): a low-tier EMPTY-cost milestone (every cost
/// item was unknown and dropped) must NOT be selected and shadow a higher
/// real-gap milestone — the higher card fires. Guards against the case where a
/// zero-cost entry slipped the parse-time drop.
#[test]
fn milestone_gap_empty_cost_does_not_shadow_real_milestone() {
    let mut s = Session::in_memory(None).unwrap();
    ingot_factory(&mut s, "SMELT", 0.0, 0.0, 1, 30.0);
    // A low-tier milestone with an EMPTY cost (all-unknown, post-retain).
    s.gamedata.milestones.insert(
        "Schematic_2-1_C".into(),
        gamedata::docs::Milestone {
            display_name: "Phantom".into(),
            tier: 2,
            cost: vec![],
        },
    );
    // A higher milestone with a real, unmet gap.
    s.gamedata.milestones.insert(
        "Schematic_4-1_C".into(),
        gamedata::docs::Milestone {
            display_name: "Real Wall".into(),
            tier: 4,
            cost: vec![("Desc_IronPlate_C".into(), 500.0)],
        },
    );

    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::MilestoneGap)
        .expect("the real higher milestone fires, not silenced by the empty one");
    assert_eq!(o.title, "Advance to Real Wall (Tier 4)");
    assert_eq!(count_kind(&opps, OpportunityKind::MilestoneGap), 1);
}

/// L2: an equal-gap tie between two cost items breaks by item CLASS (the
/// deterministic `item < *bi`), so the pick is stable across re-fetches. The
/// empire makes neither item and both cost the same → identical gaps; the
/// lexicographically-smaller class ("Desc_IronPlate_C" < "Desc_IronRod_C")
/// wins.
#[test]
fn milestone_gap_equal_gap_breaks_by_item_class() {
    let mut s = Session::in_memory(None).unwrap();
    ingot_factory(&mut s, "SMELT", 0.0, 0.0, 1, 30.0);
    s.gamedata.milestones.insert(
        "Schematic_3-1_C".into(),
        gamedata::docs::Milestone {
            display_name: "Coal Power".into(),
            tier: 3,
            // identical quantities, empire produces neither → equal gaps.
            cost: vec![
                ("Desc_IronRod_C".into(), 400.0),
                ("Desc_IronPlate_C".into(), 400.0),
            ],
        },
    );
    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::MilestoneGap).expect("fires");
    assert_eq!(
        o.item.as_deref(),
        Some("Desc_IronPlate_C"),
        "equal gaps break by the smaller item class, deterministically"
    );
}

/// The purchased-schematic set is save-derived and PERSISTS: an import captures
/// the RAW ids, hydrate surfaces them, and they survive a reopen through the
/// persist layer (mirrors the `unlocked` round-trip).
#[cfg(feature = "sqlite")]
#[test]
fn purchased_schematics_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.ficsit");
    {
        let mut s = app::Session::open(&path, None, "fixture").unwrap();
        let snap = app::import::ImportSnapshot {
            save_name: "MILE-01".into(),
            machines: vec![app::import::ImportMachine {
                class: "Build_SmelterMk1_C".into(),
                recipe: Some("Recipe_IngotIron_C".into()),
                clock: 1.0,
                ..Default::default()
            }],
            unlocked_schematics: vec!["Schematic_3-1_C".into(), "Schematic_2-4_C".into()],
            ..Default::default()
        };
        s.import_save(snap).unwrap();
        assert!(s.purchased_schematics.contains("Schematic_3-1_C"));
        assert!(s.purchased_schematics.contains("Schematic_2-4_C"));
        // surfaced through hydrate for parity with `unlocked`
        let h = s.hydrate();
        let arr = h["purchasedSchematics"]
            .as_array()
            .expect("hydrate carries purchasedSchematics");
        assert_eq!(arr.len(), 2);
        // M1: a RE-import with an EMPTY unlocked_schematics set (a transient
        // absent schematic parse) must NOT wipe the prior purchases — the
        // non-empty guard mirrors `unlocked`. The set AND its persisted blob
        // both survive.
        let empty = app::import::ImportSnapshot {
            save_name: "MILE-01-REIMPORT".into(),
            machines: vec![app::import::ImportMachine {
                class: "Build_SmelterMk1_C".into(),
                recipe: Some("Recipe_IngotIron_C".into()),
                clock: 1.0,
                ..Default::default()
            }],
            unlocked_schematics: vec![],
            ..Default::default()
        };
        s.import_save(empty).unwrap();
        assert_eq!(
            s.purchased_schematics.len(),
            2,
            "an empty re-import must not wipe prior purchases"
        );
        assert!(s.purchased_schematics.contains("Schematic_3-1_C"));
        assert!(s.purchased_schematics.contains("Schematic_2-4_C"));
    }
    // reopen: the META blob round-trips through the persist layer.
    let s2 = app::Session::open(&path, None, "fixture").unwrap();
    assert!(
        s2.purchased_schematics.contains("Schematic_3-1_C"),
        "purchased set survives reopen"
    );
    assert_eq!(s2.purchased_schematics.len(), 2);
}

/// M2: `plan_hash()` is UNCHANGED across a `purchased_schematics` mutation — the
/// save-derived purchased set is advisory input to `milestone_gap`, never plan
/// geometry, so it must stay OUT of the hash (cheap insurance that it can never
/// staleness-flag proposals or trip the per-edit merge, exactly like `unlocked`
/// and `preferences`).
#[test]
fn purchased_schematics_never_enter_plan_hash() {
    let mut s = Session::in_memory(None).unwrap();
    ingot_factory(&mut s, "SMELT", 0.0, 0.0, 1, 30.0);
    let before = s.plan_hash();
    s.purchased_schematics.insert("Schematic_3-1_C".into());
    s.purchased_schematics.insert("Schematic_2-4_C".into());
    assert_eq!(
        s.plan_hash(),
        before,
        "the purchased set is save-derived advisory input, never plan geometry"
    );
}

/// L1: an item the empire produces NONE of renders "empire makes 0/min", never
/// "-0/min" — the `+ 0.0` normalizer over `empire_output(..).max(0.0)` keeps a
/// signed zero from ever reaching the evidence. (A genuine -0.0 in derived
/// `out_rates` is unreachable through the command API — solver rates are
/// non-negative magnitudes — so this pins the positive-zero rendering the
/// normalizer guarantees.)
#[test]
fn milestone_gap_renders_positive_zero_production() {
    let mut s = Session::in_memory(None).unwrap();
    // Empire makes ingots but ZERO iron plate anywhere.
    ingot_factory(&mut s, "SMELT", 0.0, 0.0, 1, 30.0);
    s.gamedata.milestones.insert(
        "Schematic_3-1_C".into(),
        gamedata::docs::Milestone {
            display_name: "Coal Power".into(),
            tier: 3,
            cost: vec![("Desc_IronPlate_C".into(), 500.0)],
        },
    );
    let opps = next(&mut s);
    let o = find_kind(&opps, OpportunityKind::MilestoneGap).expect("fires");
    assert!(
        o.evidence.contains("empire makes 0/min"),
        "positive zero, never '-0/min': {}",
        o.evidence
    );
    assert!(
        !o.evidence.contains("-0/min"),
        "the signed-zero normalizer holds: {}",
        o.evidence
    );
}
