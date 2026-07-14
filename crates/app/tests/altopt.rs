//! W2b-D empire alternate-recipe optimizer: a DERIVED, ADVISORY, READ-ONLY
//! greedy per-item ranking (savings − retool cost). The ranking is proven over
//! SYNTHETIC gamedata (the trimmed fixture ships no unlocked alternates, so the
//! optimizer honestly returns nothing there); the CTA routing is proven against
//! a real Session — an all-◇ opportunity drafts a T2 `SetGroupRecipe` proposal,
//! any ◆ built factory routes through a W2a Refactor and the ◆ layer is asserted
//! BYTE-IDENTICAL (never mutated).

use std::collections::BTreeSet;

use app::altopt::empire_optimize;
use app::import::{ImportMachine, ImportSnapshot};
use app::Session;
use gamedata::docs::{GameData, Recipe};
use planner_core::commands::Command;
use planner_core::entities::*;
use planner_core::proposals::{ProposalItemKind, ProposalSource};
use planner_core::state::PlanState;

/// A synthetic recipe: one product at `per_cycle` per `dur_s`, ingredients given.
fn recipe(
    class: &str,
    product: &str,
    per_cycle: f64,
    dur_s: f64,
    alternate: bool,
    ingredients: Vec<(&str, f64)>,
    power_mw: f64,
) -> Recipe {
    Recipe {
        class_name: class.into(),
        display_name: class.into(),
        duration_s: dur_s,
        ingredients: ingredients
            .into_iter()
            .map(|(i, n)| (i.into(), n))
            .collect(),
        products: vec![(product.into(), per_cycle)],
        produced_in: vec!["Build_ConstructorMk1_C".into()],
        alternate,
        // recipe_power reads this override when present, so power is independent
        // of the (empty) synthetic machine table.
        variable_power_mw: Some(power_mw),
    }
}

/// A ◇ planned group on `recipe` producing its product, inside a fresh factory.
fn planned_group(state: &mut PlanState, recipe_class: &str, count: u32, clock: f64) -> Id {
    let fid = new_id();
    let gid = new_id();
    state.factories.insert(
        fid.clone(),
        Factory {
            id: fid.clone(),
            name: format!("F-{fid}"),
            position: MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            region: String::new(),
            node_claims: vec![],
            groups: vec![gid.clone()],
            ports: vec![],
            style_guide: None,
            replaces: None,
            status: Status::Planned,
            created_by: CreatedBy::Manual,
        },
    );
    state.groups.insert(
        gid.clone(),
        MachineGroup {
            id: gid.clone(),
            factory: fid,
            machine: "Build_ConstructorMk1_C".into(),
            recipe: recipe_class.into(),
            count,
            clock,
            somersloops: 0,
            planned_delta: None,
            graph_pos: GraphPos { x: 0.0, y: 0.0 },
            floor: 0,
            status: Status::Planned,
            created_by: CreatedBy::Manual,
        },
    );
    gid
}

fn unlocked(classes: &[&str]) -> BTreeSet<String> {
    classes.iter().map(|s| s.to_string()).collect()
}

/// Add an IN port for `item` to the factory owning `gid` so that synthetic
/// factory can LOCALLY source that ingredient (a boundary feed). The optimizer's
/// per-factory sourceability gate (O2) only counts a planned group when the alt's
/// ingredients are sourceable in its own factory — realistic fixtures declare the
/// feed rather than relying on an empire-wide raw-item escape hatch.
fn add_in_port(state: &mut PlanState, gid: &Id, item: &str) {
    let fid = state.groups[gid].factory.clone();
    let pid = new_id();
    state.ports.insert(
        pid.clone(),
        Port {
            id: pid.clone(),
            factory: fid.clone(),
            direction: PortDirection::In,
            item: item.into(),
            rate: 0.0,
            rate_ceiling: None,
            bound_route: None,
            graph_pos: GraphPos { x: 0.0, y: 0.0 },
            status: Status::Planned,
            created_by: CreatedBy::Manual,
        },
    );
    state.factories.get_mut(&fid).unwrap().ports.push(pid);
}

/// Two products, each with a cheaper unlocked alternate; the bigger machine
/// saving ranks first, and machines_saved/power_saved are the real recipe math.
#[test]
fn ranks_by_machines_then_power() {
    let mut gd = GameData::default();
    // Product A: standard 1/min (power 10), unlocked alt 4/min (power 5).
    gd.recipes.insert(
        "Recipe_A_Std_C".into(),
        recipe(
            "Recipe_A_Std_C",
            "Desc_A_C",
            1.0,
            60.0,
            false,
            vec![("Desc_OreA_C", 1.0)],
            10.0,
        ),
    );
    gd.recipes.insert(
        "Recipe_A_Alt_C".into(),
        recipe(
            "Recipe_A_Alt_C",
            "Desc_A_C",
            4.0,
            60.0,
            true,
            vec![("Desc_OreA_C", 1.0)],
            5.0,
        ),
    );
    // Product B: standard 1/min, unlocked alt 2/min — a smaller saving.
    gd.recipes.insert(
        "Recipe_B_Std_C".into(),
        recipe(
            "Recipe_B_Std_C",
            "Desc_B_C",
            1.0,
            60.0,
            false,
            vec![("Desc_OreB_C", 1.0)],
            4.0,
        ),
    );
    gd.recipes.insert(
        "Recipe_B_Alt_C".into(),
        recipe(
            "Recipe_B_Alt_C",
            "Desc_B_C",
            2.0,
            60.0,
            true,
            vec![("Desc_OreB_C", 1.0)],
            2.0,
        ),
    );

    let mut state = PlanState::default();
    // Two ◇ groups of A across the empire (count 3 each) + one B group (count 2),
    // each with an IN port feeding its alt's ore so the per-factory gate counts it.
    let ga1 = planned_group(&mut state, "Recipe_A_Std_C", 3, 1.0);
    let ga2 = planned_group(&mut state, "Recipe_A_Std_C", 3, 1.0);
    let gb = planned_group(&mut state, "Recipe_B_Std_C", 2, 1.0);
    add_in_port(&mut state, &ga1, "Desc_OreA_C");
    add_in_port(&mut state, &ga2, "Desc_OreA_C");
    add_in_port(&mut state, &gb, "Desc_OreB_C");

    let opps = empire_optimize(
        &state,
        &gd,
        &unlocked(&["Recipe_A_Alt_C", "Recipe_B_Alt_C"]),
    );
    assert_eq!(opps.len(), 2, "one opportunity per unlocked alt");

    // A ranks first: each group of 3 → 1 machine (ceil(3/4)), saved 2 ×2 groups = 4.
    let a = &opps[0];
    assert_eq!(a.recipe, "Recipe_A_Alt_C");
    assert_eq!(a.product, "Desc_A_C");
    assert_eq!(a.machines_saved, 4, "3→1 across two groups");
    // power: current 10×3×2 = 60, alt 5×1×2 = 10 → saved 50.
    assert!(
        (a.power_saved_mw - 50.0).abs() < 1e-9,
        "power saved {}",
        a.power_saved_mw
    );
    assert_eq!(a.affected_planned.len(), 2);
    assert!(a.affected_built.is_empty());
    assert_eq!(a.retool_est_hours, 0.0, "all ◇ planned → free retool");

    // B ranks second: 2→1, saved 1.
    let b = &opps[1];
    assert_eq!(b.recipe, "Recipe_B_Alt_C");
    assert_eq!(b.machines_saved, 1);
    assert!(
        a.machines_saved > b.machines_saved,
        "ranked by machines saved"
    );
}

/// An alt that trades one input for another surfaces the swap honestly.
#[test]
fn input_delta_surfaced() {
    let mut gd = GameData::default();
    // Standard widget: 1/min from IRON. Alt widget: 2/min from STEEL (a trade).
    gd.recipes.insert(
        "Recipe_W_Std_C".into(),
        recipe(
            "Recipe_W_Std_C",
            "Desc_Widget_C",
            1.0,
            60.0,
            false,
            vec![("Desc_Iron_C", 2.0)],
            6.0,
        ),
    );
    gd.recipes.insert(
        "Recipe_W_Alt_C".into(),
        recipe(
            "Recipe_W_Alt_C",
            "Desc_Widget_C",
            2.0,
            60.0,
            true,
            vec![("Desc_Steel_C", 1.0)],
            3.0,
        ),
    );
    let mut state = PlanState::default();
    // The alt trades iron for STEEL, so the group's factory must be able to
    // source steel locally (per-factory gate) — declare a steel IN port.
    let gw = planned_group(&mut state, "Recipe_W_Std_C", 4, 1.0);
    add_in_port(&mut state, &gw, "Desc_Steel_C");

    let opps = empire_optimize(&state, &gd, &unlocked(&["Recipe_W_Alt_C"]));
    assert_eq!(opps.len(), 1);
    let deltas = &opps[0].input_deltas;
    assert!(
        !deltas.is_empty(),
        "the input trade is surfaced, not hidden"
    );
    // cur_rate = 4/min. Iron: 2 per cycle / 1 out per cycle → −8/min. Steel: 1
    // per cycle / 2 out per cycle → +2/min.
    let iron = deltas
        .iter()
        .find(|(i, _)| i == "Desc_Iron_C")
        .map(|(_, v)| *v);
    let steel = deltas
        .iter()
        .find(|(i, _)| i == "Desc_Steel_C")
        .map(|(_, v)| *v);
    assert!(
        (iron.unwrap() + 8.0).abs() < 1e-9,
        "iron down 8/min, got {iron:?}"
    );
    assert!(
        (steel.unwrap() - 2.0).abs() < 1e-9,
        "steel up 2/min, got {steel:?}"
    );
}

/// An empty unlocked set (the fixture/e2e reality) yields no opportunities.
#[test]
fn empty_unlocked_yields_no_opportunities() {
    let mut gd = GameData::default();
    gd.recipes.insert(
        "Recipe_Std_C".into(),
        recipe(
            "Recipe_Std_C",
            "Desc_X_C",
            1.0,
            60.0,
            false,
            vec![("Desc_Ore_C", 1.0)],
            5.0,
        ),
    );
    gd.recipes.insert(
        "Recipe_Alt_C".into(),
        recipe(
            "Recipe_Alt_C",
            "Desc_X_C",
            4.0,
            60.0,
            true,
            vec![("Desc_Ore_C", 1.0)],
            2.0,
        ),
    );
    let mut state = PlanState::default();
    planned_group(&mut state, "Recipe_Std_C", 4, 1.0);
    assert!(
        empire_optimize(&state, &gd, &BTreeSet::new()).is_empty(),
        "no unlocked alts → nothing to optimize (honest degradation)"
    );
}

// ---- CTA routing against a real Session (fixture gamedata + injected alt) ----

const INGOT: &str = "Desc_IronIngot_C";

/// Inject a cheaper unlocked alternate for iron ingot into a session's gamedata,
/// returning its class. Ore is raw (sourceable), so the alt is adoptable.
fn inject_ingot_alt(s: &mut Session) -> String {
    let class = "Recipe_Alt_IngotIron_C";
    // Mirror the standard ingot recipe's ore input but at double the throughput
    // so it is strictly cheaper (fewer machines).
    let std = s
        .gamedata
        .recipes
        .get("Recipe_IngotIron_C")
        .cloned()
        .expect("fixture has Recipe_IngotIron_C");
    let alt = Recipe {
        class_name: class.into(),
        display_name: "Pure Iron Ingot".into(),
        duration_s: std.duration_s,
        ingredients: std.ingredients.clone(),
        products: vec![(INGOT.into(), per_cycle_out(&std, INGOT) * 2.0)],
        produced_in: std.produced_in.clone(),
        alternate: true,
        variable_power_mw: None,
    };
    s.gamedata.recipes.insert(class.into(), alt);
    s.unlocked.insert(class.into());
    class.into()
}

fn per_cycle_out(r: &Recipe, item: &str) -> f64 {
    r.products
        .iter()
        .find(|(i, _)| i == item)
        .map(|(_, n)| *n)
        .unwrap_or(1.0)
}

/// An all-◇ opportunity routes to a T2 `SetGroupRecipe` proposal that accepts.
#[test]
fn planned_only_routes_to_t2() {
    let mut s = Session::in_memory(None).unwrap();
    let alt = inject_ingot_alt(&mut s);

    // A ◇ planned ingot factory: ore IN → 1 constructor on the STANDARD recipe.
    let fid = s
        .edit(vec![Command::CreateFactory {
            name: "INGOTS".into(),
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
    let ore_in = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::In,
            item: "Desc_OreIron_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: GraphPos { x: 0.0, y: 0.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    let gid = s
        .edit(vec![Command::AddGroup {
            factory: fid.clone(),
            machine: "Build_SmelterMk1_C".into(),
            recipe: "Recipe_IngotIron_C".into(),
            count: 4,
            clock: 1.0,
            graph_pos: GraphPos { x: 0.0, y: 0.0 },
            floor: 0,
        }])
        .unwrap()
        .created[0]
        .clone();
    s.edit(vec![Command::AddEdge {
        factory: fid.clone(),
        from: EdgeEnd::Port(ore_in),
        to: EdgeEnd::Group(gid.clone()),
        item: "Desc_OreIron_C".into(),
        tier: 3,
    }])
    .unwrap();
    // An OUT port targeting 120/min (= 4 smelters × 30/min) pins the group at 4
    // machines through the solve write-back, so the alt (60/min) saves 2.
    let out = s
        .edit(vec![Command::AddPort {
            factory: fid.clone(),
            direction: PortDirection::Out,
            item: INGOT.into(),
            rate: 120.0,
            rate_ceiling: None,
            graph_pos: GraphPos { x: 0.0, y: 0.0 },
        }])
        .unwrap()
        .created[0]
        .clone();
    s.edit(vec![Command::AddEdge {
        factory: fid.clone(),
        from: EdgeEnd::Group(gid.clone()),
        to: EdgeEnd::Port(out.clone()),
        item: INGOT.into(),
        tier: 5,
    }])
    .unwrap();
    s.edit(vec![Command::SetPortRate {
        id: out,
        rate: 120.0,
    }])
    .unwrap();
    assert_eq!(s.state.groups[&gid].count, 4, "group pinned at 4 machines");

    // The optimizer sees the opportunity as ◇-only.
    let opps = empire_optimize(&s.state, &s.gamedata, &s.unlocked);
    let opp = opps
        .iter()
        .find(|o| o.recipe == alt)
        .expect("ingot alt opportunity");
    assert!(opp.affected_built.is_empty(), "planned only");
    assert!(opp.affected_planned.contains(&gid));

    let outcome = s.optimize_adopt(&alt).unwrap();
    assert_eq!(outcome.route, "t2", "◇-only → T2 route");
    let pid = outcome.proposals[0].clone();
    let proposal = s.state.proposals.get(&pid).unwrap().clone();
    assert_eq!(proposal.source, ProposalSource::T2Optimize);
    // The proposal carries a SetGroupRecipe onto the ◇ group (legal on planned).
    assert!(proposal.items.iter().any(|it| {
        it.kind == ProposalItemKind::Modify
            && it.commands.iter().any(|c| {
                matches!(c, Command::SetGroupRecipe { id, recipe, .. } if id == &gid && recipe == &alt)
            })
    }));

    // Accept works and flips the ◇ group onto the alt.
    s.accept_proposal(&pid).unwrap();
    assert_eq!(
        s.state.groups[&gid].recipe, alt,
        "the planned group adopted the alt"
    );
}

fn mach(class: &str, recipe: &str, x: f64) -> ImportMachine {
    ImportMachine {
        class: class.into(),
        recipe: Some(recipe.into()),
        clock: 1.0,
        x,
        y: 0.0,
        z: 0.0,
        ..Default::default()
    }
}

/// Import three co-located ◆ ingot smelters (one clustered group, count 3) and
/// return the built factory id. Three machines clears the ceil rounding: 3 → 2.
fn import_built_ingots(s: &mut Session) -> Id {
    s.import_save(ImportSnapshot {
        save_name: "BASE".into(),
        machines: vec![
            mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", 0.0),
            mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", 40.0),
            mach("Build_SmelterMk1_C", "Recipe_IngotIron_C", 80.0),
        ],
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

/// A ◆ built factory in the opportunity routes to a W2a Refactor, and the ◆
/// group's recipe/count/status are BYTE-IDENTICAL after drafting — no
/// `SetGroupRecipe` ever touches the built layer.
#[test]
fn built_routes_to_refactor_never_mutates_built() {
    let mut s = Session::in_memory(None).unwrap();
    // Import a ◆ built ingot factory FIRST — import resets the unlocked set — then
    // inject the unlocked alt so the optimizer can see it (raw-sourceable ore).
    let old_fid = import_built_ingots(&mut s);
    let alt = inject_ingot_alt(&mut s);
    // Give it an OUT port so plan_replacement has an item to reproduce.
    s.edit(vec![Command::AddPort {
        factory: old_fid.clone(),
        direction: PortDirection::Out,
        item: INGOT.into(),
        rate: 30.0,
        rate_ceiling: None,
        graph_pos: GraphPos { x: 0.0, y: 0.0 },
    }])
    .unwrap();
    let old_gid = s.state.factories[&old_fid].groups[0].clone();
    let before = s.state.groups[&old_gid].clone();

    // The optimizer sees a ◆ built opportunity, with a non-zero retool estimate.
    let opps = empire_optimize(&s.state, &s.gamedata, &s.unlocked);
    let opp = opps
        .iter()
        .find(|o| o.recipe == alt)
        .expect("ingot alt opportunity");
    assert!(
        opp.affected_built.contains(&old_gid),
        "built group in the opportunity"
    );
    assert!(
        opp.retool_est_hours > 0.0,
        "built machines cost retool hours"
    );

    let outcome = s.optimize_adopt(&alt).unwrap();
    assert_eq!(outcome.route, "refactor", "any ◆ → Refactor route");
    let pid = outcome.proposals[0].clone();
    let proposal = s.state.proposals.get(&pid).unwrap().clone();
    assert_eq!(proposal.source, ProposalSource::Refactor);
    // The refactor is a ◇ replacement carrying `replaces`, NOT a ◆ edit: no
    // SetGroupRecipe anywhere in the drafted proposal.
    assert!(
        !proposal.items.iter().any(|it| it
            .commands
            .iter()
            .any(|c| matches!(c, Command::SetGroupRecipe { .. }))),
        "a refactor never emits SetGroupRecipe"
    );
    assert!(proposal.items.iter().any(|it| it
        .commands
        .iter()
        .any(|c| matches!(c, Command::SetFactoryReplaces { replaces, .. } if replaces.as_deref() == Some(old_fid.as_str())))));

    // The ◆ built group is byte-identical — never mutated.
    let after = s.state.groups[&old_gid].clone();
    assert_eq!(before.recipe, after.recipe, "◆ recipe untouched");
    assert_eq!(before.count, after.count, "◆ count untouched");
    assert_eq!(before.status, Status::Built);
    assert_eq!(after.status, Status::Built, "◆ status untouched");
    assert_eq!(before, after, "the whole ◆ group is byte-identical");
}

/// A ◆ built factory in the opportunity that still holds a node claim flags
/// `node_reuse` — the refactored replacement would re-claim the held node.
#[test]
fn node_reuse_flagged() {
    let mut s = Session::in_memory(None).unwrap();
    let fid = import_built_ingots(&mut s);
    let alt = inject_ingot_alt(&mut s);
    // The ◆ factory holds a node claim.
    s.edit(vec![Command::ClaimNode {
        factory: fid.clone(),
        node: "bp_resourcenode496".into(),
        extractor: "Build_MinerMk2_C".into(),
        clock: 1.0,
    }])
    .unwrap();

    let opps = empire_optimize(&s.state, &s.gamedata, &s.unlocked);
    let opp = opps
        .iter()
        .find(|o| o.recipe == alt)
        .expect("ingot alt opportunity");
    assert!(opp.node_reuse, "a held node on the ◆ factory → node_reuse");
}

/// Inject a WORSE unlocked ingot alt: HALF the standard throughput, so it needs
/// MORE machines and cost-based `pick_recipe` would never choose it — only an
/// explicit PIN adopts it. Returns its class.
fn inject_ingot_alt_worse(s: &mut Session) -> String {
    let class = "Recipe_Alt_IngotIron_Worse_C";
    let std = s
        .gamedata
        .recipes
        .get("Recipe_IngotIron_C")
        .cloned()
        .expect("fixture has Recipe_IngotIron_C");
    let alt = Recipe {
        class_name: class.into(),
        display_name: "Slow Iron Ingot".into(),
        duration_s: std.duration_s,
        ingredients: std.ingredients.clone(),
        products: vec![(INGOT.into(), per_cycle_out(&std, INGOT) * 0.5)],
        produced_in: std.produced_in.clone(),
        alternate: true,
        variable_power_mw: None,
    };
    s.gamedata.recipes.insert(class.into(), alt);
    s.unlocked.insert(class.into());
    class.into()
}

/// T2 — the built-factory "adopt this alt" PINS the clicked recipe in the ◇
/// replacement's solve goal: the drafted Refactor's CREATE group is solved onto
/// the pinned alt (even though it is strictly WORSE on cost, so an unpinned solve
/// would never pick it), and the ◆ built group is left BYTE-IDENTICAL.
#[test]
fn built_refactor_pins_the_adopted_alt() {
    let mut s = Session::in_memory(None).unwrap();
    // ◆ built ingot factory FIRST (import resets unlocked), then inject the pin
    // target — a worse-on-cost alt, so only the pin can adopt it.
    let old_fid = import_built_ingots(&mut s);
    let alt = inject_ingot_alt_worse(&mut s);
    // OUT port so plan_replacement has an item (INGOT) to reproduce.
    s.edit(vec![Command::AddPort {
        factory: old_fid.clone(),
        direction: PortDirection::Out,
        item: INGOT.into(),
        rate: 30.0,
        rate_ceiling: None,
        graph_pos: GraphPos { x: 0.0, y: 0.0 },
    }])
    .unwrap();
    let old_gid = s.state.factories[&old_fid].groups[0].clone();
    let before = s.state.groups[&old_gid].clone();

    let outcome = s.optimize_adopt(&alt).unwrap();
    assert_eq!(outcome.route, "refactor", "any ◆ → Refactor route");
    let pid = outcome.proposals[0].clone();
    let proposal = s.state.proposals.get(&pid).unwrap().clone();
    assert_eq!(proposal.source, ProposalSource::Refactor);

    // The ◇ replacement's CREATE group adopts the PINNED alt recipe class.
    assert!(
        proposal.items.iter().any(|it| it
            .commands
            .iter()
            .any(|c| matches!(c, Command::AddGroup { recipe, .. } if recipe == &alt))),
        "the ◇ replacement is solved onto the pinned alt recipe"
    );
    // The pin overrode the cheaper STANDARD recipe — it is nowhere in the draft.
    assert!(
        !proposal.items.iter().any(|it| it.commands.iter().any(
            |c| matches!(c, Command::AddGroup { recipe, .. } if recipe == "Recipe_IngotIron_C")
        )),
        "the pin overrode the cheaper standard recipe (never staged)"
    );
    // The pin lives in the ◇ replacement's goal ONLY — the ◆ layer never mutates.
    assert!(
        !proposal.items.iter().any(|it| it
            .commands
            .iter()
            .any(|c| matches!(c, Command::SetGroupRecipe { .. }))),
        "a refactor never emits SetGroupRecipe onto the ◆"
    );
    let after = s.state.groups[&old_gid].clone();
    assert_eq!(
        before, after,
        "the ◆ built group is byte-identical after the pinned refactor"
    );
}

/// T3(a) — an all-◇ opportunity whose alt ingredients NO factory can locally
/// source is honest degradation, not an error: `optimize_adopt` returns empty
/// proposals + a note (never `Err`).
#[test]
fn no_local_source_yields_note_not_err() {
    let mut s = Session::in_memory(None).unwrap();
    let alt = inject_ingot_alt(&mut s);
    // A ◇ planned ingot factory on the STANDARD recipe — but with NO ore feed
    // (no IN port, no ore-producing group), so the alt's ore is unsourceable here.
    let fid = s
        .edit(vec![Command::CreateFactory {
            name: "STARVED INGOTS".into(),
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
    s.edit(vec![Command::AddGroup {
        factory: fid,
        machine: "Build_SmelterMk1_C".into(),
        recipe: "Recipe_IngotIron_C".into(),
        count: 4,
        clock: 1.0,
        graph_pos: GraphPos { x: 0.0, y: 0.0 },
        floor: 0,
    }])
    .unwrap();

    let outcome = s.optimize_adopt(&alt).unwrap();
    assert_eq!(outcome.route, "t2", "all-◇ dead-end still routes t2");
    assert!(
        outcome.proposals.is_empty(),
        "no locally-sourceable factory → no drafted proposal"
    );
    assert!(
        outcome.note.is_some(),
        "the dead-end is surfaced as a note, not swallowed as an Err"
    );
}

/// T3(b) — a negative-savings alt (needs MORE machines) is filtered from
/// `empire_optimize` at the `machines_saved <= 0` gate. The factory CAN source
/// the alt (an IN port is present) so the per-factory gate does NOT prune it —
/// isolating the savings filter as the reason it never surfaces.
#[test]
fn negative_savings_alt_filtered() {
    let mut gd = GameData::default();
    // Standard X: 4/min from ore (1 machine covers a 4/min group). Unlocked alt:
    // 1/min → 4 machines for the same rate → a machine LOSS.
    gd.recipes.insert(
        "Recipe_X_Std_C".into(),
        recipe(
            "Recipe_X_Std_C",
            "Desc_X_C",
            4.0,
            60.0,
            false,
            vec![("Desc_OreX_C", 1.0)],
            5.0,
        ),
    );
    gd.recipes.insert(
        "Recipe_X_Alt_C".into(),
        recipe(
            "Recipe_X_Alt_C",
            "Desc_X_C",
            1.0,
            60.0,
            true,
            vec![("Desc_OreX_C", 1.0)],
            2.0,
        ),
    );
    let mut state = PlanState::default();
    let gx = planned_group(&mut state, "Recipe_X_Std_C", 1, 1.0);
    add_in_port(&mut state, &gx, "Desc_OreX_C"); // sourceable — not source-pruned

    assert!(
        empire_optimize(&state, &gd, &unlocked(&["Recipe_X_Alt_C"])).is_empty(),
        "a negative-savings alt is filtered at the machines_saved <= 0 gate"
    );
}
