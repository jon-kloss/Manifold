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
    let (a, out, _route) = build_chain(s, 120.0, 2, 30.0, 30.0);
    (a, out)
}

/// A two-factory ore→ingot→rod chain with a tunable inter-factory belt:
/// upstream smelts behind an ore In ceiling of `ore_ceiling`, a Mk.`route_tier`
/// belt ships the ingots, downstream constructs rods. Output targets land
/// last (upstream `up_rate`, downstream `down_rate`), so route saturation and
/// any deficit become true on the final edit. Returns (upstream factory,
/// upstream out port, route id).
fn build_chain(
    s: &mut Session,
    ore_ceiling: f64,
    route_tier: u8,
    up_rate: f64,
    down_rate: f64,
) -> (Id, Id, Id) {
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
            rate_ceiling: Some(ore_ceiling),
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
    let route = s
        .edit(vec![Command::AddRoute {
            kind: RouteKind::Belt { tier: route_tier },
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
        .unwrap()
        .created[0]
        .clone();
    s.edit(vec![Command::SetPortRate {
        id: out.clone(),
        rate: up_rate,
    }])
    .unwrap();
    s.edit(vec![Command::SetPortRate {
        id: rod_out,
        rate: down_rate,
    }])
    .unwrap();
    (a, out, route)
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

/// Efficiency grammar (route_bottleneck): a route at full capacity WITH a
/// deficit registered through it is causal attribution — one amber card.
#[test]
fn route_bottleneck_fires_on_deficit_through_a_full_route() {
    let mut s = Session::in_memory(None).unwrap();
    // Healthy 120/min chain over a Mk.2 belt, then the link is downgraded to
    // Mk.1 (cap 60): the recompute starves downstream THROUGH the now-full
    // route — the route itself caps demand. (Targets are set while feasible:
    // an explicitly edited target that clamps is written back, so the honest
    // starve path is a later Recompute.)
    let (_a, _out, route) = build_chain(&mut s, 120.0, 2, 120.0, 120.0);
    assert!(s.advisor.cards.is_empty(), "healthy chain, silent advisor");
    s.edit(vec![Command::SetRouteTier {
        id: route.clone(),
        tier: 1,
    }])
    .unwrap();
    let cards: Vec<_> = s
        .advisor
        .cards
        .iter()
        .filter(|c| c.rule == "route_bottleneck")
        .collect();
    assert_eq!(
        cards.len(),
        1,
        "exactly one bottleneck card: {:?}",
        s.advisor.cards
    );
    assert_eq!(
        cards[0].severity,
        app::advisor::Severity::Trend,
        "causal attribution rides amber — the starve itself is the red card"
    );
    assert!(
        matches!(&cards[0].cta, Some(app::advisor::CardCta::Trace { id, .. }) if *id == route),
        "CTA traces the capping route"
    );
    assert!(
        s.advisor.cards.iter().any(|c| c.rule == "new_deficit"),
        "the starve still reports at Conflict"
    );
}

/// The old ≥75% grammar's kill shot: a full route whose consumers are all
/// satisfied is OPTIMAL — the advisor stays silent.
#[test]
fn full_route_meeting_demand_stays_silent() {
    let mut s = Session::in_memory(None).unwrap();
    // 60/min through a Mk.1 belt (cap 60): saturation 1.0, demand met.
    let (_a, _out, route) = build_chain(&mut s, 120.0, 1, 60.0, 60.0);
    let derived = s.solve_all_readonly();
    assert!(
        derived.routes[&route].saturation >= 0.999,
        "the scenario really runs full: {} at {}",
        route,
        derived.routes[&route].saturation
    );
    assert!(
        s.advisor.cards.is_empty(),
        "full route meeting demand must not alarm: {:?}",
        s.advisor.cards
    );
}

/// A deficit over a SLACK route is an upstream production problem — the
/// starve card fires, the bottleneck card must not blame the link.
#[test]
fn starved_but_slack_route_blames_upstream() {
    let mut s = Session::in_memory(None).unwrap();
    let (_a, out, _route) = build_chain(&mut s, 120.0, 2, 30.0, 30.0);
    // Upstream dips to 10/min on the Mk.2 belt (cap 120) — plenty of slack.
    s.edit(vec![Command::SetPortRate {
        id: out,
        rate: 10.0,
    }])
    .unwrap();
    assert!(
        s.advisor.cards.iter().any(|c| c.rule == "new_deficit"),
        "starve reports: {:?}",
        s.advisor.cards
    );
    assert!(
        !s.advisor.cards.iter().any(|c| c.rule == "route_bottleneck"),
        "slack route is not the culprit: {:?}",
        s.advisor.cards
    );
}

/// Pins FULL (0.999): 95% saturation with a real deficit through the route is
/// still not bottleneck evidence — loosening the threshold to 0.9-ish would
/// re-introduce the old ≥95% congestion grammar.
#[test]
fn nearly_full_route_with_deficit_stays_silent() {
    let mut s = Session::in_memory(None).unwrap();
    // Healthy 120/min chain, then upstream dips to 57/min and the link is
    // downgraded to Mk.1 (cap 60): 57/60 = 95% saturation with a real
    // deficit through the route — still not FULL.
    let (_a, out, route) = build_chain(&mut s, 120.0, 2, 120.0, 120.0);
    s.edit(vec![Command::SetPortRate {
        id: out,
        rate: 57.0,
    }])
    .unwrap();
    s.edit(vec![Command::SetRouteTier { id: route, tier: 1 }])
        .unwrap();
    assert!(
        s.advisor.cards.iter().any(|c| c.rule == "new_deficit"),
        "starve reports: {:?}",
        s.advisor.cards
    );
    assert!(
        !s.advisor.cards.iter().any(|c| c.rule == "route_bottleneck"),
        "95% is not full: {:?}",
        s.advisor.cards
    );
}

#[cfg(feature = "sqlite")]
#[test]
fn restart_does_not_refire_still_true_conditions() {
    // M18: arming state persists — a deficit reported before shutdown must
    // not produce a duplicate card on the next launch while it is still true.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.ficsit");
    let a = {
        let mut s = Session::open(&path, None, "fixture").unwrap();
        let (a, out) = build_starvable(&mut s);
        let resp = s
            .edit(vec![Command::SetPortRate {
                id: out,
                rate: 10.0,
            }])
            .unwrap();
        assert_eq!(
            resp.advisor
                .cards
                .iter()
                .filter(|c| c.rule == "new_deficit")
                .count(),
            1,
            "deficit fires once before shutdown"
        );
        a
    }; // session dropped — "app closed"

    let mut s = Session::open(&path, None, "fixture").unwrap();
    assert_eq!(
        s.advisor
            .feed(false)
            .cards
            .iter()
            .filter(|c| c.rule == "new_deficit")
            .count(),
        1,
        "persisted card survives the restart"
    );
    // trivial edit → advise() runs over the still-true deficit
    let resp = s
        .edit(vec![Command::RenameFactory {
            id: a,
            name: "UPSTREAM".into(),
        }])
        .unwrap();
    assert_eq!(
        resp.advisor
            .cards
            .iter()
            .filter(|c| c.rule == "new_deficit")
            .count(),
        1,
        "still-true condition must not re-fire a duplicate card after restart"
    );
}

#[test]
fn chat_intent_drafts_a_reviewable_proposal() {
    let mut s = Session::in_memory(None).unwrap();
    // De-flake: the "offline" assertion below must not depend on whatever
    // FICSIT_AI_* the host environment happens to export.
    s.ai = app::ai::AiConfig::from_lookup(|_| None);
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

#[test]
fn failed_edit_persists_no_advisor_cards() {
    // M9 companion: advise() runs only after a durable commit, so an edit
    // whose persist fails must not gate or persist cards for a state that
    // never existed.
    let mut s = Session::in_memory(None).unwrap();
    let (_a, out) = build_starvable(&mut s);
    assert!(s.advisor.cards.is_empty(), "healthy empire, no cards");
    let disk_cards_before = s.store.load_advisor_cards().unwrap().len();

    s.store.faults_mut().fail_commits = 1;
    assert!(s
        .edit(vec![Command::SetPortRate {
            id: out.clone(),
            rate: 10.0,
        }])
        .is_err());
    assert!(
        s.advisor.cards.is_empty(),
        "no phantom in-memory cards: {:?}",
        s.advisor.cards
    );
    assert_eq!(
        s.store.load_advisor_cards().unwrap().len(),
        disk_cards_before,
        "no phantom persisted cards"
    );

    // The same edit, persisted, produces the deficit card as usual.
    let resp = s
        .edit(vec![Command::SetPortRate {
            id: out,
            rate: 10.0,
        }])
        .unwrap();
    assert!(resp.advisor.cards.iter().any(|c| c.rule == "new_deficit"));
    assert!(s.store.load_advisor_cards().unwrap().len() > disk_cards_before);
}

/// Review minor M12: rate-parse failures must not be misreported as item-match
/// failures, and trailing words / comma decimals must not defeat the parse.
#[test]
fn chat_rate_parse_is_forgiving_and_errors_name_the_right_culprit() {
    let mut s = Session::in_memory(None).unwrap();

    // Trailing words after the rate no longer defeat the "/min" strip.
    let reply = app::chat::chat(
        &mut s,
        &app::chat::ContextScope::Empire,
        "produce iron rod at 30/min please",
    );
    assert!(
        reply.proposal.is_some(),
        "trailing words parse: {}",
        reply.reply
    );

    // Comma decimal is accepted as a courtesy.
    let reply = app::chat::chat(
        &mut s,
        &app::chat::ContextScope::Empire,
        "produce iron rod at 22,5/min",
    );
    assert!(
        reply.proposal.is_some(),
        "comma decimal parses: {}",
        reply.reply
    );

    // Item matched but rate garbage → the reply blames the RATE, not the item.
    let reply = app::chat::chat(
        &mut s,
        &app::chat::ContextScope::Empire,
        "produce iron rod at lots/min",
    );
    assert!(reply.proposal.is_none());
    assert!(
        reply.reply.contains("rate") || reply.reply.contains("positive"),
        "blames the rate: {}",
        reply.reply
    );
    assert!(
        !reply.reply.contains("couldn't match"),
        "must not blame the item: {}",
        reply.reply
    );

    // Item genuinely unknown → the item-match reply is still the right one.
    let reply = app::chat::chat(
        &mut s,
        &app::chat::ContextScope::Empire,
        "produce unobtainium at 30/min",
    );
    assert!(reply.proposal.is_none());
    assert!(reply.reply.contains("couldn't match"), "{}", reply.reply);
}

/// A drift card's REVIEW CTA is only actionable while its proposal is open:
/// closing the proposal drops the card at the feed boundary — derived, zero
/// writes — while every other card kind and the mute set ride through, and
/// the undo that reopens the proposal revives the card for free.
#[test]
fn feed_drops_review_cards_whose_proposal_closed() {
    use app::import::{ImportMachine, ImportSnapshot};
    use app::session::ImportOutcome;
    use planner_core::proposals::ProposalStatus;

    let mut s = Session::in_memory(None).unwrap();
    // A non-Review card to ride through: starve the rod line.
    let (_a, out) = build_starvable(&mut s);
    s.edit(vec![Command::SetPortRate {
        id: out,
        rate: 10.0,
    }])
    .unwrap();
    // A drift card with a Review CTA: import a built layer, then re-import
    // a grown save.
    let mch = |x: f64| ImportMachine {
        class: "Build_SmelterMk1_C".into(),
        recipe: Some("Recipe_IngotIron_C".into()),
        clock: 1.0,
        x,
        y: 90000.0,
        z: 0.0,
        ..Default::default()
    };
    let snap = |n: usize| ImportSnapshot {
        save_name: "DRIFT".into(),
        machines: (0..n).map(|i| mch(50.0 * i as f64)).collect(),
        ..Default::default()
    };
    s.import_save(snap(2)).unwrap();
    let ImportOutcome::Drift { proposal, .. } = s.import_save(snap(3)).unwrap() else {
        panic!("expected drift");
    };
    let feed = s.advisor_feed();
    assert!(
        feed.cards.iter().any(
            |c| matches!(&c.cta, Some(app::advisor::CardCta::Review { proposal: p }) if *p == proposal)
        ),
        "drift card in the feed: {:?}",
        feed.cards
    );
    assert!(
        feed.cards.iter().any(|c| c.rule == "new_deficit"),
        "non-Review card present: {:?}",
        feed.cards
    );

    // Manual reject closes the proposal → the card expires out of the feed.
    let resp = s
        .edit(vec![Command::SetProposalStatus {
            id: proposal.clone(),
            status: ProposalStatus::Rejected,
        }])
        .unwrap();
    for feed in [&resp.advisor, &s.advisor_feed()] {
        assert!(
            !feed
                .cards
                .iter()
                .any(|c| matches!(&c.cta, Some(app::advisor::CardCta::Review { .. }))),
            "closed proposal, no Review card: {:?}",
            feed.cards
        );
        assert!(
            feed.cards.iter().any(|c| c.rule == "new_deficit"),
            "other card kinds unaffected"
        );
        assert!(feed.muted.is_empty(), "expiry is not a mute");
    }
    // Zero writes: the card itself was never dismissed — undoing the reject
    // reopens the proposal and the card comes straight back.
    assert!(s.advisor.cards.iter().all(|c| !c.dismissed));
    s.undo().unwrap().unwrap();
    assert!(
        s.advisor_feed().cards.iter().any(
            |c| matches!(&c.cta, Some(app::advisor::CardCta::Review { proposal: p }) if *p == proposal)
        ),
        "reopened proposal revives the card"
    );
}

/// A comma is a decimal comma ("22,5" → 22.5) OR thousands grouping
/// ("1,000" → 1000) by shape — either way the drafted goal must carry the
/// magnitude the user wrote, not a 45×/1000× misread.
#[test]
fn chat_rate_commas_preserve_magnitude() {
    let mut s = Session::in_memory(None).unwrap();

    let reply = app::chat::chat(
        &mut s,
        &app::chat::ContextScope::Empire,
        "produce iron rod at 22,5/min",
    );
    let pid = reply.proposal.expect("decimal comma drafts");
    assert_eq!(
        s.state.proposals[&pid].goal,
        vec![("Desc_IronRod_C".to_string(), 22.5)],
        "22,5 is twenty-two and a half"
    );

    let reply = app::chat::chat(
        &mut s,
        &app::chat::ContextScope::Empire,
        "produce iron rod at 1,000/min",
    );
    match reply.proposal {
        // Feasible in this catalog → the goal carries the grouped magnitude.
        Some(pid) => assert_eq!(
            s.state.proposals[&pid].goal,
            vec![("Desc_IronRod_C".to_string(), 1000.0)],
            "1,000 is one thousand"
        ),
        // Infeasible at 1000/min is fine — the provenance line still has to
        // show the solver was asked for one thousand, not one.
        None => assert!(
            reply.saw.contains("1000.0/min"),
            "goal magnitude honest: {}",
            reply.saw
        ),
    }
}
