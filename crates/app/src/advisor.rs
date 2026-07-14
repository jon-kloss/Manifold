//! Ambient advisor (SDD §9, UI spec screen 3). Heuristic rules are pure
//! functions over derived state; the gate keeps it quiet: a card fires only
//! when a condition BECOMES true (new-event arming), and each such true-edge
//! is reported at most once — the per-rule 30 s debounce and pause DELAY the
//! report to a later gate pass, they never cancel it. Muting a rule
//! permanently silences its conditions (they count as seen, so unmuting never
//! ambushes with cards for conditions that were true the whole time). Offline
//! / no key, the same heuristics feed the same cards — a model call would only
//! rewrite the prose, so the budget is tracked and displayed but unspent.
//! Silence is a feature: the advisor's loudest voice is a badge count.

use std::collections::{BTreeMap, BTreeSet};

use planner_core::entities::Id;
use serde::{Deserialize, Serialize};

use crate::session::Derived;
use planner_core::state::PlanState;

pub const DEBOUNCE_S: u64 = 30;
/// Visible hourly model-call budget (A: "visible hourly call budget").
pub const HOURLY_CALL_BUDGET: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// ⚠ red — something is wrong right now.
    Conflict,
    /// ▲ amber — heading somewhere bad.
    Trend,
    /// ● gray — worth knowing.
    Tip,
}

/// A card's call-to-action: everything routes through existing review
/// surfaces — the advisor never edits the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CardCta {
    /// Pre-fill the wizard (FIX WITH SOLVER pattern).
    PlanProduction { item: String, rate: f64 },
    /// Select an entity on the map / in a factory.
    Trace { selection: String, id: Id },
    /// Open a proposal review (drift).
    Review { proposal: Id },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorCard {
    pub id: Id,
    pub severity: Severity,
    pub title: String,
    pub body: String,
    /// Heuristic rule id — the mute key.
    pub rule: String,
    /// Provenance: exactly what the rule saw, rendered in the footer.
    pub saw: String,
    /// RFC3339 creation time.
    pub at: String,
    pub dismissed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cta: Option<CardCta>,
}

/// One armed event = one prospective card. `key` identifies the CONDITION
/// (not the rule) so re-arming only happens when a condition newly appears.
pub struct Event {
    pub key: String,
    pub rule: &'static str,
    pub severity: Severity,
    pub title: String,
    pub body: String,
    pub saw: String,
    pub cta: Option<CardCta>,
}

/// Evaluate every heuristic over the current state. Pure: no gating here.
pub fn evaluate(state: &PlanState, derived: &Derived) -> Vec<Event> {
    let mut events = Vec::new();
    let fname = |id: &Id| -> String {
        state
            .factories
            .get(id)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| id.clone())
    };

    // NodeConflict — intentional double-booking renders CRIT until resolved
    for (node, n) in &derived.nodes {
        if n.conflict {
            events.push(Event {
                key: format!("conflict:{node}"),
                rule: "node_conflict",
                severity: Severity::Conflict,
                title: format!("Node {node} is double-booked"),
                body: format!(
                    "{} claims share this node — combined extraction exceeds what it yields. \
                     One factory will starve in practice.",
                    n.claims
                ),
                saw: format!("{} claims on {node}", n.claims),
                cta: Some(CardCta::Trace {
                    selection: "node".into(),
                    id: node.clone(),
                }),
            });
        }
    }

    // NewDeficit — a target somewhere is not being fed
    for d in &derived.deficits {
        let short = d.needed - d.supplied;
        events.push(Event {
            key: format!("deficit:{}:{}", d.factory, d.item),
            rule: "new_deficit",
            severity: Severity::Conflict,
            title: format!("{} is starved of {}", fname(&d.factory), d.item),
            body: format!(
                "It needs {:.1}/min but upstream ships {:.1}/min — {:.1}/min short. \
                 The solver can plan the missing production.",
                d.needed, d.supplied, short
            ),
            saw: format!(
                "deficit {}: need {:.1}, supplied {:.1}",
                d.item, d.needed, d.supplied
            ),
            cta: Some(CardCta::PlanProduction {
                item: d.item.clone(),
                rate: short.ceil().max(1.0),
            }),
        });
    }

    // SaturationHigh — any route ≥75% is a trend worth flagging
    for (rid, dr) in &derived.routes {
        if dr.saturation >= 0.75 {
            events.push(Event {
                key: format!("sat:{rid}"),
                rule: "saturation_high",
                severity: Severity::Trend,
                title: "A route is running hot".into(),
                body: format!(
                    "{:.0}% of capacity ({:.1}/{:.1} per min). One tier bump or a second \
                     route keeps headroom before it binds.",
                    dr.saturation * 100.0,
                    dr.flow,
                    dr.capacity
                ),
                saw: format!("route {} at {:.0}%", rid, dr.saturation * 100.0),
                cta: Some(CardCta::Trace {
                    selection: "route".into(),
                    id: rid.clone(),
                }),
            });
        }
    }

    // PowerSwing — circuit margin dips under 20% headroom
    for c in &derived.circuits {
        let (headroom, level) = crate::session::circuit_level(c.generation_mw, c.demand_mw);
        if level != "ok" {
            let crit = level == "crit";
            events.push(Event {
                key: format!("power:{}", c.name),
                rule: "power_swing",
                severity: if crit {
                    Severity::Conflict
                } else {
                    Severity::Trend
                },
                title: format!(
                    "{} margin is {}",
                    c.name,
                    if crit { "critical" } else { "thin" }
                ),
                body: format!(
                    "Demand {:.0} MW against {:.0} MW generation — {:.0}% headroom. \
                     A demand spike browns out the grid; plan generation before it does.",
                    c.demand_mw,
                    c.generation_mw,
                    headroom.max(0.0) * 100.0
                ),
                saw: format!("{}: {:.0}/{:.0} MW", c.name, c.demand_mw, c.generation_mw),
                cta: None,
            });
        }
    }

    // DriftDetected — an open SaveReimport proposal is unreviewed game drift
    for p in state.proposals.values() {
        if p.source == planner_core::proposals::ProposalSource::SaveReimport
            && matches!(
                p.status,
                planner_core::proposals::ProposalStatus::Draft
                    | planner_core::proposals::ProposalStatus::Reviewing
            )
        {
            events.push(Event {
                key: format!("drift:{}", p.id),
                rule: "drift_detected",
                severity: Severity::Tip,
                title: "The game drifted from the built layer".into(),
                body: format!(
                    "{} carries {} unreviewed change(s) from the last re-import. \
                     Review to sync — nothing applies until you accept.",
                    p.title,
                    p.items.len()
                ),
                saw: format!("{} · {} items", p.title, p.items.len()),
                cta: Some(CardCta::Review {
                    proposal: p.id.clone(),
                }),
            });
        }
    }

    events
}

/// The gate: new-event arming + per-rule debounce + mutes + pause. Owned by
/// the session; persistence hooks live there. Arming state survives restarts
/// via [`GateSnapshot`] so still-true conditions don't re-fire every launch.
#[derive(Default)]
pub struct AdvisorState {
    pub cards: Vec<AdvisorCard>,
    pub muted: BTreeSet<String>,
    pub paused: bool,
    /// Condition keys whose true-edge has been ACCOUNTED FOR — reported
    /// (card fired) or muted-away. Keys suppressed by debounce or pause are
    /// deliberately absent so they re-arm and report on a later gate pass.
    active_keys: BTreeSet<String>,
    /// rule → last fire epoch-seconds (debounce).
    last_fire: BTreeMap<String, u64>,
    /// Visible hourly model-call budget (unspent while offline).
    pub calls_this_hour: u32,
    hour_started: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorFeed {
    pub cards: Vec<AdvisorCard>,
    pub muted: Vec<String>,
    pub paused: bool,
    pub calls_this_hour: u32,
    pub call_budget: u32,
    /// "offline" | "ready" — no key means the heuristics speak for themselves.
    pub ai_status: String,
}

/// The gate's durable arming state: which condition true-edges have been
/// accounted for, plus each rule's last fire time. Persisted outside the
/// undo journal like cards/mutes — undoing a plan edit must not re-arm what
/// the advisor already reported. `calls_this_hour`/`hour_started` are
/// deliberately excluded: the budget is a display courtesy tied to a wall
/// clock, and it self-heals on the first gate after restart.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct GateSnapshot {
    active_keys: BTreeSet<String>,
    last_fire: BTreeMap<String, u64>,
}

impl AdvisorState {
    /// Run the gate over fresh events. Returns newly created cards (already
    /// appended to `self.cards`); the caller persists them.
    ///
    /// Each true-edge is reported at most once: a key enters `active_keys`
    /// only when its card fired or its rule is muted. Debounce and pause
    /// leave the key un-seen, so the report is delayed to a later gate pass
    /// — never cancelled, never duplicated.
    pub fn gate(
        &mut self,
        events: Vec<Event>,
        now_epoch_s: u64,
        now_rfc3339: &str,
    ) -> Vec<AdvisorCard> {
        // budget window roll
        if now_epoch_s.saturating_sub(self.hour_started) >= 3600 {
            self.hour_started = now_epoch_s;
            self.calls_this_hour = 0;
        }
        if self.paused {
            // Prune conditions that cleared while paused; arm nothing new —
            // edges suppressed here stay un-seen and report on the first
            // gate after unpause.
            let current: BTreeSet<String> = events.into_iter().map(|e| e.key).collect();
            self.active_keys.retain(|k| current.contains(k));
            return Vec::new();
        }
        let mut next_active = BTreeSet::new();
        let mut created = Vec::new();
        for e in events {
            if self.active_keys.contains(&e.key) {
                // Edge already accounted for — never re-report.
                next_active.insert(e.key);
                continue;
            }
            if self.muted.contains(e.rule) {
                // Mute = never report. Counting the edge as seen means
                // unmuting doesn't ambush with cards for conditions that
                // were true the whole time; a clear-and-reappear after
                // unmute is a fresh edge and fires normally.
                next_active.insert(e.key);
                continue;
            }
            let debounced = self
                .last_fire
                .get(e.rule)
                .map(|t| now_epoch_s.saturating_sub(*t) < DEBOUNCE_S)
                .unwrap_or(false);
            if debounced {
                // Suppressed AND un-seen: the key re-arms next gate, so the
                // report lands once the window passes.
                continue;
            }
            self.last_fire.insert(e.rule.to_string(), now_epoch_s);
            let card = AdvisorCard {
                id: planner_core::entities::new_id(),
                severity: e.severity,
                title: e.title,
                body: e.body,
                rule: e.rule.to_string(),
                saw: e.saw,
                at: now_rfc3339.to_string(),
                dismissed: false,
                cta: e.cta,
            };
            self.cards.push(card.clone());
            created.push(card);
            next_active.insert(e.key);
        }
        self.active_keys = next_active;
        created
    }

    /// Serialize the arming state for persistence (see [`GateSnapshot`]).
    pub fn gate_snapshot_json(&self) -> String {
        serde_json::to_string(&GateSnapshot {
            active_keys: self.active_keys.clone(),
            last_fire: self.last_fire.clone(),
        })
        .unwrap_or_default()
    }

    /// Hydrate the arming state from a persisted snapshot. Malformed input
    /// is ignored — the gate falls back to empty (pre-persistence) behavior.
    pub fn restore_gate_snapshot(&mut self, json: &str) {
        if let Ok(snap) = serde_json::from_str::<GateSnapshot>(json) {
            self.active_keys = snap.active_keys;
            self.last_fire = snap.last_fire;
        }
    }

    pub fn feed(&self, ai_ready: bool) -> AdvisorFeed {
        AdvisorFeed {
            cards: self
                .cards
                .iter()
                .filter(|c| !c.dismissed)
                .cloned()
                .collect(),
            muted: self.muted.iter().cloned().collect(),
            paused: self.paused,
            calls_this_hour: self.calls_this_hour,
            call_budget: HOURLY_CALL_BUDGET,
            ai_status: if ai_ready {
                "ready".into()
            } else {
                "offline".into()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(key: &str, rule: &'static str) -> Event {
        Event {
            key: key.into(),
            rule,
            severity: Severity::Tip,
            title: "t".into(),
            body: "b".into(),
            saw: format!("saw:{key}"),
            cta: None,
        }
    }

    #[test]
    fn gate_arms_only_new_conditions() {
        let mut st = AdvisorState::default();
        let made = st.gate(vec![ev("a", "r1")], 1000, "t0");
        assert_eq!(made.len(), 1);
        // same condition persists → no re-fire, even past the debounce window
        let made = st.gate(vec![ev("a", "r1")], 2000, "t1");
        assert!(made.is_empty(), "persisting condition must not re-arm");
        // condition clears, then reappears → fires again
        st.gate(vec![], 3000, "t2");
        let made = st.gate(vec![ev("a", "r1")], 4000, "t3");
        assert_eq!(made.len(), 1);
    }

    #[test]
    fn gate_debounces_per_rule_and_respects_mutes_and_pause() {
        let mut st = AdvisorState::default();
        // two different conditions of the same rule inside 30s → one card
        let made = st.gate(vec![ev("a", "r1"), ev("b", "r1")], 1000, "t");
        assert_eq!(made.len(), 1, "debounce folds same-rule bursts");
        // muted rule stays silent even for new conditions
        st.muted.insert("r1".into());
        st.gate(vec![], 2000, "t");
        let made = st.gate(vec![ev("c", "r1")], 3000, "t");
        assert!(made.is_empty(), "muted rule never fires");
        // pause silences everything
        st.muted.clear();
        st.paused = true;
        st.gate(vec![], 4000, "t");
        let made = st.gate(vec![ev("d", "r2")], 5000, "t");
        assert!(made.is_empty(), "paused advisor is silent");
    }

    #[test]
    fn debounced_condition_fires_after_window() {
        let mut st = AdvisorState::default();
        // two same-rule conditions in one call → only the first fires
        let made = st.gate(vec![ev("a", "r1"), ev("b", "r1")], 1000, "t");
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].saw, "saw:a");
        // still inside the 30 s window → b stays suppressed but un-seen
        let made = st.gate(vec![ev("a", "r1"), ev("b", "r1")], 1010, "t");
        assert!(made.is_empty(), "debounce window still holds");
        // window passed → the delayed report lands, exactly once, and it's b
        let made = st.gate(vec![ev("a", "r1"), ev("b", "r1")], 1031, "t");
        assert_eq!(made.len(), 1, "debounced edge fires after the window");
        assert_eq!(made[0].saw, "saw:b", "the fired condition never re-fires");
        // both now seen → silence
        let made = st.gate(vec![ev("a", "r1"), ev("b", "r1")], 2000, "t");
        assert!(made.is_empty(), "both edges reported exactly once");
    }

    #[test]
    fn condition_arising_while_paused_fires_on_unpause() {
        let mut st = AdvisorState::default();
        // a fires pre-pause and stays seen through the pause
        let made = st.gate(vec![ev("a", "r1")], 1000, "t");
        assert_eq!(made.len(), 1);
        st.paused = true;
        let made = st.gate(vec![ev("a", "r1"), ev("b", "r2")], 2000, "t");
        assert!(made.is_empty(), "paused advisor is silent");
        st.paused = false;
        let made = st.gate(vec![ev("a", "r1"), ev("b", "r2")], 3000, "t");
        assert_eq!(made.len(), 1, "edge suppressed by pause fires on unpause");
        assert_eq!(made[0].saw, "saw:b");
        assert_eq!(made[0].rule, "r2");
    }

    #[test]
    fn muted_condition_does_not_fire_on_unmute() {
        let mut st = AdvisorState::default();
        st.muted.insert("r1".into());
        let made = st.gate(vec![ev("a", "r1")], 1000, "t");
        assert!(made.is_empty(), "muted rule never fires");
        st.muted.clear();
        let made = st.gate(vec![ev("a", "r1")], 2000, "t");
        assert!(
            made.is_empty(),
            "seen-while-muted edge does not ambush on unmute"
        );
        // clear, then reappear → a fresh edge fires normally
        st.gate(vec![], 3000, "t");
        let made = st.gate(vec![ev("a", "r1")], 4000, "t");
        assert_eq!(made.len(), 1, "clear-and-reappear after unmute fires");
    }

    #[test]
    fn gate_snapshot_restart_roundtrip() {
        let mut st = AdvisorState::default();
        let made = st.gate(vec![ev("a", "r1")], 1000, "t");
        assert_eq!(made.len(), 1);
        let json = st.gate_snapshot_json();

        // "restart": fresh state hydrated from the snapshot
        let mut st2 = AdvisorState::default();
        st2.restore_gate_snapshot(&json);
        let made = st2.gate(vec![ev("a", "r1")], 5000, "t");
        assert!(
            made.is_empty(),
            "still-true condition is silent after restart"
        );
        let made = st2.gate(vec![ev("a", "r1"), ev("b", "r2")], 5000, "t");
        assert_eq!(made.len(), 1, "genuinely new condition fires after restart");
        assert_eq!(made[0].rule, "r2");
    }
}
