//! Opportunity engine (PR 9, offline core) — "what should I work on next" as a
//! DERIVED projection, same species as buildqueue.rs: pure over
//! `(state, gamedata, derived, world, unlocked)`, no stored entities, no model
//! call. Eight candidate families each carry solver-derived numbers (evidence)
//! and an ACTION that lands on an EXISTING pipe — wizard prefill, map
//! selection, or an audit tab — so acting on a suggestion is always either
//! pure navigation or the already-undoable wizard/review flow.
//!
//! HONEST SILENCE is the contract: a family whose input data is absent emits
//! nothing — never a guessed number. A healthy finished base returns an empty
//! list, and that emptiness is the feature (the advisor's "silence is a
//! feature" doctrine, applied to ambition instead of alarm).
//!
//! Ranking is a documented class-order tuple (broken → milestone → savings →
//! growth), magnitude-descending within a class (distance ASCENDING for
//! untapped nodes), capped at 12. NO cross-unit arithmetic — MW overdraw and
//! machines saved are never summed into one score (house precedent: altopt's
//! lexicographic ordering).

use std::collections::{BTreeMap, BTreeSet};

use gamedata::docs::{extraction_rate, GameData};
use gamedata::worldnodes::WorldSnapshot;
use planner_core::entities::*;
use planner_core::state::{NextPreferences, PlanState};
use serde::Serialize;

use crate::session::{circuit_level, Derived};

/// "Running at capacity" within solver float noise — mirrors `FULL` in
/// advisor.rs and `routeBottleneck` in renderer/src/lib/format.ts (the same
/// efficiency-grammar rule, third consumer).
const FULL: f64 = 0.999;

/// Solver float noise floor for gap/overdraw gating — a candidate must clear
/// this to exist at all (never surface a 1e-12 rounding artifact as advice).
const EPS: f64 = 1e-6;

/// How near an unclaimed pure node must sit to an existing factory to count
/// as a growth opportunity, in world meters (`MapPos` is the save coordinate
/// frame in meters; the bundled snapshot spans ~7.5 km × 7.5 km). 2 500 m is
/// "same neighborhood": ~a third of the map's radius, comfortably past the
/// ~800 m belt-vs-rail boundary in `transport::suggest_kind` but short of a
/// cross-map expedition. Distances use the same 2D `hypot` as node-drift
/// detection; cave nodes measure from their ENTRANCE (routes must go via it).
const UNTAPPED_RADIUS_M: f64 = 2500.0;

/// How many nearest untapped nodes to surface (growth ideas, not a catalog).
const UNTAPPED_LIMIT: usize = 3;

/// Ranked list cap — a shortlist, not a report.
const CAP: usize = 12;

/// Demoted ranking CLASS for a `power_deficit` under the `ignore_power`
/// preference (PR 3): the overdraw FACT never leaves the list, but its class
/// sinks below the actionable repair families the player chose to act on
/// (`deficit_repair` class 1, `route_bottleneck_fix` class 2) — a REAL
/// cross-class demotion, not a magnitude hack (power_deficit is class 0's sole
/// member, so nudging magnitude alone left it at #1). Reuses `power_margin`'s
/// band; that advisory card is itself hidden under `ignore_power`, so the two
/// never collide. The status-bar power chip and STARVING section are untouched.
const DEMOTED_POWER_CLASS: u8 = 3;

/// Honest note appended to a demoted `power_deficit` (PR 3): the preference
/// quieted the SUGGESTIONS, it cannot un-overdraw a grid.
const IGNORE_POWER_NOTE: &str = " — power ignored by preference — this grid is still overdrawn";

/// Candidate family, in ranking-class order (the discriminant IS the class).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityKind {
    /// Class 0 — a grid is overdrawn right now (broken).
    PowerDeficit,
    /// Class 1 — a target somewhere is starved (broken).
    DeficitRepair,
    /// Class 2 — a full route provably caps demand (broken, causal).
    RouteBottleneckFix,
    /// Class 3 — a grid is one spike from a brownout (trending broken).
    PowerMargin,
    /// Class 4 — the next unpurchased HUB milestone needs an item the empire
    /// under-produces (broken → milestone → savings): see [`milestone_gap`].
    MilestoneGap,
    /// Class 5 — an unlocked alternate saves machines empire-wide (savings).
    AltAdopt,
    /// Class 6 — a claimed node runs under 100% clock (untapped throughput).
    UnderExtracted,
    /// Class 7 — an unclaimed pure node near an existing factory (growth).
    UntappedNode,
}

/// A card's call-to-action. Every variant maps onto an EXISTING pipe: the
/// wizard prefill (already undoable end-to-end), a map selection (pure
/// navigation), or an audit-drawer tab. The engine never edits the plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum OpportunityAction {
    /// Open the wizard pre-filled (the FIX WITH SOLVER pattern).
    WizardGoal { item: String, rate: f64 },
    /// Select a route on the map (drawer carries the tier control).
    SelectRoute { id: Id },
    /// Select a resource node on the map.
    SelectNode { id: String },
    /// Select a factory on the map (claims live in its drawer).
    SelectFactory { id: Id },
    /// Open an audit-drawer tab (`"power" | "optimizer" | …`).
    OpenAudit { tab: String },
}

/// One ranked next move. `id` is DETERMINISTIC (kind + subject ids, never
/// random) so re-fetches keep stable React keys and tests can address rows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opportunity {
    pub id: String,
    pub kind: OpportunityKind,
    pub title: String,
    /// Provenance: exactly what the engine saw, numbers formatted Rust-side
    /// (advisor `saw` discipline — the renderer never re-derives them).
    pub evidence: String,
    /// Item class for the renderer's ItemIcon chip, when one is on stage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub action: OpportunityAction,
}

/// Internal candidate: the ranking tuple stays out of the payload.
struct Candidate {
    /// Ranking class (== kind discriminant order, kept explicit for the sort).
    class: u8,
    /// Within-class urgency, LARGER = FIRST. Only ever compared against the
    /// same class's magnitudes, so units never mix (MW overdraw vs missing/min
    /// vs machines saved; untapped nodes store the NEGATED distance so nearer
    /// ranks first) — mirrors altopt's no-cross-unit-arithmetic ordering.
    magnitude: f64,
    opp: Opportunity,
}

/// Item display name, falling back to the trimmed class (buildqueue's rule).
fn item_name(gd: &GameData, item: &str) -> String {
    gd.items
        .get(item)
        .map(|i| i.display_name.clone())
        .unwrap_or_else(|| {
            item.trim_start_matches("Desc_")
                .trim_end_matches("_C")
                .to_string()
        })
}

/// `A → B` route label from its port (or factory) endpoints.
fn route_endpoints(state: &PlanState, r: &Route) -> String {
    let name = |pid: &Id| {
        state
            .ports
            .get(pid)
            .and_then(|p| state.factories.get(&p.factory))
            .or_else(|| state.factories.get(pid))
            .map(|f| f.name.clone())
    };
    match (name(&r.endpoints.0), name(&r.endpoints.1)) {
        (Some(a), Some(b)) => format!("{a} → {b}"),
        _ => "route".into(),
    }
}

/// MW overdraw for display: one decimal below 0.5 MW — an overdrawn grid must
/// never read "overdrawn by 0 MW" — whole MW otherwise (round, not ceil: the
/// evidence pair carries the exact figures).
fn fmt_overdraw_mw(mw: f64) -> String {
    if mw < 0.5 {
        format!("{mw:.1}")
    } else {
        format!("{mw:.0}")
    }
}

/// Two-gap decomposition of one DeficitRow (H1). The upstream WITNESS is the
/// src-port achieved out-rate of the row's route (resolved `state.ports` →
/// `p.factory` → `derived.factories[..].ports[src]`): upstream factories solve
/// BLIND to downstream belts, so this rate is honest proof of how much of the
/// need production already covers. Split the miss into:
///
/// - `production_gap = max(0, needed − produced)` — no machines make it;
/// - `transport_gap  = max(0, min(needed, produced) − supplied)` — made
///   upstream but capped by the route.
///
/// When the route or its source port is unresolvable the whole miss counts as
/// production gap: the deficit card may over-ask, but it never hides a starve.
fn row_gaps(state: &PlanState, derived: &Derived, d: &crate::session::DeficitRow) -> (f64, f64) {
    let missing = (d.needed - d.supplied).max(0.0);
    let produced = d
        .route
        .as_ref()
        .and_then(|rid| state.routes.get(rid))
        .and_then(|r| state.ports.get(&r.endpoints.0))
        .and_then(|p| {
            derived
                .factories
                .get(&p.factory)
                .and_then(|df| df.ports.get(&p.id))
        })
        .copied();
    match produced {
        Some(produced) => (
            (d.needed - produced).max(0.0),
            (d.needed.min(produced) - d.supplied).max(0.0),
        ),
        None => (missing, 0.0),
    }
}

/// Empire-wide PRODUCTION gap per item — the post-decomposition demand signal
/// [`under_extracted`] gates on (a transport-capped miss is not a production
/// demand; upgrading the belt, not the miner clock, fixes it).
fn production_gaps(state: &PlanState, derived: &Derived) -> BTreeMap<String, f64> {
    let mut by_item: BTreeMap<String, f64> = BTreeMap::new();
    for d in &derived.deficits {
        *by_item.entry(d.item.clone()).or_insert(0.0) += row_gaps(state, derived, d).0;
    }
    by_item
}

/// Class 0 — `power_deficit`: any circuit with headroom < 0. Evidence is the
/// derived generation/demand MW pair; magnitude is the overdraw in MW.
///
/// EMPIRE FALLBACK (save-imported bases draw no power routes → zero circuits →
/// the per-grid loop is structurally silent): when NO circuits exist but the
/// empire totals prove an overdraw, emit one plan-wide class-0 card. This is
/// pigeonhole-honest — `total_power_mw` is the same Σ over factory draws the
/// per-grid demand sums use, and `total_generation_mw` the same solved
/// POWER_ITEM outputs `gen_of` sums per grid (no self-draw pollution), so
/// empire demand > empire generation PROVES at least one physical grid is
/// overdrawn. The converse does NOT hold for margins: a thin empire margin
/// proves nothing per-grid (one grid can be overdrawn while another idles), so
/// there is deliberately NO empire-level `power_margin` fallback. The
/// `total_generation_mw > 0` gate keeps mid-planning bases (machines drawn, no
/// generators yet) un-nagged.
fn power_deficit(derived: &Derived, prefs: &NextPreferences, out: &mut Vec<Candidate>) {
    // PR 3: `ignore_power` may DEMOTE + NOTE this card (a real overdraw is a
    // FACT, never hidden), but it can never suppress it.
    let ignored = prefs.ignore_power;
    // A demoted overdraw keeps its true magnitude (the overdraw figure is a
    // FACT) — only its ranking CLASS changes, sinking it below the repairs.
    let class = if ignored { DEMOTED_POWER_CLASS } else { 0 };
    let note = |mut evidence: String| {
        if ignored {
            evidence.push_str(IGNORE_POWER_NOTE);
        }
        evidence
    };
    for c in &derived.circuits {
        let (headroom, _) = circuit_level(c.generation_mw, c.demand_mw);
        if headroom >= 0.0 {
            continue;
        }
        let overdraw = c.demand_mw - c.generation_mw;
        out.push(Candidate {
            class,
            magnitude: overdraw,
            opp: Opportunity {
                id: format!("power_deficit:{}", c.name),
                kind: OpportunityKind::PowerDeficit,
                title: format!(
                    "{} is overdrawn by {} MW",
                    c.name,
                    fmt_overdraw_mw(overdraw)
                ),
                evidence: note(format!(
                    "{:.0} MW demand against {:.0} MW generated",
                    c.demand_mw, c.generation_mw
                )),
                item: None,
                action: OpportunityAction::OpenAudit {
                    tab: "power".into(),
                },
            },
        });
    }
    if derived.circuits.is_empty()
        && derived.total_generation_mw > 0.0
        && derived.total_power_mw > derived.total_generation_mw + EPS
    {
        let overdraw = derived.total_power_mw - derived.total_generation_mw;
        out.push(Candidate {
            class,
            magnitude: overdraw,
            opp: Opportunity {
                id: "power_deficit:empire".into(),
                kind: OpportunityKind::PowerDeficit,
                title: format!(
                    "Plan-wide power demand exceeds generation by {} MW",
                    fmt_overdraw_mw(overdraw)
                ),
                evidence: note(format!(
                    "{:.0} MW demand vs {:.0} MW generated — no power routes drawn, per-grid balance unknown",
                    derived.total_power_mw, derived.total_generation_mw
                )),
                item: None,
                action: OpportunityAction::OpenAudit {
                    tab: "power".into(),
                },
            },
        });
    }
}

/// Class 1 — `deficit_repair`: DeficitRows grouped by item (one empire-wide
/// card per item, not one per starved port), with each row H1-decomposed via
/// [`row_gaps`]. The card exists — and its number, magnitude, and wizard
/// prefill are sized — ONLY from the summed PRODUCTION gap: planning new
/// machines for a transport-capped miss would build redundant production (the
/// route card owns that share). A nonzero transport share is still named in
/// the evidence; when the transport share is zero but a contributing route
/// runs FULL (upstream produces exactly the belt cap — the starved-at-cap
/// case), the evidence names the full route instead, because fixing production
/// alone will immediately hit that cap.
fn deficit_repair(state: &PlanState, gd: &GameData, derived: &Derived, out: &mut Vec<Candidate>) {
    #[derive(Default)]
    struct Agg {
        needed: f64,
        supplied: f64,
        rows: usize,
        production_gap: f64,
        transport_gap: f64,
        /// "the Mk.N route" label of a FULL contributing route, if any.
        full_route: Option<String>,
    }
    let mut by_item: BTreeMap<String, Agg> = BTreeMap::new();
    for d in &derived.deficits {
        let (pg, tg) = row_gaps(state, derived, d);
        let e = by_item.entry(d.item.clone()).or_default();
        e.needed += d.needed;
        e.supplied += d.supplied;
        e.rows += 1;
        e.production_gap += pg;
        e.transport_gap += tg;
        if e.full_route.is_none() {
            if let Some(rid) = &d.route {
                if derived
                    .routes
                    .get(rid)
                    .is_some_and(|dr| dr.saturation >= FULL)
                {
                    e.full_route = Some(match state.routes.get(rid).map(|r| &r.kind) {
                        Some(RouteKind::Belt { tier }) => format!("the Mk.{tier} route"),
                        _ => "the route".into(),
                    });
                }
            }
        }
    }
    for (item, a) in by_item {
        if a.production_gap <= EPS {
            continue;
        }
        let mut evidence = format!(
            "need {:.1}/min, supplied {:.1}/min across {} port(s)",
            a.needed, a.supplied, a.rows
        );
        if a.transport_gap > EPS {
            evidence.push_str(&format!(
                "; {:.1}/min more capped by full route(s)",
                a.transport_gap
            ));
        } else if let Some(route) = &a.full_route {
            evidence.push_str(&format!(
                "; {route} is already full — upgrading it is also required once production rises"
            ));
        }
        out.push(Candidate {
            class: 1,
            magnitude: a.production_gap,
            opp: Opportunity {
                id: format!("deficit_repair:{item}"),
                kind: OpportunityKind::DeficitRepair,
                title: format!(
                    "{} is short {:.1}/min empire-wide",
                    item_name(gd, &item),
                    a.production_gap
                ),
                evidence,
                item: Some(item.clone()),
                action: OpportunityAction::WizardGoal {
                    item,
                    rate: a.production_gap.ceil().max(1.0),
                },
            },
        });
    }
}

/// Class 2 — `route_bottleneck_fix`: a route at FULL capacity with a deficit
/// registered THROUGH it whose H1 TRANSPORT gap is nonzero — a full route
/// whose consumers are satisfied is OPTIMAL, and a full route starving only
/// because upstream makes exactly the cap recovers nothing by itself (the
/// deficit card carries that case and mentions the route). The card is gated
/// AND sized on the recoverable rate; the title names the concrete fix: the
/// SMALLEST sufficient belt tier for `flow + recoverable` (never a blind +1),
/// "+1 consist" / "+1 truck" for rail/truck (the drawer's own steppers), or a
/// second drone route (DroneSpec has no count stepper).
fn route_bottleneck_fix(
    state: &PlanState,
    gd: &GameData,
    derived: &Derived,
    prefs: &NextPreferences,
    out: &mut Vec<Candidate>,
) {
    // Per-item PRODUCTION gaps — only needed to tell a `no_trains`-suppressed
    // rail route that is a TRANSPORT-ONLY starve (re-emit under a non-train
    // framing) from a MIXED one (a `deficit_repair` card already alludes to the
    // route in its evidence — leave the "+1 consist" suggestion suppressed).
    let prod_gaps = prefs.no_trains.then(|| production_gaps(state, derived));
    for (rid, dr) in &derived.routes {
        if dr.saturation < FULL {
            continue;
        }
        let recoverable: f64 = derived
            .deficits
            .iter()
            .filter(|d| d.route.as_ref() == Some(rid))
            .map(|d| row_gaps(state, derived, d).1)
            .sum();
        if recoverable <= EPS {
            continue;
        }
        let Some(route) = state.routes.get(rid) else {
            continue;
        };
        // PR 3: `no_trains` suppresses RAIL route-fix suggestions (the fix names
        // a "+1 consist"); belt/pipe/truck/drone route cards are unaffected.
        // EXCEPTION (PR #11 M4): a rail route that carries a transport-only
        // starve is the ONLY advice about a factory the player already starved
        // with a train they built — dropping it hides a real problem. When no
        // `deficit_repair` covers the item's production gap, RE-EMIT the card
        // under a non-train framing (name the route as the cap, suggest a
        // belt/truck alternative — never "+1 consist").
        let is_rail = matches!(route.kind, RouteKind::Rail { .. });
        let reframe_no_train = if prefs.no_trains && is_rail {
            let production_capped = dr
                .item
                .as_ref()
                .and_then(|i| prod_gaps.as_ref().map(|g| g.get(i).copied().unwrap_or(0.0)))
                .unwrap_or(0.0)
                > EPS;
            if production_capped {
                continue; // mixed gap — deficit_repair already alludes to it
            }
            true
        } else {
            false
        };
        let endpoints = route_endpoints(state, route);
        let fix = if reframe_no_train {
            // The fact is an EXISTING overloaded rail route, not a suggestion to
            // add a consist — point at a belt/truck alternative instead.
            "carry it by belt or truck instead".to_string()
        } else {
            match &route.kind {
                RouteKind::Belt { tier } => {
                    // Smallest tier that actually clears the decomposed need
                    // (reuses planner_core's belt table — never a copy).
                    match ((tier + 1)..=6u8)
                        .find(|t| belt_capacity(*t) + EPS >= dr.flow + recoverable)
                    {
                        Some(t) => format!("bump it to Mk.{t}"),
                        None => "beyond Mk.6 — add a parallel belt".into(),
                    }
                }
                RouteKind::Rail { .. } => "+1 consist".into(),
                RouteKind::Truck { .. } => "+1 truck".into(),
                RouteKind::Drone { .. } => "add a second drone route".into(),
                _ => "add a second route".into(),
            }
        };
        out.push(Candidate {
            class: 2,
            magnitude: recoverable,
            opp: Opportunity {
                id: format!("route_bottleneck_fix:{rid}"),
                kind: OpportunityKind::RouteBottleneckFix,
                title: format!("{endpoints} caps demand — {fix}"),
                evidence: format!(
                    "{:.1}/{:.1} per min at {:.0}% with {:.1}/min recoverable through it",
                    dr.flow,
                    dr.capacity,
                    dr.saturation * 100.0,
                    recoverable
                ),
                item: dr.item.clone().filter(|i| gd.items.contains_key(i)),
                action: OpportunityAction::SelectRoute { id: rid.clone() },
            },
        });
    }
}

/// Class 3 — `power_margin`: 0 ≤ headroom < 0.20 (the `circuit_level` warn
/// band, reused so the threshold lives in ONE place). Magnitude is the
/// NEGATED headroom: thinner margin ranks first within the class. The
/// percentage FLOORS in title and evidence both — 19.5% headroom must read
/// "19%", never round up out of its own alarm band.
fn power_margin(derived: &Derived, prefs: &NextPreferences, out: &mut Vec<Candidate>) {
    // PR 3: `ignore_power` HIDES this purely-advisory card entirely (headroom
    // is a suggestion, not a fact — nothing is broken yet).
    if prefs.ignore_power {
        return;
    }
    for c in &derived.circuits {
        let (headroom, level) = circuit_level(c.generation_mw, c.demand_mw);
        if headroom < 0.0 || level == "ok" {
            continue;
        }
        let pct = (headroom * 100.0).floor();
        out.push(Candidate {
            class: 3,
            magnitude: -headroom,
            opp: Opportunity {
                id: format!("power_margin:{}", c.name),
                kind: OpportunityKind::PowerMargin,
                title: format!("{} has only {pct:.0}% headroom", c.name),
                evidence: format!(
                    "{pct:.0}% headroom ({:.0} of {:.0} MW drawn)",
                    c.demand_mw, c.generation_mw
                ),
                item: None,
                action: OpportunityAction::OpenAudit {
                    tab: "power".into(),
                },
            },
        });
    }
}

/// Empire-wide gross OUTPUT rate of `item` — summed over every group's derived
/// `out_rates` across all factories. The same accessor the power rollups
/// (`gen_of` / `total_generation_mw`) use for POWER_ITEM; pure over `derived`,
/// no re-solve, and gross production is the honest "how much the empire makes"
/// figure (matching buildqueue's "milestone built = gross production").
///
/// B3 (PR 4 review) — GROSS, disclosed, not NET: a per-item net surplus
/// (`out_rates − in_rates` across groups) is CHEAPLY derivable, but it is not
/// unambiguously "correct" as a divertable figure — a milestone part fully
/// consumed by an unrelated line nets to 0 and would render a misleading
/// "makes 0/min" for an empire visibly producing it, trading gross's
/// overstatement for an equal understatement. Since neither denominator is
/// unambiguously right (and HUB stockpiles are untracked either way), we keep
/// the informative gross figure and DISCLOSE its framing in the evidence.
fn empire_output(derived: &Derived, item: &str) -> f64 {
    derived
        .factories
        .values()
        .flat_map(|f| f.groups.values())
        .filter_map(|g| g.out_rates.get(item).copied())
        .sum()
}

/// Class 4 — `milestone_gap`: the single next HUB milestone the empire can't
/// yet build from an hour of its own production. The next milestone is chosen
/// FRONTIER-anchored (B2, PR 4 review): Satisfactory lets a player SKIP
/// milestones freely, so "lowest unpurchased across the whole tree" nags a
/// veteran to back-fill an early tier they deliberately passed. Instead the
/// FRONTIER is the highest `tier` among PURCHASED milestones, and the next
/// honest step is the lowest UNPURCHASED milestone AT that frontier tier (by
/// class name). If the frontier tier has no unpurchased milestone left, the
/// next tier is Space-Elevator-phase-gated — invisible to us — so we stay
/// SILENT rather than point at a milestone the player can't yet buy. When
/// NOTHING is purchased (a fresh import), fall back to the lowest-overall
/// milestone by `(tier, class_name)` — a genuine tier-1 first step.
///
/// For each costed item, `gap = max(0, qty − 60·production)` — units still
/// short of a one-hour build at the empire's current gross OUTPUT rate; the
/// largest-gap item is surfaced with a WizardGoal producing the remainder in
/// ~60 min (`(gap / 60).ceil()`, a clean ≥1 number, the deficit_repair
/// grammar), and a multi-item bill flags the rest ("· +N more in this
/// milestone", B4) so the single surfaced item never reads as the whole cost.
///
/// HONEST FRAMING: HUB inventory is untracked (the save carries no
/// lifetime-crafted counter) AND `produced` is GROSS production (never nets
/// downstream consumption), so the number is neither an inventory claim nor a
/// divertable-surplus claim — the evidence DISCLOSES this ("based on current
/// production; stockpiles not counted", B3). We help PRODUCE the parts as a
/// rate, and stay SILENT when the empire already out-produces every cost
/// within an hour (`gap ≤ ε`). Silent, too, when no milestones are parsed (the
/// trimmed fixture) or the frontier tier is cleared/all purchased. Exactly ONE
/// card — the next milestone — never a per-milestone nag.
fn milestone_gap(
    gd: &GameData,
    purchased: &BTreeSet<String>,
    derived: &Derived,
    out: &mut Vec<Candidate>,
) {
    // Frontier = highest tier among PURCHASED milestones (None → nothing
    // purchased yet, a fresh import).
    let frontier: Option<u32> = gd
        .milestones
        .iter()
        .filter(|(id, _)| purchased.contains(id.as_str()))
        .map(|(_, m)| m.tier)
        .max();
    // Candidates: unpurchased, non-empty-cost (B1 belt-and-suspenders — an
    // empty-cost milestone that slipped the parse-time drop must never be
    // selected and silence the family), AT the frontier tier when one exists.
    // No frontier → all tiers eligible (lowest overall wins below). A cleared
    // frontier (nothing left at that tier) yields None → honest silence.
    let Some((class, m)) = gd
        .milestones
        .iter()
        .filter(|(id, m)| {
            !purchased.contains(id.as_str())
                && !m.cost.is_empty()
                && frontier.is_none_or(|t| m.tier == t)
        })
        .min_by(|(a_id, a), (b_id, b)| a.tier.cmp(&b.tier).then_with(|| a_id.cmp(b_id)))
    else {
        return;
    };
    // Largest-gap cost item: gap = max(0, qty − 60·production). Deterministic
    // tie-break by item class so the pick is stable across re-fetches.
    let mut best: Option<(f64, &String, f64, f64)> = None; // (gap, item, qty, produced)
    for (item, qty) in &m.cost {
        // A production rate is never negative; clamp float noise. The trailing
        // `+ 0.0` normalizes a signed -0.0 (an empty/near-zero solve — and LLVM
        // fmax can pass -0.0 through `.max`) to +0.0, so the evidence never
        // reads "makes -0/min".
        let produced = empire_output(derived, item).max(0.0) + 0.0;
        let gap = (qty - produced * 60.0).max(0.0);
        let replace = match &best {
            None => true,
            Some((bg, bi, _, _)) => gap > *bg || (gap == *bg && item < *bi),
        };
        if replace {
            best = Some((gap, item, *qty, produced));
        }
    }
    let Some((gap, item, qty, produced)) = best else {
        return; // no resolvable cost entries — silence
    };
    if gap <= EPS {
        return; // the empire already out-produces every cost within an hour
    }
    // B4: a milestone costs >1 item — flag the rest so the surfaced (largest-
    // gap) item never reads as the whole bill (mitigates the largest-RAW-unit
    // pick landing on the numerous-but-trivial item over the real wall).
    let more = m.cost.len() - 1;
    let bill = if more > 0 {
        format!(" · +{more} more in this milestone")
    } else {
        String::new()
    };
    out.push(Candidate {
        class: 4,
        magnitude: gap,
        opp: Opportunity {
            id: format!("milestone_gap:{class}"),
            kind: OpportunityKind::MilestoneGap,
            title: format!("Advance to {} (Tier {})", m.display_name, m.tier),
            // B3: `produced` is GROSS output — disclose the framing so the
            // number isn't read as an inventory or divertable-surplus claim.
            evidence: format!(
                "needs {qty} {}; empire makes {produced:.0}/min — {gap:.0} short of a 1-hour build{bill} · based on current production; stockpiles not counted",
                item_name(gd, item)
            ),
            item: Some(item.clone()),
            action: OpportunityAction::WizardGoal {
                item: item.clone(),
                rate: (gap / 60.0).ceil(),
            },
        },
    });
}

/// Class 5 — `alt_adopt`: the TOP alternate-recipe opportunity by machines
/// saved. The computation is REUSED from altopt (`empire_optimize` already
/// ranks lexicographically and only surfaces net wins whose savings equal
/// adoptable savings by construction) — this family never re-derives it. The
/// card shows the WHOLE trade altopt computed: machines saved, the power
/// verb ("saves"/"costs" — never an ambiguous sign), the retool estimate, and
/// any genuinely NEW input chain the alternate would demand. "New" = a
/// positive input delta on an item no group in the plan produces and no
/// boundary In port imports (the deltas alone can't say — an alt's own
/// ingredients always read positive there).
fn alt_adopt(
    state: &PlanState,
    gd: &GameData,
    unlocked: &BTreeSet<String>,
    out: &mut Vec<Candidate>,
) {
    let Some(top) = crate::altopt::empire_optimize(state, gd, unlocked)
        .into_iter()
        .next()
    else {
        return; // nothing unlocked / no net win — honest silence
    };
    // Display-only prefix strip: the family chip already says ALT, so
    // "Alt Alternate: X" must not happen here. `recipe_name` itself stays
    // data-fidelity for the drawer/API.
    let recipe_name = top
        .recipe_name
        .strip_prefix("Alternate: ")
        .unwrap_or(&top.recipe_name);
    let mut parts: Vec<String> = vec![format!("−{} machines", top.machines_saved)];
    if top.power_saved_mw >= 0.0 {
        parts.push(format!("saves {:.0} MW", top.power_saved_mw));
    } else {
        parts.push(format!("costs {:.0} MW", -top.power_saved_mw));
    }
    if top.retool_est_hours > 0.0 {
        parts.push(format!("~{:.1} h retool", top.retool_est_hours));
    }
    // Items the empire can already source: every group's recipe products plus
    // every boundary In-port item.
    let handled: BTreeSet<&str> = state
        .groups
        .values()
        .filter_map(|g| gd.recipes.get(&g.recipe))
        .flat_map(|r| r.products.iter().map(|(i, _)| i.as_str()))
        .chain(
            state
                .ports
                .values()
                .filter(|p| p.direction == PortDirection::In)
                .map(|p| p.item.as_str()),
        )
        .collect();
    let mut new_chains = top
        .input_deltas
        .iter()
        .filter(|(i, v)| *v > EPS && !handled.contains(i.as_str()));
    if let Some((i, v)) = new_chains.next() {
        let more = if new_chains.next().is_some() {
            "…"
        } else {
            ""
        };
        parts.push(format!(
            "needs new {} chain ({v:.1}/min){more}",
            item_name(gd, i)
        ));
    }
    parts.push(format!("on {}", top.product_name));
    out.push(Candidate {
        class: 5,
        magnitude: top.machines_saved as f64,
        opp: Opportunity {
            id: format!("alt_adopt:{}", top.recipe),
            kind: OpportunityKind::AltAdopt,
            title: format!(
                "Alt {recipe_name} saves {} machines empire-wide",
                top.machines_saved
            ),
            evidence: parts.join(" · "),
            item: Some(top.product),
            action: OpportunityAction::OpenAudit {
                tab: "optimizer".into(),
            },
        },
    });
}

/// Does the solver report `c`'s factory genuinely BOUND by this claim's own
/// input ceiling on `item`? True only for an `InputCeiling` binding (from the
/// factory's `target_ceiling` or a shortfall) whose port belongs to the
/// claim's factory, carries the item, AND whose reported ceiling equals the
/// port's STORED `rate_ceiling` — the empire pass injects route supply as an
/// effective ceiling, so a binding that reports the route's figure is a
/// transport limit, not the claim's. A bare `target_ceiling.is_some()` is
/// NEVER the test (every single-output factory reports one); for the
/// target-ceiling channel the factory must also be running AT the ceiling —
/// an output achieving `max_rate` — since the ceiling is reported even when
/// the target sits far below it.
fn claim_ceiling_binds(state: &PlanState, derived: &Derived, c: &NodeClaim, item: &str) -> bool {
    let Some(df) = derived.factories.get(&c.factory) else {
        return false;
    };
    let matches = |binding: &solver::model::Constraint| -> bool {
        let solver::model::Constraint::InputCeiling {
            port,
            item: b_item,
            ceiling,
        } = binding
        else {
            return false;
        };
        b_item == item
            && state.ports.get(port).is_some_and(|p| {
                p.factory == c.factory
                    && p.item == *item
                    && p.rate_ceiling
                        .is_some_and(|rc| (rc - ceiling).abs() <= 1e-6)
            })
    };
    let at_ceiling = |max_rate: f64| df.ports.values().any(|&r| r >= max_rate - 1e-6);
    df.target_ceiling
        .as_ref()
        .is_some_and(|tc| matches(&tc.binding) && at_ceiling(tc.max_rate))
        || df
            .shortfalls
            .values()
            .any(|s| s.binding.as_ref().is_some_and(matches))
}

/// Class 6 — `under_extracted`: a claimed node under 100% clock whose lost
/// extraction is actually DEMANDED — a positive post-decomposition production
/// gap on the node's item empire-wide, or the owning factory genuinely bound
/// by the claim's own ceiling ([`claim_ceiling_binds`]). Deliberate ratio-
/// matching on an item nobody is short of stays silent (a healthy base is
/// quiet); there is NO per-resource exemption list — the demand gate IS the
/// honesty mechanism (a 94%-clock water pump is correct advice exactly when
/// water is short). Save-only claims (no catalog node → no item, no purity)
/// are honest silence. Magnitude is the USABLE gain — the raise-to-100% rate
/// delta capped by the item's production gap when one exists — and AT MOST
/// ONE card per item surfaces (the largest gain), so a field of throttled
/// miners is one idea, not a nag list.
fn under_extracted(
    state: &PlanState,
    gd: &GameData,
    derived: &Derived,
    world: &WorldSnapshot,
    out: &mut Vec<Candidate>,
) {
    let demand = production_gaps(state, derived);
    // item → (gain, claim id, node id, purity, clock, factory)
    struct Pick {
        gain: f64,
        magnitude: f64,
        claim: Id,
        node: String,
        purity: String,
        clock: f64,
        factory: Id,
    }
    let mut best: BTreeMap<String, Pick> = BTreeMap::new();
    for c in state.node_claims.values() {
        if c.clock >= 1.0 - 1e-9 {
            continue;
        }
        // Save-only claims resolve to no catalog node: no item, no purity,
        // no honest number — silence (never an actor-path title).
        let Some(node) = world.nodes.iter().find(|n| n.id == c.node) else {
            continue;
        };
        let Some(machine) = gd.machines.get(&c.extractor) else {
            continue;
        };
        let gain = extraction_rate(machine, &node.purity, 1.0)
            - extraction_rate(machine, &node.purity, c.clock);
        if gain <= EPS {
            continue;
        }
        let item_gap = demand.get(&node.item).copied().unwrap_or(0.0);
        if item_gap <= EPS && !claim_ceiling_binds(state, derived, c, &node.item) {
            continue; // nobody is short of this item — not an opportunity
        }
        // Usable gain: capped by the empire production gap when that is the
        // demand signal; a ceiling-bound factory can absorb the whole raise
        // (its ceiling rises with the clock).
        let magnitude = if item_gap > EPS {
            gain.min(item_gap)
        } else {
            gain
        };
        let better = best.get(&node.item).is_none_or(|p| gain > p.gain);
        if better {
            best.insert(
                node.item.clone(),
                Pick {
                    gain,
                    magnitude,
                    claim: c.id.clone(),
                    node: node.id.clone(),
                    purity: node.purity.clone(),
                    clock: c.clock,
                    factory: c.factory.clone(),
                },
            );
        }
    }
    for (item, p) in best {
        let fname = state
            .factories
            .get(&p.factory)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| p.factory.clone());
        let mut purity = p.purity.clone();
        if let Some(first) = purity.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        out.push(Candidate {
            class: 6,
            magnitude: p.magnitude,
            opp: Opportunity {
                id: format!("under_extracted:{}", p.claim),
                kind: OpportunityKind::UnderExtracted,
                title: format!(
                    "{purity} {} node is extracting at {:.0}% clock",
                    item_name(gd, &item),
                    p.clock * 100.0
                ),
                evidence: format!(
                    "{} · claimed by {fname} · +{:.1}/min available at 100%",
                    p.node, p.gain
                ),
                item: Some(item),
                action: OpportunityAction::SelectFactory { id: p.factory },
            },
        });
    }
}

/// Class 7 — `untapped_node`: unclaimed PURE nodes within
/// [`UNTAPPED_RADIUS_M`] of any existing factory — deduped to the NEAREST
/// node per item (three coal pins in one seam are one idea), then the nearest
/// [`UNTAPPED_LIMIT`] across items. Node position resolution: a cave node's
/// ENTRANCE always wins (routes must physically go via it — a plan-local
/// override corrects the node marker, not the way in); the override applies
/// to entrance-less nodes; else the catalog x/y. With no factories there is
/// no anchor — honest silence, never a map-wide dump.
fn untapped_node(
    state: &PlanState,
    gd: &GameData,
    world: &WorldSnapshot,
    out: &mut Vec<Candidate>,
) {
    if state.factories.is_empty() {
        return;
    }
    let claimed: BTreeSet<&str> = state
        .node_claims
        .values()
        .map(|c| c.node.as_str())
        .collect();
    // item → (distance m, node id, nearest factory name)
    let mut best: BTreeMap<&str, (f64, &str, String)> = BTreeMap::new();
    for n in &world.nodes {
        if n.purity != "pure" || claimed.contains(n.id.as_str()) {
            continue;
        }
        let (nx, ny) = match &n.entrance {
            Some(e) => (e.x, e.y),
            None => match state.node_overrides.get(&n.id).and_then(|o| o.pos) {
                Some(p) => (p.x, p.y),
                None => (n.x, n.y),
            },
        };
        let Some((dist, fname)) = state
            .factories
            .values()
            .map(|f| ((f.position.x - nx).hypot(f.position.y - ny), &f.name))
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        else {
            continue;
        };
        if dist > UNTAPPED_RADIUS_M {
            continue;
        }
        let replace = best
            .get(n.item.as_str())
            .is_none_or(|(d, id, _)| (dist, n.id.as_str()) < (*d, *id));
        if replace {
            best.insert(n.item.as_str(), (dist, &n.id, fname.clone()));
        }
    }
    let mut near: Vec<(f64, &str, &str, String)> = best
        .into_iter()
        .map(|(item, (dist, id, fname))| (dist, id, item, fname))
        .collect();
    near.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(b.1))
    });
    for (dist, id, item, fname) in near.into_iter().take(UNTAPPED_LIMIT) {
        out.push(Candidate {
            class: 7,
            // Negated so the shared magnitude-DESC sort reads distance ASC.
            magnitude: -dist,
            opp: Opportunity {
                id: format!("untapped_node:{id}"),
                kind: OpportunityKind::UntappedNode,
                title: format!("Pure {} node near {fname}, unclaimed", item_name(gd, item)),
                evidence: format!("~{dist:.0} m from {fname} · pure · {id}"),
                item: Some(item.to_string()),
                action: OpportunityAction::SelectNode { id: id.to_string() },
            },
        });
    }
}

/// Derive the ranked next-move list. Pure over its inputs; compute-on-demand
/// (the dev bridge / shell call it behind `solve_all_readonly`, exactly like
/// the advisor feed) — no persistence, nothing undoable.
///
/// Ranking: class ASC (the family order above: broken → milestone → savings
/// → growth), then magnitude DESC within the class (each class's magnitude is
/// a single unit — MW, items/min, machines, negated meters — never mixed),
/// then the deterministic id. Capped at [`CAP`].
pub fn derive_opportunities(
    state: &PlanState,
    gd: &GameData,
    derived: &Derived,
    world: &WorldSnapshot,
    unlocked: &BTreeSet<String>,
    purchased: &BTreeSet<String>,
    prefs: &NextPreferences,
) -> Vec<Opportunity> {
    let mut cands: Vec<Candidate> = Vec::new();
    power_deficit(derived, prefs, &mut cands);
    deficit_repair(state, gd, derived, &mut cands);
    route_bottleneck_fix(state, gd, derived, prefs, &mut cands);
    power_margin(derived, prefs, &mut cands);
    milestone_gap(gd, purchased, derived, &mut cands);
    alt_adopt(state, gd, unlocked, &mut cands);
    under_extracted(state, gd, derived, world, &mut cands);
    untapped_node(state, gd, world, &mut cands);

    cands.sort_by(|a, b| {
        a.class
            .cmp(&b.class)
            .then_with(|| {
                b.magnitude
                    .partial_cmp(&a.magnitude)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.opp.id.cmp(&b.opp.id))
    });
    cands.truncate(CAP);
    cands.into_iter().map(|c| c.opp).collect()
}
