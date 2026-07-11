//! Advisor + chat: the non-naggy contract at the session level — cards fire on
//! newly-armed events only, dismiss mutes the rule persistently, and chat
//! intents materialize through the solver as reviewable proposals.

use app::Session;
use planner_core::commands::Command;
use planner_core::entities::*;

fn gp(x: f64, y: f64) -> GraphPos {
    GraphPos { x, y }
}

/// Two factories where dropping the upstream target starves the downstream.
fn build_starvable(s: &mut Session) -> (Id, Id) {
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
    let a = mk(s, "UP", 0.0);
    let b = mk(s, "DOWN", 400.0);
    let ore_in = s
        .edit(vec![Command::AddPort {
            factory: a.clone(),
            direction: PortDirection::In,
            item: "Desc_OreIron_C".into(),
            rate: 0.0,
            rate_ceiling: Some(120.0),
            graph_pos: gp(0.0, 100.0),
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
            graph_pos: gp(600.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let smelt = s
        .edit(vec![Command::AddGroup {
            factory: a.clone(),
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
            EdgeEnd::Port(out.clone()),
            "Desc_IronIngot_C",
        ),
    ] {
        s.edit(vec![Command::AddEdge {
            factory: a.clone(),
            from,
            to,
            item: item.into(),
            tier: 3,
        }])
        .unwrap();
    }
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
    let rod_out = s
        .edit(vec![Command::AddPort {
            factory: b.clone(),
            direction: PortDirection::Out,
            item: "Desc_IronRod_C".into(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: gp(600.0, 100.0),
        }])
        .unwrap()
        .created[0]
        .clone();
    let g = s
        .edit(vec![Command::AddGroup {
            factory: b.clone(),
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
            EdgeEnd::Port(inp.clone()),
            EdgeEnd::Group(g.clone()),
            "Desc_IronIngot_C",
        ),
        (
            EdgeEnd::Group(g),
            EdgeEnd::Port(rod_out.clone()),
            "Desc_IronRod_C",
        ),
    ] {
        s.edit(vec![Command::AddEdge {
            factory: b.clone(),
            from,
            to,
            item: item.into(),
            tier: 2,
        }])
        .unwrap();
    }
    s.edit(vec![Command::AddRoute {
        kind: RouteKind::Belt { tier: 2 },
        from: out.clone(),
        to: inp,
        path: vec![
            MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            MapPos {
                x: 400.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    }])
    .unwrap();
    s.edit(vec![Command::SetPortRate {
        id: out.clone(),
        rate: 30.0,
    }])
    .unwrap();
    s.edit(vec![Command::SetPortRate {
        id: rod_out,
        rate: 30.0,
    }])
    .unwrap();
    (a, out)
}

#[test]
fn advisor_fires_on_new_deficit_and_dismiss_mutes() {
    let mut s = Session::in_memory(None).unwrap();
    let (_a, out) = build_starvable(&mut s);
    assert!(
        s.advisor.feed(false).cards.is_empty(),
        "healthy empire, silent advisor: {:?}",
        s.advisor.feed(false).cards
    );

    // upstream dips → deficit → exactly one card, with a solver CTA
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: out.clone(),
            rate: 10.0,
        }])
        .unwrap();
    let deficit_cards: Vec<_> = resp
        .advisor
        .cards
        .iter()
        .filter(|c| c.rule == "new_deficit")
        .collect();
    assert_eq!(deficit_cards.len(), 1, "one card per new condition");
    assert!(deficit_cards[0].saw.contains("deficit"));
    assert!(matches!(
        deficit_cards[0].cta,
        Some(app::advisor::CardCta::PlanProduction { .. })
    ));

    // dismissing mutes the rule: recreate the condition → no new card
    let card_id = deficit_cards[0].id.clone();
    let feed = s.advisor_dismiss(&card_id);
    assert!(feed.muted.contains(&"new_deficit".to_string()));
    s.edit(vec![Command::SetPortRate {
        id: out.clone(),
        rate: 30.0,
    }])
    .unwrap(); // clears
    let resp = s
        .edit(vec![Command::SetPortRate { id: out, rate: 5.0 }])
        .unwrap(); // re-arms
    assert!(
        !resp
            .advisor
            .cards
            .iter()
            .any(|c| c.rule == "new_deficit" && !c.dismissed),
        "muted rule stays silent even for brand-new conditions"
    );
}

#[test]
fn chat_intent_drafts_a_reviewable_proposal() {
    let mut s = Session::in_memory(None).unwrap();
    let reply = app::chat::chat(
        &mut s,
        &app::chat::ContextScope::Empire,
        "produce Iron Rod at 10/min",
    );
    let pid = reply.proposal.expect("intent → proposal");
    assert!(reply.reply.contains("PROPOSAL #1"));
    assert_eq!(
        s.state.proposals[&pid].source,
        planner_core::proposals::ProposalSource::Chat
    );
    // nothing applied: the plan has no factories until the proposal is accepted
    assert!(s.state.factories.is_empty());

    // status queries answer with causal lines, offline engine flagged
    let reply = app::chat::chat(&mut s, &app::chat::ContextScope::Empire, "any deficits?");
    assert_eq!(reply.engine, "offline");
    assert!(reply.reply.contains("No deficits"));
    // context snapshot is honest about its size
    let ctx = app::chat::compact_state(&mut s, &app::chat::ContextScope::Empire);
    assert!(
        ctx.bytes > 0 && ctx.bytes < 30_000,
        "compact: {}",
        ctx.bytes
    );
}
