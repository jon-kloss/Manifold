//! Proposals (SDD §3, §10; mocks 3a–3c): reviewable, partially-acceptable
//! change sets. A proposal NEVER mutates the plan — accept materializes the
//! included items' commands as ordinary ◇ planned entities in one undo step.
//!
//! Cross-references between items (a route needs the port a sibling item
//! creates) use `$alias` placeholder ids: `Id` is a string, so a command may
//! carry `"$site1.out"` where an id belongs; accept resolves aliases to the
//! real ULIDs in creation order. Placeholders never leak into canonical state
//! — an unresolved alias aborts the accept before anything commits.

use serde::{Deserialize, Serialize};

use crate::commands::Command;
use crate::entities::Id;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Draft,
    Reviewing,
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalSource {
    GlobalSolver,
    T2Optimize,
    Advisor,
    Chat,
    SaveReimport,
    /// A refactor/cutover proposal (W2a): a ◇ replacement sited beside a running
    /// ◆ factory, bound to it by a trailing `SetFactoryReplaces` alias command.
    Refactor,
}

/// Display grouping in the review panel (mock 3a: CREATE / MODIFY / CLAIM / ROUTE).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalItemKind {
    Create,
    Modify,
    Claim,
    RouteAdd,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalItem {
    pub id: Id,
    pub kind: ProposalItemKind,
    /// Unchecked rows are skipped at accept; consequences recompute live.
    pub included: bool,
    /// Entity name line, e.g. `+ INGOT POINT 2 — NEW`.
    pub label: String,
    /// One-line detail (mock 3a), e.g. `2× SMELTER · iron ingot 60/min`.
    pub detail: String,
    /// Right-aligned mono impact, e.g. `+96 MW`, `FREE ✓`.
    pub impact: String,
    /// Commands this item materializes to. Id fields may hold `$alias` refs.
    pub commands: Vec<Command>,
    /// Parallel to `commands`: alias each command's created id binds to.
    pub aliases: Vec<Option<String>>,
    /// Item ids that must be included (and accepted first) for this item.
    #[serde(default)]
    pub depends_on: Vec<Id>,
    /// SaveReimport drift payload (`import::SyncOp`) — accept applies it to
    /// the ◆ Built layer directly, the one documented exception to ◇-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<serde_json::Value>,
}

/// Milestone target carried alongside a goal: the game hands players a huge
/// total-quantity goal (e.g. 2,500 Versatile Frameworks) with no plan. A
/// milestone rides passively inside the proposal — the recipe/siting/routing
/// algorithm is untouched; only the review surface annotates the target and
/// its time-at-rate. Live "built so far" progress is a later unit's job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Milestone {
    /// Goal item class, mirrors the proposal's first goal item.
    pub item: String,
    /// Total quantity the player must accumulate.
    pub total: f64,
    /// Production rate the plan targets (per minute); total/rate = the ETA.
    pub rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: Id,
    pub source: ProposalSource,
    /// Human goal line, e.g. `PRODUCE MODULAR FRAME AT 8.0/MIN`.
    pub title: String,
    /// Goal items: (item class, rate/min). Drives the GOAL CHECK footer cell.
    pub goal: Vec<(String, f64)>,
    pub status: ProposalStatus,
    /// Review number stamp (`PROPOSAL #7`) — monotonic per plan.
    pub number: u32,
    /// RFC3339 of the solve snapshot.
    pub snapshot_time: String,
    /// FNV-1a of the plan projection at solve time; mismatch ⇒ STALE badge.
    pub input_hash: String,
    /// Provenance line fragment, e.g. `GLOBAL SOLVER · 0.8s`.
    pub provenance: String,
    pub items: Vec<ProposalItem>,
    /// Optional total-quantity target (goal-mode). serde-default so legacy
    /// plan files that predate milestones deserialize to `None` — no migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<Milestone>,
}

impl Proposal {
    pub fn item(&self, item_id: &str) -> Option<&ProposalItem> {
        self.items.iter().find(|i| i.id == item_id)
    }
}

/// Stable content hash for stale detection (std's DefaultHasher is not
/// guaranteed stable across processes; FNV-1a is).
pub fn fnv1a(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Substitute `$alias` placeholder ids inside a command using the symbol
/// table. Errors on unresolved aliases (accept must abort cleanly).
pub fn resolve_aliases(
    cmd: &Command,
    symbols: &std::collections::BTreeMap<String, Id>,
) -> Result<Command, String> {
    let mut value = serde_json::to_value(cmd).map_err(|e| e.to_string())?;
    substitute(&mut value, symbols)?;
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn substitute(
    value: &mut serde_json::Value,
    symbols: &std::collections::BTreeMap<String, Id>,
) -> Result<(), String> {
    match value {
        serde_json::Value::String(s) if s.starts_with('$') => {
            let key = s.trim_start_matches('$');
            match symbols.get(key) {
                Some(id) => *s = id.clone(),
                None => return Err(format!("unresolved proposal alias ${key}")),
            }
        }
        serde_json::Value::Array(items) => {
            for v in items {
                substitute(v, symbols)?;
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                substitute(v, symbols)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::MapPos;

    #[test]
    fn aliases_resolve_inside_commands() {
        let cmd = Command::AddRoute {
            kind: crate::entities::RouteKind::Belt { tier: 3 },
            from: "$site.out".into(),
            to: "$consumer.in".into(),
            path: vec![MapPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }],
        };
        let mut symbols = std::collections::BTreeMap::new();
        symbols.insert("site.out".to_string(), "01ABC".to_string());
        symbols.insert("consumer.in".to_string(), "01DEF".to_string());
        let resolved = resolve_aliases(&cmd, &symbols).unwrap();
        match resolved {
            Command::AddRoute { from, to, .. } => {
                assert_eq!(from, "01ABC");
                assert_eq!(to, "01DEF");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn unresolved_alias_is_an_error() {
        let cmd = Command::DeleteRoute { id: "$nope".into() };
        let symbols = std::collections::BTreeMap::new();
        assert!(resolve_aliases(&cmd, &symbols).is_err());
    }

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv1a(b"ficsit"), fnv1a(b"ficsit"));
        assert_ne!(fnv1a(b"ficsit"), fnv1a(b"ficsit2"));
    }

    fn bare_proposal(milestone: Option<Milestone>) -> Proposal {
        Proposal {
            id: "01ID".into(),
            source: ProposalSource::GlobalSolver,
            title: "PRODUCE MODULAR FRAME AT 8.0/MIN".into(),
            goal: vec![("Desc_ModularFrame_C".into(), 8.0)],
            status: ProposalStatus::Draft,
            number: 7,
            snapshot_time: "2026-07-13T00:00:00Z".into(),
            input_hash: "deadbeef".into(),
            provenance: "GLOBAL SOLVER".into(),
            items: vec![],
            milestone,
        }
    }

    #[test]
    fn milestone_round_trips() {
        let p = bare_proposal(Some(Milestone {
            item: "Desc_ModularFrame_C".into(),
            total: 2500.0,
            rate: 8.0,
        }));
        let v = serde_json::to_value(&p).unwrap();
        // present-when-Some, and camelCase like the rest of the proposal
        assert!(v["milestone"]["total"].as_f64().unwrap() == 2500.0);
        let back: Proposal = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);

        // None milestone is omitted entirely (skip_serializing_if) …
        let bare = bare_proposal(None);
        let v = serde_json::to_value(&bare).unwrap();
        assert!(v.get("milestone").is_none(), "None milestone omits the key");

        // … and legacy JSON lacking the key deserializes to None — no migration.
        let legacy: Proposal = serde_json::from_value(v).unwrap();
        assert!(legacy.milestone.is_none());
    }
}
