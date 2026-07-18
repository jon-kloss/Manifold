//! Bring-your-own-model ranking layer (PR 10, AI-next 2 of 3). The MODEL
//! NEVER CALCULATES: the opportunity engine (opportunities.rs) derives every
//! candidate and every number; a configured OpenAI-compatible endpoint only
//! RANKS and NARRATES that fixed list. One chat-completions call covers
//! OpenAI / Anthropic-compat / OpenRouter / Groq / Ollama / LM Studio.
//!
//! The honesty firewall is [`apply_model_ranking`]: a PURE function from
//! `(candidates, model reply)` to a ranked list. Reply ids that aren't
//! candidates are dropped, duplicates are dropped, candidates the model
//! omitted are appended in heuristic order, notes attach only to known ids,
//! and notes/headline are length-clamped. Cards come ONLY from `candidates` —
//! there is no code path by which model output creates a card, changes an
//! action, or rewrites a title/evidence line.
//!
//! Failure is quiet + surfaced: any provider fault (HTTP error, bad JSON,
//! timeout) returns the untouched heuristic list with a short `error` string
//! for the status-bar chip. The endpoint always answers.
//!
//! CONCURRENCY: ranking is a two-phase split. [`prepare_rank`] runs UNDER the
//! session lock (one acquisition: derive candidates, snapshot config +
//! context into a [`RankJob`]); [`execute_rank`] is pure over that owned job
//! (`Send` by construction), so the blocking provider round-trip runs OFF the
//! lock and a slow or hung endpoint never wedges hydrate/edit/solve.
//! [`rank_next_moves`] is the in-line façade over both halves.
//!
//! KEY HYGIENE: [`AiConfig`] deliberately derives neither `Serialize` nor
//! `Debug`. The key leaves the process only as the Authorization header of
//! the provider call — never echoed by GET /api/ai/config, never in hydrate,
//! never logged, never persisted (v1; the Tauri shell owns keychain
//! persistence later — see DECISIONS.md).

use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "native-http")]
use std::time::Duration;

use planner_core::state::NextPreferences;
use serde::{Deserialize, Serialize};

use crate::opportunities::Opportunity;
use crate::session::Session;

/// Cap on validated wildcard ideas (PR 3) — a brainstorm, not a backlog.
const WILDCARD_CAP: usize = 3;

/// Sanity ceiling for a model-invented wildcard `rate` (PR #11 M3). The value
/// flows verbatim into the wizard prefill; a negative, zero, non-finite, or
/// absurd figure (1e12/min) is meaningless there, so it is dropped server-side
/// (the item prefill still stands; the wizard falls back to its own default).
const WILDCARD_MAX_RATE: f64 = 100_000.0;

/// Default provider-call timeout. Configurable per session (POST
/// /api/ai/config `timeoutSecs`) so tests can run the timeout path fast.
pub const DEFAULT_TIMEOUT_SECS: u64 = 20;

/// Length clamp for model prose (headline and per-card notes) — commentary,
/// not essays. Overlong text is cut to at most this many chars INCLUDING the
/// trailing ellipsis, at a whitespace boundary (see [`clamp`]).
const PROSE_CLAMP: usize = 240;

/// The system prompt, checked in as reviewable source. The contract it states
/// is the same one [`apply_model_ranking`] enforces mechanically.
pub const RANK_SYSTEM_PROMPT: &str = "\
You are a Satisfactory factory advisor inside FICSIT Planner.
You receive the planner's derived empire state and a FIXED list of candidate next moves.
The candidates arrive pre-sorted in the planner's own heuristic order — a sane baseline; depart from it only where you see a clear reason.
The planner already did all the math. You never calculate anything.
Your only job: RANK the candidates by what the player should do first, and say why, briefly.
Rules:
- Reference only candidate ids from the given list. Never invent a candidate or an action.
- Every number you mention must appear verbatim in the provided state, candidate titles, or evidence lines. Never derive, sum, or convert numbers.
- Broken things (overdrawn grids, starved factories) usually outrank growth ideas; use judgment on ties.
- Headline: one calm sentence, at most 25 words, naming the single best next move and why it is first.
- Notes: one calm sentence each, at most 20 words, about that candidate's rank.
Reply with STRICT JSON only — no markdown, no code fences, no text before or after the JSON — exactly this shape:
{\"order\": [\"<candidate id>\", ...], \"headline\": \"<one sentence>\", \"notes\": {\"<candidate id>\": \"<one sentence>\"}}
\"order\" must list every candidate id exactly once; \"notes\" entries are optional.";

/// In-memory model endpoint config (Session-held). Defaults from env:
/// `FICSIT_AI_BASE_URL`, `FICSIT_AI_MODEL`, `FICSIT_AI_KEY`.
///
/// Deliberately NOT `Serialize`/`Debug`: the only serializable projection is
/// [`AiConfigPublic`], which carries `has_key`, never the key.
pub struct AiConfig {
    /// OpenAI-compatible base, e.g. `https://api.openai.com/v1` — the call
    /// goes to `{base_url}/chat/completions`.
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub timeout_secs: u64,
}

impl AiConfig {
    /// Build a config from any `key → value` source using the env-var key
    /// names. Split from [`Self::from_env`] so the parsing rules — trim,
    /// blank = unset — are unit-testable without touching (or racing on)
    /// real process env. Tests also use it to pin a session to a KNOWN-empty
    /// config regardless of whatever `FICSIT_AI_*` the host exports.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let get = |k: &str| {
            lookup(k)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        Self {
            base_url: get("FICSIT_AI_BASE_URL").unwrap_or_default(),
            model: get("FICSIT_AI_MODEL").unwrap_or_default(),
            api_key: get("FICSIT_AI_KEY"),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Usable for a model call: base URL + model both present. A key is NOT
    /// required (Ollama / LM Studio run keyless).
    pub fn configured(&self) -> bool {
        !self.base_url.is_empty() && !self.model.is_empty()
    }
}

/// What GET /api/ai/config returns — the ONLY serialized view of the config.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfigPublic {
    pub configured: bool,
    pub base_url: String,
    pub model: String,
    /// The key round-trips as a boolean, never as text.
    pub has_key: bool,
}

/// POST /api/ai/config body. `api_key` absent/null = keep the current key
/// (the UI's password field placeholder reads "unchanged"); empty string =
/// clear it; anything else = replace it. `timeout_secs` absent = keep;
/// present = clamped to 1..=120 (floor keeps the fast-timeout test seam,
/// ceiling keeps a fat-fingered value from wedging a rank worker for hours).
///
/// Deliberately NOT `Debug`: this struct carries the raw key in transit, and
/// key hygiene here is compile-enforced, not convention-enforced.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfigUpdate {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

pub fn config_public(s: &Session) -> AiConfigPublic {
    AiConfigPublic {
        configured: s.ai.configured(),
        base_url: s.ai.base_url.clone(),
        model: s.ai.model.clone(),
        has_key: s.ai.api_key.is_some(),
    }
}

/// Apply a config update in memory. Nothing here touches disk: v1 does not
/// persist the key (or base/model) anywhere — env + this setter are the two
/// sources, and a restart honestly forgets what was typed.
pub fn set_config(s: &mut Session, update: AiConfigUpdate) -> AiConfigPublic {
    s.ai.base_url = update.base_url.trim().trim_end_matches('/').to_string();
    s.ai.model = update.model.trim().to_string();
    match update.api_key {
        None => {}
        Some(k) if k.trim().is_empty() => s.ai.api_key = None,
        Some(k) => s.ai.api_key = Some(k.trim().to_string()),
    }
    if let Some(t) = update.timeout_secs {
        s.ai.timeout_secs = t.clamp(1, 120);
    }
    config_public(s)
}

/// The model's expected reply shape. Built by [`coerce_reply`], which is
/// lenient: MISSING fields degrade individually (no order → heuristic order; no
/// notes → no notes), and a WRONG-TYPED field is salvaged where sensible
/// (numeric `order` ids / numeric notes are stringified) or dropped, rather than
/// failing the whole parse. A reply that carries no model CONTENT — no headline,
/// no surviving wildcard, and an order/notes referencing no real candidate — is
/// treated as a schema failure by [`ranked_response`] (it must not wear the
/// `engine:"model"` badge on the untouched heuristic order).
#[derive(Debug, Default, Deserialize)]
pub struct ModelReply {
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub headline: Option<String>,
    #[serde(default)]
    pub notes: BTreeMap<String, String>,
    /// PR 3 — the ONE labeled firewall exception: ideas BEYOND the derived
    /// candidate list. Every field degrades individually (all serde-default),
    /// validated server-side in [`validate_wildcards`] before it can surface.
    #[serde(default)]
    pub wildcards: Vec<WildcardReply>,
}

/// Raw, UNTRUSTED wildcard from the model reply. `title`/`rationale` are clamped
/// prose; `item` is kept only if it exists in the catalog; `rate` is a starting
/// hint the user edits in the wizard — NEVER a solver fact.
#[derive(Debug, Default, Deserialize)]
pub struct WildcardReply {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub item: Option<String>,
    #[serde(default)]
    pub rate: Option<f64>,
}

/// A validated wildcard idea (PR 3). Structurally segregated from `Opportunity`:
/// it carries NO engine action and NO trusted numbers. The renderer fences it
/// behind an AI badge + a "solve it to make it real" disclaimer, and "TRY IT"
/// hands it to the wizard — it never writes plan state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Wildcard {
    pub title: String,
    pub rationale: String,
    /// Catalog-validated item class (dropped if unknown), for the wizard prefill.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// Untrusted starting rate the user edits — only meaningful with an `item`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
}

/// One ranked move: the untouched engine card plus (at most) an attached
/// model note. `note` is the ONLY model-writable field.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedOpportunity {
    #[serde(flatten)]
    pub opportunity: Opportunity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// POST /api/next/rank response. `engine: "heuristic"` is byte-identical in
/// card content to GET /api/next (same derivation function); `error` carries
/// the short status-bar string when a model call was attempted and failed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankResponse {
    pub engine: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub opportunities: Vec<RankedOpportunity>,
    /// PR 3 wildcard ideas — model-only and additive; omitted (skip) when empty
    /// so the heuristic/offline path stays byte-identical to PR 10.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub wildcards: Vec<Wildcard>,
}

fn heuristic(candidates: Vec<Opportunity>, error: Option<String>) -> RankResponse {
    RankResponse {
        engine: "heuristic",
        model: None,
        headline: None,
        error,
        opportunities: candidates
            .into_iter()
            .map(|opportunity| RankedOpportunity {
                opportunity,
                note: None,
            })
            .collect(),
        // The engine does not brainstorm — the offline path carries no ideas.
        wildcards: Vec::new(),
    }
}

/// Honest prose clamp: at most [`PROSE_CLAMP`] chars INCLUDING the ellipsis.
/// Overlong text keeps its first `PROSE_CLAMP - 1` chars, cut back to the
/// last whitespace so a truncation can never end mid-token — a naive cut
/// like "…margin of 1,500" → "…of 1,5" MANUFACTURES a number the model never
/// said, in text rendered under the AI badge. A single unbroken token has no
/// whitespace to cut at and falls back to the hard cut, still
/// ellipsis-marked. Char-based throughout (never splits a UTF-8 scalar).
fn clamp(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= PROSE_CLAMP {
        return t.to_string();
    }
    let head: String = t.chars().take(PROSE_CLAMP - 1).collect();
    let kept = match head.rfind(char::is_whitespace) {
        Some(i) => &head[..i],
        None => head.as_str(),
    };
    let mut out = kept.trim_end().to_string();
    out.push('…');
    out
}

/// Distinctive fragments lifted from [`RANK_SYSTEM_PROMPT`]. A small/weak model
/// (e.g. the on-device 1B) sometimes parrots an instruction bullet back as its
/// "headline" or a note instead of following it — we must never render the
/// prompt to the user, so any prose containing one of these is treated as an
/// echo and dropped (the heuristic order + titles still stand). Kept lowercase;
/// the check lowercases the candidate. Fragments are chosen to be phrases no
/// genuine one-sentence advisor headline would contain.
const PROMPT_ECHO_MARKERS: &[&str] = &[
    "at most 25 words",
    "at most 20 words",
    "one calm sentence",
    "single best next move",
    "why it is first",
    "candidate id",
    "strict json",
    "the planner already did",
    "you never calculate",
    "reply with",
    "one sentence each",
    "about that candidate",
];

/// True when `text` looks like leaked prompt instructions rather than advisor
/// prose: it opens with a bulleted field label ("Headline:" / "Notes:") or
/// contains any [`PROMPT_ECHO_MARKERS`] fragment. Case-insensitive.
fn looks_like_prompt_echo(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    if lower.starts_with("headline:") || lower.starts_with("notes:") {
        return true;
    }
    PROMPT_ECHO_MARKERS.iter().any(|m| lower.contains(m))
}

/// THE VALIDATION FIREWALL — pure, unit-tested directly. Maps the model reply
/// onto the fixed candidate list:
///
/// - unknown ids in `order` are DROPPED;
/// - duplicate ids are DROPPED (first occurrence wins);
/// - candidates missing from `order` are APPENDED in heuristic order;
/// - notes attach only to ids that survived (unknown-id notes vanish);
/// - notes and headline are length-clamped.
///
/// Every `Opportunity` in the output is moved verbatim from `candidates`:
/// model output cannot create a card, change an action, or alter a title or
/// evidence line, by construction.
pub fn apply_model_ranking(
    candidates: Vec<Opportunity>,
    reply: &ModelReply,
) -> (Option<String>, Vec<RankedOpportunity>) {
    let known: BTreeSet<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
    // Unreachable today (the engine derives ids uniquely), but a duplicate
    // candidate id would silently collapse a card below — catch it in tests.
    debug_assert_eq!(
        known.len(),
        candidates.len(),
        "engine-side candidate ids must be unique"
    );
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut order: Vec<&str> = Vec::new();
    for id in &reply.order {
        if known.contains(id.as_str()) && seen.insert(id.as_str()) {
            order.push(id.as_str());
        }
    }
    for c in &candidates {
        if seen.insert(c.id.as_str()) {
            order.push(c.id.as_str());
        }
    }
    let order: Vec<String> = order.into_iter().map(String::from).collect();
    let mut by_id: BTreeMap<String, Opportunity> =
        candidates.into_iter().map(|c| (c.id.clone(), c)).collect();
    let ranked = order
        .iter()
        .map(|id| RankedOpportunity {
            note: reply
                .notes
                .get(id)
                .map(|n| clamp(n))
                // a weak model can parrot an instruction bullet as a "note" —
                // drop it rather than leak the prompt (card still renders).
                .filter(|n| !n.is_empty() && !looks_like_prompt_echo(n)),
            opportunity: by_id.remove(id).expect("order contains only known ids"),
        })
        .collect();
    let headline = reply
        .headline
        .as_deref()
        .map(clamp)
        // never surface a headline that is really the prompt echoed back.
        .filter(|h| !h.is_empty() && !looks_like_prompt_echo(h));
    (headline, ranked)
}

/// PR 3: a one-line preferences nudge injected into the USER message (never the
/// checked-in system prompt). Empty string when no preferences are set, so the
/// no-prefs request body stays byte-identical to PR 10 (the prefs field is only
/// added to the JSON when this is non-empty). The heuristic engine filter is the
/// hard guarantee; this prose line is the documented soft nudge.
pub fn preferences_prompt(prefs: &NextPreferences) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if prefs.no_trains {
        parts.push("avoid recommending trains");
    }
    if prefs.ignore_power {
        parts.push("deprioritize power for now");
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("Player preferences: {}.", parts.join("; "))
}

/// Case-insensitive keyword hit for the wildcard preference filter (PR 3).
fn mentions_any(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|k| text.contains(k))
}

/// Validate untrusted wildcard ideas (PR 3 firewall exception): clamp prose with
/// the SAME word-boundary clamp as notes/headline, keep `item` ONLY when it is a
/// real catalog class (else drop the hint, keep the idea), drop empty-title
/// entries, honor preferences (train/power ideas are suggestions — filtered like
/// their heuristic siblings), and cap the list. `rate` is a starting hint the
/// wizard lets the user edit (never a solver fact), so it is clamped to a sane
/// positive band and dropped otherwise (PR #11 M3) — but never trusted.
fn validate_wildcards(
    raw: &[WildcardReply],
    catalog: &BTreeSet<String>,
    prefs: &NextPreferences,
) -> Vec<Wildcard> {
    let mut out: Vec<Wildcard> = Vec::new();
    for w in raw {
        let title = w.title.as_deref().map(clamp).unwrap_or_default();
        if title.is_empty() {
            continue; // no title, no idea
        }
        let rationale = w.rationale.as_deref().map(clamp).unwrap_or_default();
        // Preference filter — wildcards are all SUGGESTIONS (never facts), so a
        // train idea is dropped under `no_trains` and a power idea under
        // `ignore_power`, exactly as their heuristic counterparts hide.
        let haystack = format!(
            "{} {} {}",
            title.to_lowercase(),
            rationale.to_lowercase(),
            w.item.as_deref().unwrap_or("").to_lowercase()
        );
        if prefs.no_trains
            && mentions_any(
                &haystack,
                &[
                    "train",
                    "rail",
                    "consist",
                    "locomotive",
                    "freight",
                    "station",
                ],
            )
        {
            continue;
        }
        if prefs.ignore_power
            && mentions_any(
                &haystack,
                &["power", "generator", "grid", "megawatt", " mw"],
            )
        {
            continue;
        }
        // Item hint survives only if the catalog knows it; otherwise the idea
        // stands as pure text with no prefill (and the rate goes with it — a
        // rate without a valid item is meaningless to the wizard).
        let item = w.item.clone().filter(|i| catalog.contains(i));
        // Clamp the model-invented rate to a sane positive band (PR #11 M3):
        // finite and 0 < rate <= WILDCARD_MAX_RATE, else drop it and let the
        // wizard use its own default. A rate without a valid item is dropped too.
        let rate = w
            .rate
            .filter(|r| r.is_finite() && *r > 0.0 && *r <= WILDCARD_MAX_RATE)
            .filter(|_| item.is_some());
        out.push(Wildcard {
            title,
            rationale,
            item,
            rate,
        });
        if out.len() >= WILDCARD_CAP {
            break;
        }
    }
    out
}

/// Strip a courtesy markdown fence (```json … ```): some small models fence
/// despite instructions, and unfencing is lossless — the inner text still has
/// to parse as the strict schema or we fall back.
fn strip_fences(content: &str) -> &str {
    let t = content.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    rest.trim().strip_suffix("```").unwrap_or(rest).trim()
}

/// Salvage a [`ModelReply`] from model prose. Strip a courtesy fence, seek the
/// first JSON container ('{' or '['), then stream-deserialize exactly one value —
/// the stream iterator parses a single complete value and IGNORES anything after
/// it, so "Sure! {…} Let me know!" succeeds where a first-`{`/last-`}` window
/// would not. The parsed value is then coerced LENIENTLY (see [`coerce_reply`]):
/// small/quantized on-device models routinely get the shape right but a field's
/// TYPE slightly wrong (an `order` of numbers, a note that's a number) or emit a
/// bare array instead of the `{order:[…]}` envelope — salvaging those recovers
/// the model's work instead of throwing it away. Returns None only when there is
/// no readable JSON container at all (genuinely non-JSON output) → heuristic
/// fallback (never worse than the old strict parse). Prose braces BEFORE the
/// real JSON still fail the parse.
fn extract_reply(content: &str) -> Option<ModelReply> {
    let t = strip_fences(content);
    let start = t.find(['{', '['])?;
    let value: serde_json::Value = serde_json::Deserializer::from_str(&t[start..])
        .into_iter::<serde_json::Value>()
        .next()?
        .ok()?;
    Some(coerce_reply(value))
}

/// One model-supplied id: a JSON string, or a bare number coerced to its decimal
/// string (a weak model sometimes emits `"order":[1,2]`).
fn value_to_id(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// One note/prose value: a string, or a number/bool stringified (some models
/// answer a per-id note with a bare number).
fn value_to_prose(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Leniently map a parsed JSON value onto a [`ModelReply`]. A bare array is read
/// as the ordered id list; an object contributes each field we can read and
/// drops the rest (a wrong-typed field no longer rejects the whole reply). An
/// all-empty result is fine — the shared emptiness guard in [`ranked_response`]
/// treats it as a schema miss, so this never wears the model badge on nothing.
fn coerce_reply(value: serde_json::Value) -> ModelReply {
    use serde_json::Value;
    let mut reply = ModelReply::default();
    match value {
        Value::Array(items) => {
            reply.order = items.iter().filter_map(value_to_id).collect();
        }
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("order") {
                reply.order = items.iter().filter_map(value_to_id).collect();
            }
            reply.headline = map
                .get("headline")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            if let Some(Value::Object(notes)) = map.get("notes") {
                reply.notes = notes
                    .iter()
                    .filter_map(|(k, v)| value_to_prose(v).map(|s| (k.clone(), s)))
                    .collect();
            }
            if let Some(Value::Array(items)) = map.get("wildcards") {
                reply.wildcards = items
                    .iter()
                    .filter_map(|v| serde_json::from_value::<WildcardReply>(v.clone()).ok())
                    .collect();
            }
        }
        _ => {}
    }
    reply
}

/// Provider-call failure: a SHORT user-facing message (status-bar chip) plus
/// the HTTP status when there was one, so [`execute_rank`] can decide
/// whether a lean retry makes sense. The key never appears in any message.
#[cfg(feature = "native-http")]
struct ProviderError {
    status: Option<u16>,
    message: String,
}

#[cfg(feature = "native-http")]
impl ProviderError {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            status: None,
            message: message.into(),
        }
    }
}

/// One blocking OpenAI-compatible chat-completions call. Errors map to SHORT
/// user-facing strings (status-bar chip); the key travels only in the
/// Authorization header and never appears in any error text.
#[cfg(feature = "native-http")]
fn call_provider(
    base_url: &str,
    api_key: Option<&str>,
    timeout_secs: u64,
    body: &serde_json::Value,
) -> Result<ModelReply, ProviderError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .build();
    let url = format!("{base_url}/chat/completions");
    let mut req = agent.post(&url).set("Content-Type", "application/json");
    if let Some(key) = api_key {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    let resp = req.send_string(&body.to_string()).map_err(|e| match e {
        // HTTP-error bodies usually say WHY ("temperature is not supported",
        // "model not found") — surface a sanitized snippet: control chars
        // flattened to spaces, the key defensively stripped BEFORE the cut
        // (a truncation must never leave a partial key), first 160 chars.
        ureq::Error::Status(code, resp) => {
            let raw = resp.into_string().unwrap_or_default();
            let mut clean: String = raw
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            if let Some(key) = api_key {
                clean = clean.replace(key, "<redacted>");
            }
            let snippet: String = clean.trim().chars().take(160).collect();
            let message = if snippet.is_empty() {
                format!("model endpoint returned HTTP {code}")
            } else {
                format!("model endpoint returned HTTP {code}: {snippet}")
            };
            ProviderError {
                status: Some(code),
                message,
            }
        }
        // Transport errors (refused, DNS, timeout) print URL + cause — never
        // headers, so never the key.
        ureq::Error::Transport(t) => {
            let msg: String = t.to_string().chars().take(160).collect();
            ProviderError::plain(format!("model call failed: {msg}"))
        }
    })?;
    let text = resp
        .into_string()
        .map_err(|_| ProviderError::plain("model reply unreadable"))?;
    let envelope: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| ProviderError::plain("model reply was not JSON"))?;
    let content = envelope["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| ProviderError::plain("model reply missing message content"))?;
    extract_reply(content)
        .ok_or_else(|| ProviderError::plain("model reply did not match the rank schema"))
}

/// Everything the OFF-LOCK provider call needs, snapshotted under ONE lock
/// acquisition by [`prepare_rank`]. Owns plain data only, so it is `Send` by
/// construction and the blocking HTTP round-trip can run on any thread while
/// the session lock stays free for edits/hydrate.
///
/// `user` is the fully-serialized USER MESSAGE (empire state + candidate
/// list, one JSON string): [`execute_rank`]'s lean retry rebuilds a request
/// BODY from it without ever re-touching the session.
///
/// Without `native-http` (the wasm build) only `candidates` is read — the
/// provider fields are still snapshotted by [`prepare_rank`] for shape
/// parity, so silence the dead-code lint rather than fork the struct.
#[cfg_attr(not(feature = "native-http"), allow(dead_code))]
pub struct RankJob {
    base_url: String,
    model: String,
    api_key: Option<String>,
    timeout_secs: u64,
    candidates: Vec<Opportunity>,
    user: String,
    /// Catalog item classes (snapshotted under the lock) — the wildcard
    /// firewall keeps an `item` hint only when it names a real one.
    catalog_items: BTreeSet<String>,
    /// Plan preferences (snapshotted under the lock) — filter train/power
    /// wildcard ideas consistently with the heuristic engine.
    prefs: NextPreferences,
}

impl RankJob {
    /// The system prompt the model must be given alongside [`Self::user_message`].
    pub fn system_prompt(&self) -> &'static str {
        RANK_SYSTEM_PROMPT
    }
    /// The user message (JSON: empire state + candidates + optional preferences).
    pub fn user_message(&self) -> &str {
        &self.user
    }
    /// The model id the caller declared (echoed back on the badge).
    pub fn model_id(&self) -> &str {
        &self.model
    }
    /// Generation token budget for a HOST-run model, mirroring the native
    /// provider path (`request_body`): it scales with candidate count so the
    /// reply JSON never truncates mid-string at megabase scale — a flat cap
    /// would MANUFACTURE the very parse failure the firewall exists to prevent.
    pub fn max_tokens(&self) -> usize {
        256 + 48 * self.candidates.len()
    }
}

/// Apply a raw model reply — text produced by a model the HOST ran (a browser
/// WebGPU/WASM model, say) — to the job through the exact same validation
/// firewall the native provider path uses. Pure: no HTTP, safe in wasm. A reply
/// that isn't parseable JSON degrades to the heuristic list with a surfaced
/// error — identical to a failed provider call. EMPTY content is a QUIET degrade
/// (no error): it means the model produced nothing — an aborted or torn-down
/// generation (e.g. the user disabled on-device AI mid-rank) — not a parse
/// failure worth flagging on the status chip.
pub fn apply_rank_reply(job: RankJob, content: &str) -> RankResponse {
    // On-device (host-run) models are best-effort enrichment. Anything that
    // isn't a usable ranking — empty output, pure prose, or JSON that carries no
    // ranking — degrades QUIETLY to the heuristic order (no error chip): the
    // engine badge already reads "heuristic", so flagging every flaky local
    // generation as an error is noise, not signal. (The native provider path
    // keeps its loud, actionable errors — a configured endpoint is expected to
    // return valid JSON.)
    match extract_reply(content) {
        Some(reply) => ranked_response(job, &reply, true),
        None => heuristic(job.candidates, None),
    }
}

/// Outcome of the under-lock half of a rank: either the answer is already
/// known (unconfigured / nothing to rank) or a [`RankJob`] remains to be
/// executed OFF the session lock.
pub enum RankPrep {
    Done(RankResponse),
    Call(RankJob),
}

/// Rank-call projection of the empire snapshot: [`crate::chat::compact_state`]
/// (Empire scope) REUSED, then post-processed for the model — the chat surface
/// and the rank call share ONE derivation; this is a view over it, not a fork.
/// Measured on an 80-factory megabase the unprojected payload was ~16.7k chars
/// (past Ollama's default 4k-token context — silent truncation for exactly the
/// local-small-model user the settings hint courts):
///
/// - factories lose their `id`: NAMES are the join key everywhere the model
///   reads (titles, evidence, deficit rows), and 80 ULIDs are ~2k chars the
///   model never needs.
/// - zero-rate outputs are dropped, EXCEPT on factories whose `status` is
///   `built`: a BUILT factory producing zero is the anomaly itself —
///   filtering it would hide the WHY behind the deficit cards — while
///   planned/under-construction zeros are definitional clutter (nothing
///   unbuilt produces yet).
/// - deficit rows swap the factory ULID for the factory NAME and drop the
///   `port`/`route` ids the model cannot join on anything.
/// - circuits and totals pass through untouched.
pub fn rank_state(s: &mut Session) -> serde_json::Value {
    let mut state = crate::chat::compact_state(s, &crate::chat::ContextScope::Empire).payload;
    if let Some(factories) = state.get_mut("factories").and_then(|f| f.as_array_mut()) {
        for f in factories {
            let built = f.get("status").and_then(|st| st.as_str()) == Some("built");
            let Some(obj) = f.as_object_mut() else {
                continue;
            };
            obj.remove("id");
            if !built {
                if let Some(outputs) = obj.get_mut("outputs").and_then(|o| o.as_object_mut()) {
                    outputs.retain(|_, rate| rate.as_f64().unwrap_or(0.0) > 0.0);
                }
            }
        }
    }
    if let Some(deficits) = state.get_mut("deficits").and_then(|d| d.as_array_mut()) {
        for row in deficits {
            let name = row
                .get("factory")
                .and_then(|id| id.as_str())
                .and_then(|id| s.state.factories.get(id))
                .map(|f| f.name.clone());
            let Some(obj) = row.as_object_mut() else {
                continue;
            };
            obj.remove("port");
            obj.remove("route");
            if let Some(name) = name {
                obj.insert("factory".into(), serde_json::Value::String(name));
            }
        }
    }
    state
}

/// PHASE 1 (under the session lock — the caller's lock scope should end the
/// moment this returns). Candidates come from the SAME derivation as GET
/// /api/next ([`Session::next_moves`] — never a second source of truth);
/// config + context are snapshotted so nothing later needs `&Session`.
pub fn prepare_rank(s: &mut Session) -> RankPrep {
    if !s.ai.configured() {
        // No provider configured → the free heuristic list, no model call.
        return RankPrep::Done(heuristic(s.next_moves(), None));
    }
    build_rank_prep(s, None)
}

/// ON-DEVICE variant (Web/WebLLM): the model runs in the browser, so there is
/// no `base_url`/key to configure — [`AiConfig`] is bypassed entirely. This
/// keeps the on-device feature independent of the bring-your-own-model config
/// surface: with the browser engine ready the host drives prepare → run →
/// [`apply_rank_reply`]; with it absent, plain [`prepare_rank`] still returns a
/// clean heuristic (no "provider unavailable" error). `model` is carried only
/// for provenance — the browser engine already holds the weights.
pub fn prepare_rank_on_device(s: &mut Session, model: &str) -> RankPrep {
    build_rank_prep(s, Some(model))
}

/// Shared body of the two `prepare_rank*` entry points: snapshot candidates +
/// context into a self-contained [`RankJob`] (nothing later needs `&Session`).
/// `on_device` set → the browser runs the model (empty base_url/key); unset →
/// the job carries the configured provider coordinates.
fn build_rank_prep(s: &mut Session, on_device: Option<&str>) -> RankPrep {
    let candidates = s.next_moves();
    if candidates.is_empty() {
        // Nothing to rank — honest silence needs no model call.
        return RankPrep::Done(heuristic(candidates, None));
    }
    // Context = the SAME empire snapshot the chat surface shows the user
    // (chat::compact_state), post-projected for the rank call by
    // [`rank_state`]: names replace ULIDs, definitional zeros drop out.
    let state = rank_state(s);
    let cand_view: Vec<serde_json::Value> = candidates
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "kind": c.kind,
                "title": c.title,
                "evidence": c.evidence,
            })
        })
        .collect();
    // PR 3: inject the preferences line into the USER message. Added as a JSON
    // field ONLY when non-empty, so the no-prefs payload is byte-identical to
    // PR 10 (pins the request-shape tests).
    let prefs = s.state.meta.preferences.clone();
    let prefs_line = preferences_prompt(&prefs);
    let mut user_obj = serde_json::json!({ "state": state, "candidates": cand_view });
    if !prefs_line.is_empty() {
        user_obj["preferences"] = serde_json::Value::String(prefs_line);
    }
    let user = user_obj.to_string();
    let (base_url, model, api_key) = match on_device {
        Some(m) => (String::new(), m.to_string(), None),
        None => (
            s.ai.base_url.trim_end_matches('/').to_string(),
            s.ai.model.clone(),
            s.ai.api_key.clone(),
        ),
    };
    RankPrep::Call(RankJob {
        base_url,
        model,
        api_key,
        timeout_secs: s.ai.timeout_secs,
        candidates,
        user,
        catalog_items: s.gamedata.items.keys().cloned().collect(),
        prefs,
    })
}

/// Build the chat-completions body from the job. `lean` omits every OPTIONAL
/// param — `temperature`, `response_format`, `max_tokens` — for the one-shot
/// 400/422 retry (strict endpoints reject knobs they don't support).
/// `max_tokens` scales with the candidate count: a flat cap would truncate
/// the reply JSON mid-string at megabase scale and MANUFACTURE the very
/// parse failure it exists to prevent.
#[cfg(feature = "native-http")]
fn request_body(job: &RankJob, lean: bool) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": job.model,
        "messages": [
            { "role": "system", "content": RANK_SYSTEM_PROMPT },
            { "role": "user", "content": job.user },
        ],
    });
    if !lean {
        body["temperature"] = serde_json::json!(0.2);
        // Honored by providers that support it, harmlessly ignored elsewhere.
        body["response_format"] = serde_json::json!({ "type": "json_object" });
        body["max_tokens"] = serde_json::json!(256 + 48 * job.candidates.len());
    }
    body
}

/// Ok-arm finish. A reply that carries no MODEL CONTENT is a schema failure:
/// `{}` buried in prose, a structurally unrelated JSON object, or an `order`/
/// `notes` made up ENTIRELY of ids that aren't candidates (which the firewall
/// drops, leaving the untouched heuristic order). Any of those would otherwise
/// ship as `engine:"model"` — a silent no-op wearing the AI badge — so they fall
/// back instead (native surfaces the actionable error; on-device stays quiet).
/// Partial replies still degrade per field (see [`ModelReply`]).
fn ranked_response(job: RankJob, reply: &ModelReply, on_device: bool) -> RankResponse {
    // Validate wildcards FIRST (catalog + preferences): a reply that carries
    // ONLY valid wildcards is still model CONTENT, not a schema failure.
    let wildcards = validate_wildcards(&reply.wildcards, &job.catalog_items, &job.prefs);
    let headline_blank = reply.headline.as_deref().unwrap_or("").trim().is_empty();
    // "Carries content" = references at least one REAL candidate (post-firewall),
    // a headline, or a surviving wildcard. Gating on raw order/notes non-emptiness
    // would let an all-unknown-id reply (e.g. `{"order":[1,2,3]}` — candidate ids
    // are structured strings, never bare numbers) wear the model badge and swallow
    // the native schema error, since apply_model_ranking drops every unknown id.
    let known: BTreeSet<&str> = job.candidates.iter().map(|c| c.id.as_str()).collect();
    let refs_candidate = reply.order.iter().any(|id| known.contains(id.as_str()))
        || reply.notes.keys().any(|id| known.contains(id.as_str()));
    if !refs_candidate && headline_blank && wildcards.is_empty() {
        return heuristic(
            job.candidates,
            (!on_device).then(|| "model reply did not match the rank schema".to_string()),
        );
    }
    let (headline, opportunities) = apply_model_ranking(job.candidates, reply);
    RankResponse {
        engine: "model",
        model: Some(job.model),
        headline,
        error: None,
        opportunities,
        wildcards,
    }
}

/// PHASE 2 (OFF the session lock — pure over the job, safe on any thread).
/// One provider call; on HTTP 400/422 exactly one retry with the optional
/// params dropped — those two statuses are how strict endpoints reject a
/// knob they don't support (reasoning tiers reject `temperature`, some
/// servers reject `response_format`/`max_tokens`). NEVER retried: 401/403
/// (auth — the same credentials fail the same way), 404 (wrong base or
/// model — the same request meets the same miss) and 429 (rate limit — an
/// immediate retry only digs the hole deeper). Every failure path answers
/// with the heuristic list plus a surfaced `error`.
///
/// Without the `native-http` feature (the wasm build), there is no blocking
/// HTTP client to make the call: return the untouched heuristic list plus a
/// clear error. The JS-`fetch` path that reinstates model ranking in the
/// browser is Phase 4 — the pure firewall (`apply_model_ranking`) and
/// `prepare_rank` stay available for it.
#[cfg(not(feature = "native-http"))]
pub fn execute_rank(job: RankJob) -> RankResponse {
    heuristic(
        job.candidates,
        Some(
            "model ranking needs the host runtime — provider call unavailable in this build".into(),
        ),
    )
}

#[cfg(feature = "native-http")]
pub fn execute_rank(job: RankJob) -> RankResponse {
    let full = request_body(&job, false);
    match call_provider(
        &job.base_url,
        job.api_key.as_deref(),
        job.timeout_secs,
        &full,
    ) {
        Ok(reply) => ranked_response(job, &reply, false),
        Err(first) if matches!(first.status, Some(400 | 422)) => {
            let lean = request_body(&job, true);
            match call_provider(
                &job.base_url,
                job.api_key.as_deref(),
                job.timeout_secs,
                &lean,
            ) {
                Ok(reply) => ranked_response(job, &reply, false),
                Err(second) => heuristic(
                    job.candidates,
                    Some(format!(
                        "{} (retried without optional params)",
                        second.message
                    )),
                ),
            }
        }
        Err(first) => heuristic(job.candidates, Some(first.message)),
    }
}

/// POST /api/next/rank, in-line: prepare + execute back to back. Correct for
/// callers that already own the session exclusively (tests, serial tools);
/// the Tauri shell and the dev bridge call the two halves separately so the
/// lock is not held across the provider round-trip.
pub fn rank_next_moves(s: &mut Session) -> RankResponse {
    match prepare_rank(s) {
        RankPrep::Done(resp) => resp,
        RankPrep::Call(job) => execute_rank(job),
    }
}

#[cfg(test)]
mod extract_tests {
    use super::*;

    // The lenient salvage layer (small on-device models are sloppy JSON writers).
    // These exercise extract_reply/coerce_reply directly — no session or provider.

    #[test]
    fn plain_envelope_parses() {
        let r =
            extract_reply(r#"{"order":["a","b"],"headline":"hi","notes":{"a":"first"}}"#).unwrap();
        assert_eq!(r.order, vec!["a", "b"]);
        assert_eq!(r.headline.as_deref(), Some("hi"));
        assert_eq!(r.notes.get("a").map(String::as_str), Some("first"));
    }

    #[test]
    fn fenced_json_is_unwrapped() {
        let r = extract_reply("```json\n{\"order\":[\"a\"]}\n```").unwrap();
        assert_eq!(r.order, vec!["a"]);
    }

    #[test]
    fn prose_before_and_after_the_json_is_ignored() {
        let r = extract_reply("Sure! {\"order\":[\"a\",\"b\"]} Hope that helps!").unwrap();
        assert_eq!(r.order, vec!["a", "b"]);
    }

    #[test]
    fn a_bare_array_is_read_as_the_order() {
        // A weak model drops the envelope and just lists ids.
        let r = extract_reply(r#"["x","y","z"]"#).unwrap();
        assert_eq!(r.order, vec!["x", "y", "z"]);
        assert!(r.headline.is_none());
    }

    #[test]
    fn numeric_order_ids_are_coerced_to_strings() {
        // `"order":[1,2]` instead of `["1","2"]` — a common small-model slip.
        let r = extract_reply(r#"{"order":[1,2,3]}"#).unwrap();
        assert_eq!(r.order, vec!["1", "2", "3"]);
    }

    #[test]
    fn a_numeric_note_is_stringified_not_rejected() {
        // A wrong-typed note used to fail the WHOLE parse; now it degrades locally.
        let r = extract_reply(r#"{"order":["a"],"notes":{"a":3}}"#).unwrap();
        assert_eq!(r.order, vec!["a"]);
        assert_eq!(r.notes.get("a").map(String::as_str), Some("3"));
    }

    #[test]
    fn non_json_prose_yields_none() {
        assert!(extract_reply("I cannot help with that.").is_none());
        assert!(extract_reply("").is_none());
    }

    #[test]
    fn empty_object_coerces_to_an_empty_reply() {
        // Parses, but carries nothing — the ranked_response guard rejects it.
        let r = extract_reply("here you go: {}").unwrap();
        assert!(r.order.is_empty() && r.headline.is_none() && r.notes.is_empty());
    }
}
