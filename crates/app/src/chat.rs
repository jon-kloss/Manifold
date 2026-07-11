//! Chat + context serializer (SDD §9). The context snapshot is exactly what a
//! model would see — dense, aggregated, user-viewable, size shown in the bar.
//! Offline (no key) the chat runs a small deterministic engine: status
//! questions answer from derived state with SAW provenance, and
//! "produce <item> at <rate>/min" intents materialize through the SAME global
//! solver validation path a model's `proposal_intent` would use. The model
//! never mutates state either way — it can only draft goals the solver solves.

use planner_core::entities::{Id, PortDirection};
use serde::{Deserialize, Serialize};

use crate::session::Session;
use crate::wizard::{global_solve, WizardGoal, WizardOutcome};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "scope")]
pub enum ContextScope {
    Empire,
    Factory { id: Id },
    Selection { id: Id },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSnapshot {
    pub payload: serde_json::Value,
    /// Exact serialized size — the honesty number in the context bar.
    pub bytes: usize,
    pub snapshot_time: String,
}

pub fn compact_state(s: &mut Session, scope: &ContextScope) -> ContextSnapshot {
    let derived = s.solve_all_readonly();
    let payload = match scope {
        ContextScope::Empire => {
            let factories: Vec<serde_json::Value> = s
                .state
                .factories
                .values()
                .map(|f| {
                    let df = derived.factories.get(&f.id);
                    let outputs: serde_json::Map<String, serde_json::Value> = f
                        .ports
                        .iter()
                        .filter_map(|pid| s.state.ports.get(pid))
                        .filter(|p| p.direction == PortDirection::Out)
                        .map(|p| {
                            let rate = df.and_then(|d| d.ports.get(&p.id)).copied().unwrap_or(0.0);
                            (p.item.clone(), serde_json::json!(rate))
                        })
                        .collect();
                    serde_json::json!({
                        "id": f.id,
                        "name": f.name,
                        "status": f.status,
                        "powerMw": df.map(|d| d.total_power_mw).unwrap_or(0.0),
                        "outputs": outputs,
                    })
                })
                .collect();
            serde_json::json!({
                "scope": "empire",
                "factories": factories,
                "deficits": derived.deficits.iter().take(10).collect::<Vec<_>>(),
                "circuits": derived.circuits,
                "totals": {
                    "factories": s.state.factories.len(),
                    "groups": s.state.groups.len(),
                    "routes": s.state.routes.len(),
                    "drawMw": derived.total_power_mw,
                    "generationMw": derived.total_generation_mw,
                },
            })
        }
        ContextScope::Factory { id } | ContextScope::Selection { id } => {
            let f = s.state.factories.get(id);
            let df = derived.factories.get(id);
            serde_json::json!({
                "scope": "factory",
                "factory": f,
                "groups": f.map(|f| f.groups.iter().filter_map(|g| s.state.groups.get(g)).collect::<Vec<_>>()),
                "ports": f.map(|f| f.ports.iter().filter_map(|p| s.state.ports.get(p)).collect::<Vec<_>>()),
                "derived": df,
            })
        }
    };
    let bytes = payload.to_string().len();
    ContextSnapshot {
        payload,
        bytes,
        snapshot_time: crate::jobs::now_rfc3339(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatReply {
    pub reply: String,
    /// Causal-chain block lines: (severity ok|warn|crit, mono text).
    pub causal: Vec<(String, String)>,
    /// Clickable entity chips: (display name, selection kind, id).
    pub entities: Vec<(String, String, Id)>,
    /// Set when the answer drafted a proposal (review to apply).
    pub proposal: Option<Id>,
    /// Provenance: what the engine saw.
    pub saw: String,
    /// "offline" | "ready" — offline answers come from the heuristic engine.
    pub engine: String,
}

/// The offline chat engine. With a key, a model would produce the prose and
/// `proposal_intent` blocks — the materialization path below stays identical.
pub fn chat(s: &mut Session, _scope: &ContextScope, message: &str) -> ChatReply {
    let msg = message.to_lowercase();
    let engine = if s.ai_key.is_some() {
        "ready"
    } else {
        "offline"
    };
    let derived = s.solve_all_readonly();
    let saw_base = format!(
        "{} factories · {} routes · snapshot {}",
        s.state.factories.len(),
        s.state.routes.len(),
        crate::jobs::now_rfc3339()
    );

    // ---- proposal_intent: "produce <item> at <rate>[/min]" ----
    if let Some(rest) = msg
        .strip_prefix("produce ")
        .or_else(|| msg.find("produce ").map(|i| &msg[i + "produce ".len()..]))
    {
        if let Some((item_part, rate_part)) = rest.split_once(" at ") {
            let rate: f64 = rate_part
                .trim()
                .trim_end_matches("/min")
                .split_whitespace()
                .next()
                .and_then(|t| t.parse().ok())
                .unwrap_or(0.0);
            let item = s
                .gamedata
                .items
                .values()
                .find(|i| i.display_name.to_lowercase() == item_part.trim())
                .or_else(|| {
                    s.gamedata
                        .items
                        .values()
                        .find(|i| i.display_name.to_lowercase().contains(item_part.trim()))
                })
                .map(|i| (i.class_name.clone(), i.display_name.clone()));
            if let (Some((class, display)), true) = (item, rate > 0.0) {
                return intent_to_proposal(s, &class, &display, rate, engine);
            }
            return ChatReply {
                reply: format!(
                    "I couldn't match \"{}\" to an item in the catalog. Try the exact item name, \
                     e.g. \"produce Iron Rod at 30/min\".",
                    item_part.trim()
                ),
                causal: vec![],
                entities: vec![],
                proposal: None,
                saw: saw_base,
                engine: engine.into(),
            };
        }
    }

    // ---- power status ----
    if msg.contains("power") || msg.contains("grid") || msg.contains("mw") {
        let mut causal = Vec::new();
        for c in &derived.circuits {
            let headroom = if c.generation_mw > 0.0 {
                (c.generation_mw - c.demand_mw) / c.generation_mw
            } else {
                -1.0
            };
            let sev = if headroom < 0.05 {
                "crit"
            } else if headroom < 0.2 {
                "warn"
            } else {
                "ok"
            };
            causal.push((
                sev.to_string(),
                format!(
                    "{}: {:.0} MW draw / {:.0} MW gen · {:.0}% headroom",
                    c.name,
                    c.demand_mw,
                    c.generation_mw,
                    headroom.max(0.0) * 100.0
                ),
            ));
        }
        let reply = if causal.is_empty() {
            format!(
                "No grids yet — total draw is {:.0} MW, unsourced. Draw a ⚡ power line between \
                 factories to form one.",
                derived.total_power_mw
            )
        } else {
            format!(
                "Empire draw {:.0} MW against {:.0} MW generation across {} grid(s).",
                derived.total_power_mw,
                derived.total_generation_mw,
                derived.circuits.len()
            )
        };
        return ChatReply {
            reply,
            causal,
            entities: vec![],
            proposal: None,
            saw: saw_base,
            engine: engine.into(),
        };
    }

    // ---- deficit status ----
    if msg.contains("deficit") || msg.contains("starv") || msg.contains("short") {
        let mut causal = Vec::new();
        let mut entities = Vec::new();
        for d in derived.deficits.iter().take(8) {
            let name = s
                .state
                .factories
                .get(&d.factory)
                .map(|f| f.name.clone())
                .unwrap_or_default();
            causal.push((
                "crit".to_string(),
                format!(
                    "{name} short {:.1}/min of {} (needs {:.1}, gets {:.1})",
                    d.needed - d.supplied,
                    d.item,
                    d.needed,
                    d.supplied
                ),
            ));
            entities.push((name, "factory".to_string(), d.factory.clone()));
        }
        let reply = if causal.is_empty() {
            "No deficits — every target is fed.".to_string()
        } else {
            format!(
                "{} deficit(s). Say \"produce <item> at <rate>/min\" and I'll draft the fix as a proposal.",
                causal.len()
            )
        };
        return ChatReply {
            reply,
            causal,
            entities,
            proposal: None,
            saw: saw_base,
            engine: engine.into(),
        };
    }

    ChatReply {
        reply: if engine == "offline" {
            "AI OFFLINE — the heuristic engine answers: \"power\", \"deficits\", or \
             \"produce <item> at <rate>/min\" (drafts a reviewable proposal)."
                .into()
        } else {
            "Model relay not wired in this build — the heuristic engine answers: \"power\", \
             \"deficits\", or \"produce <item> at <rate>/min\"."
                .into()
        },
        causal: vec![],
        entities: vec![],
        proposal: None,
        saw: saw_base,
        engine: engine.into(),
    }
}

/// The `proposal_intent` materialization path: the intent becomes a wizard
/// goal, the global solver validates it, and the result is an ordinary
/// reviewable proposal — real consequences, not model arithmetic.
fn intent_to_proposal(
    s: &mut Session,
    item_class: &str,
    display: &str,
    rate: f64,
    engine: &str,
) -> ChatReply {
    let goal = WizardGoal {
        items: vec![(item_class.to_string(), rate)],
        constraints: Default::default(),
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut log_lines = 0usize;
    let outcome = global_solve(
        &s.state,
        &s.gamedata,
        &s.world,
        &goal,
        s.plan_hash(),
        crate::jobs::now_rfc3339(),
        |_, _| log_lines += 1,
        &cancel,
    );
    match outcome {
        WizardOutcome::Proposal { mut proposal } => {
            proposal.source = planner_core::proposals::ProposalSource::Chat;
            proposal.provenance = "CHAT INTENT · GLOBAL SOLVER".into();
            let resp = s.edit(vec![planner_core::commands::Command::CreateProposal {
                proposal,
            }]);
            match resp {
                Ok(r) => {
                    let pid = r.created[0].clone();
                    let number = s.state.proposals.get(&pid).map(|p| p.number).unwrap_or(0);
                    ChatReply {
                        reply: format!(
                            "Drafted PROPOSAL #{number} — produce {display} at {rate:.1}/min. \
                             Nothing applies until you review and accept it."
                        ),
                        causal: vec![],
                        entities: vec![],
                        proposal: Some(pid),
                        saw: format!(
                            "goal {display} {rate:.1}/min → global solver ({log_lines} log lines)"
                        ),
                        engine: engine.into(),
                    }
                }
                Err(e) => ChatReply {
                    reply: format!("The solver drafted a proposal but storing it failed: {e}"),
                    causal: vec![],
                    entities: vec![],
                    proposal: None,
                    saw: String::new(),
                    engine: engine.into(),
                },
            }
        }
        WizardOutcome::Infeasible(inf) => ChatReply {
            reply: format!(
                "Infeasible: {}. Best achievable is {:.1}/min. Relaxations: {}",
                inf.binding,
                inf.best_rate,
                inf.relaxations.join(" · ")
            ),
            causal: vec![("warn".into(), inf.binding.clone())],
            entities: vec![],
            proposal: None,
            saw: format!("goal {display} {rate:.1}/min → infeasible"),
            engine: engine.into(),
        },
        WizardOutcome::Cancelled => ChatReply {
            reply: "Solve cancelled.".into(),
            causal: vec![],
            entities: vec![],
            proposal: None,
            saw: String::new(),
            engine: engine.into(),
        },
    }
}
