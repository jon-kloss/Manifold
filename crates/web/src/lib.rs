//! web — the wasm-bindgen wrapper that runs the FICSIT Planner `Session` in a
//! browser. Phase 2 proved the core compiles AND runs on
//! `wasm32-unknown-unknown` over an in-memory store; Phase 3 makes it a real
//! transport for the renderer:
//!
//! - `WebSession::new(docs, blob)` reconstructs a pre-loaded `MemoryPlanStore`
//!   from a saved snapshot blob (the worker reads it out of IndexedDB), or
//!   starts empty over the bundled fixture.
//! - `export_blob()` serializes the whole store back to bytes so the worker can
//!   `put` it to IndexedDB after every mutation (persistence is a SNAPSHOT
//!   layer, NOT a `PlanStore` impl: `PlanStore` is sync and IndexedDB is async,
//!   so the store stays the sync `MemoryPlanStore` and durability is a blob).
//! - `dispatch(cmd, args)` is ONE router that mirrors the dev-bridge route
//!   table (`crates/app/src/bin/dev_bridge.rs`) — the exact request→`Session`
//!   mapping the bridge already encodes — so the renderer's `WasmBackend`
//!   speaks the same command surface it speaks to Tauri and the dev bridge.
//!
//! v1 honesty notes:
//! - `native-http` is OFF in wasm, so `next_rank` returns the heuristic list
//!   plus an honest "needs the host runtime" error (JS `fetch` is Phase 4).
//! - The wizard SOLVE runs SYNCHRONOUSLY in the worker: `wizard_solve` runs the
//!   whole global solve inline and returns a jobId for an ALREADY-COMPLETE job;
//!   `wizard_progress` returns that finished job (done = true) with its full log
//!   and outcome. The worker thread blocks during the solve (seconds) but the
//!   UI thread stays live because the worker is off it. True streaming progress
//!   is later.

use std::collections::BTreeMap;

use app::ai::{self, RankPrep};
use app::chat::{self, ContextScope};
use app::jobs::{now_rfc3339, JobProgress, LogLine};
use app::wizard::WizardGoal;
use app::Session;
use persist::MemoryPlanStore;
use planner_core::commands::Command;
use planner_core::entities::RouteKind;
use planner_core::state::NextPreferences;
use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Serialize any `Serialize` value across the wasm boundary the way the
/// renderer expects: `json_compatible` so `BTreeMap`s become plain objects
/// (matching the TS `Record` types), not ES2015 `Map`s — identical to
/// `solver-wasm`'s convention.
fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Deserialize a dispatch argument off the wasm boundary. A malformed argument
/// is a 400-class error in dev-bridge terms; here it becomes a rejected promise
/// the renderer surfaces on its status-bar chip.
fn from_js<T: DeserializeOwned>(args: JsValue) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(args).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// A browser-resident planner session: one canonical `Session` over an
/// in-memory store, driven by the renderer through a `WasmBackend`. The worker
/// owns this and the IndexedDB snapshot around it.
#[wasm_bindgen]
pub struct WebSession {
    inner: Session,
    /// Synchronous wizard jobs (v1): a solve runs inline in the worker and its
    /// finished result is parked here, keyed by jobId, so `wizard_progress` can
    /// serve the same `{ log, done, outcome }` shape the async dev-bridge/Tauri
    /// path serves — only always already-done.
    jobs: BTreeMap<String, JobProgress>,
    /// A rank job parked between `next_rank_prepare` (which handed its messages to
    /// the host so a browser-run model — WebLLM — could produce a reply) and
    /// `next_rank_apply` (which validates that reply through the firewall). Tagged
    /// with a monotonic id so two overlapping ranks (an epoch bump lands a second
    /// prepare while the first's model call is still running) can't cross-consume:
    /// `apply` only takes the job whose id it was handed by its OWN prepare; a
    /// non-matching apply degrades to a clean heuristic. A fresh prepare replaces
    /// any stale pending job.
    pending_rank: Option<(u64, ai::RankJob)>,
    /// Monotonic tag source for `pending_rank` (see above).
    rank_seq: u64,
}

#[wasm_bindgen]
impl WebSession {
    /// Build a session. `docs_json` is the raw bytes of an uploaded `Docs.json`
    /// (real game catalog); `None` falls back to the bundled fixture, exactly
    /// like the desktop app's fixture path. `blob` is a previously-exported
    /// snapshot (from [`WebSession::export_blob`], read back out of IndexedDB);
    /// `None` starts a fresh empty plan. Panics are routed to the console for
    /// legible wasm stack traces.
    #[wasm_bindgen(constructor)]
    pub fn new(docs_json: Option<Vec<u8>>, blob: Option<Vec<u8>>) -> Result<WebSession, JsValue> {
        console_error_panic_hook::set_once();
        let store = match blob {
            Some(bytes) => MemoryPlanStore::from_snapshot_bytes(&bytes)
                .map_err(|e| JsValue::from_str(&format!("saved plan is unreadable: {e}")))?,
            None => MemoryPlanStore::new(),
        };
        // The build tag surfaces to the renderer as `gamedata.buildVersion`; the
        // UI reads `=== "fixture"` as "no real catalog yet". An uploaded Docs.json
        // (Phase 4) is a real catalog, so tag it "uploaded" — that flips the
        // first-run "upload your Docs.json" prompt off. No docs → still "fixture"
        // (the bundled catalog), byte-identical to the Phase-3 behavior.
        let build = if docs_json.is_some() {
            "uploaded"
        } else {
            "fixture"
        };
        let inner = Session::with_store(Box::new(store), docs_json, build)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(WebSession {
            inner,
            jobs: BTreeMap::new(),
            pending_rank: None,
            rank_seq: 0,
        })
    }

    /// Serialize the WHOLE store to bytes for the IndexedDB snapshot. The worker
    /// calls this after every mutating dispatch and `put`s the result under the
    /// current-plan key. Never fails observably: an in-memory store always
    /// encodes, but if it somehow could not, an empty blob is returned rather
    /// than trapping (the next mutation re-snapshots).
    pub fn export_blob(&self) -> Vec<u8> {
        self.inner.store.export_snapshot().unwrap_or_default()
    }

    /// THE router. `cmd` selects a `Session` operation mirroring the dev-bridge
    /// `(method, url)` route table; `args` carries that route's request body
    /// (the exact shapes the `WasmBackend` sends). Results marshal back with the
    /// same `json_compatible` convention the renderer already consumes, wrapped
    /// in an envelope `{ mutated, result }`.
    ///
    /// `mutated` is the Rust-driven mutation signal (M1): each arm declares
    /// whether it WROTE the store — mirroring the dev-bridge GET-vs-store-writing
    /// -POST distinction — so the worker knows, authoritatively and without a
    /// hand-kept allowlist that can drift, exactly when to snapshot to IndexedDB.
    pub fn dispatch(&mut self, cmd: &str, args: JsValue) -> Result<JsValue, JsValue> {
        let (mutated, result) = self.dispatch_inner(cmd, args)?;
        let env = js_sys::Object::new();
        js_sys::Reflect::set(
            &env,
            &JsValue::from_str("mutated"),
            &JsValue::from_bool(mutated),
        )?;
        js_sys::Reflect::set(&env, &JsValue::from_str("result"), &result)?;
        Ok(env.into())
    }

    // ---- Phase-2 convenience methods (kept; also reachable through dispatch) ----

    /// Full projection for the renderer's initial hydration.
    pub fn hydrate(&mut self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.hydrate())
    }

    /// Apply one or more commands as a single undoable step. `cmds` is a JS
    /// array of `Command` objects; returns the `EditResponse`.
    pub fn edit(&mut self, cmds: JsValue) -> Result<JsValue, JsValue> {
        let cmds: Vec<Command> = from_js(cmds)?;
        to_js(&self.inner.edit(cmds).map_err(err)?)
    }

    /// Read-only ranked next moves (heuristic engine) over a fresh solve.
    pub fn next_moves(&mut self) -> Result<JsValue, JsValue> {
        to_js(&self.inner.next_moves())
    }
}

impl WebSession {
    /// The dispatch body, returning `(mutated, result)`: the bool each arm
    /// declares as its authoritative "did this write the store?" signal, and the
    /// marshaled reply value. `dispatch` wraps this into the `{ mutated, result }`
    /// envelope the worker unwraps. Read arms return `false`; store-writing arms
    /// (the ones dev-bridge exposes as store-mutating POSTs) return `true`.
    fn dispatch_inner(&mut self, cmd: &str, args: JsValue) -> Result<(bool, JsValue), JsValue> {
        match cmd {
            // ---- core plan surface ----
            "hydrate" => Ok((false, to_js(&self.inner.hydrate())?)),
            "edit" => {
                #[derive(serde::Deserialize)]
                struct Args {
                    cmds: Vec<Command>,
                }
                let a: Args = from_js(args)?;
                let resp = self.inner.edit(a.cmds).map_err(err)?;
                Ok((true, to_js(&resp)?))
            }
            "undo" => Ok((true, to_js(&self.inner.undo().map_err(err)?)?)),
            "redo" => Ok((true, to_js(&self.inner.redo().map_err(err)?)?)),
            // Start over: wipe the plan (keep the catalog). `mutated=true` so the
            // worker snapshots the now-empty store blob to IndexedDB.
            "new_empire" => Ok((true, to_js(&self.inner.new_empire().map_err(err)?)?)),
            "set_view_state" => {
                // The renderer sends the ViewState object; the store persists it
                // as a JSON string (mirrors dev-bridge `POST /api/view`). Writes
                // the store (mutated=true), but the worker debounces its snapshot
                // (L1) since a pan/zoom fires this per gesture.
                let v: serde_json::Value = from_js(args)?;
                self.inner.set_view_state(&v.to_string()).map_err(err)?;
                Ok((true, to_js(&serde_json::json!({ "ok": true }))?))
            }

            // ---- wizard jobs (synchronous v1: solve inline, park the result) ----
            // A wizard result only becomes state when a later `edit`/accept
            // applies it, so none of these write the store.
            "wizard_solve" => {
                let goal: WizardGoal = from_js(args)?;
                let id = self.run_wizard(goal);
                Ok((false, to_js(&serde_json::json!({ "jobId": id }))?))
            }
            "wizard_progress" => {
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Args {
                    job_id: String,
                    #[serde(default)]
                    after: usize,
                }
                let a: Args = from_js(args)?;
                let progress = match self.jobs.get(&a.job_id) {
                    Some(p) => JobProgress {
                        // Serve only the log tail past `after`, same as the
                        // async registry — the renderer polls incrementally.
                        log: p.log.iter().skip(a.after).cloned().collect(),
                        done: p.done,
                        outcome: p.outcome.clone(),
                    },
                    None => return Err(JsValue::from_str("unknown job")),
                };
                let out = to_js(&progress)?;
                // L4: a terminal job has served its final result — drop it so the
                // `jobs` map does not grow for the session's lifetime. The solve
                // is synchronous (always done), so this frees it on first poll.
                if progress.done {
                    self.jobs.remove(&a.job_id);
                }
                Ok((false, out))
            }
            "wizard_cancel" => {
                // The solve already ran to completion synchronously, so there is
                // nothing to cancel; report false (nothing was in flight) and
                // drop the parked result. Honest v1 behavior.
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Args {
                    job_id: String,
                }
                let a: Args = from_js(args)?;
                let existed = self.jobs.remove(&a.job_id).is_some();
                Ok((false, to_js(&serde_json::json!({ "cancelled": existed }))?))
            }

            // ---- optimizer / proposal surface ----
            "t2_optimize" => {
                #[derive(serde::Deserialize)]
                struct Args {
                    factory: String,
                }
                let a: Args = from_js(args)?;
                let mut proposal = app::wizard::t2_optimize(
                    &self.inner.state,
                    &self.inner.gamedata,
                    &self.inner.unlocked,
                    &a.factory,
                );
                if let Some(p) = proposal.as_mut() {
                    p.input_hash = self.inner.plan_hash();
                    p.snapshot_time = now_rfc3339();
                }
                Ok((false, to_js(&serde_json::json!({ "proposal": proposal }))?))
            }
            "proposal_accept" => {
                let id = string_arg(args, "id")?;
                Ok((true, to_js(&self.inner.accept_proposal(&id).map_err(err)?)?))
            }
            "proposal_eval" => {
                let id = string_arg(args, "id")?;
                Ok((false, to_js(&self.inner.eval_proposal(&id).map_err(err)?)?))
            }
            // W2a: plan a whole-factory replacement → store the Draft proposal and
            // return { response, proposal } so the renderer opens review.
            "plan_replacement" => {
                let fid = string_arg(args, "factory")?;
                let proposal = self.inner.plan_replacement(fid, None).map_err(err)?;
                let resp = self
                    .inner
                    .edit(vec![Command::CreateProposal { proposal }])
                    .map_err(err)?;
                let pid = resp.created.first().cloned().unwrap_or_default();
                Ok((
                    true,
                    to_js(&serde_json::json!({ "response": resp, "proposal": pid }))?,
                ))
            }
            "cutover_plan" => {
                let fid = string_arg(args, "factory")?;
                Ok((false, to_js(&self.inner.cutover_plan(fid).map_err(err)?)?))
            }
            "optimize_empire" => Ok((
                false,
                to_js(&app::altopt::empire_optimize(
                    &self.inner.state,
                    &self.inner.gamedata,
                    &self.inner.unlocked,
                ))?,
            )),
            "optimize_adopt" => {
                let recipe = string_arg(args, "recipe")?;
                Ok((
                    true,
                    to_js(&self.inner.optimize_adopt(&recipe).map_err(err)?)?,
                ))
            }

            // ---- read-only opportunity / rank surface ----
            "next_moves" => Ok((
                false,
                to_js(&serde_json::json!({ "opportunities": self.inner.next_moves() }))?,
            )),
            // native-http is OFF in wasm, so there is no in-process HTTP client.
            // `next_rank` is the no-model path: prepare returns the heuristic list
            // directly (unconfigured) and execute_rank only echoes it back with an
            // honest error if a config somehow slipped through. Read-only.
            "next_rank" => {
                let resp = match ai::prepare_rank(&mut self.inner) {
                    RankPrep::Done(r) => r,
                    RankPrep::Call(job) => ai::execute_rank(job),
                };
                Ok((false, to_js(&resp)?))
            }
            // On-device model split. `next_rank_prepare` runs the under-lock half
            // for a browser-run (WebLLM) model — the host calls it ONLY once its
            // engine is ready, so it bypasses AiConfig entirely
            // (`prepare_rank_on_device`). With no candidates it returns the finished
            // heuristic response ({mode:"done"}); otherwise it parks the job and
            // hands its system+user messages to the host to run in-browser
            // ({mode:"call"}). The arg is the active model id (provenance only).
            // Read-only — it derives, it never writes the store.
            "next_rank_prepare" => {
                let model: String = from_js(args).unwrap_or_default();
                let msg = match ai::prepare_rank_on_device(&mut self.inner, &model) {
                    RankPrep::Done(r) => {
                        self.pending_rank = None;
                        serde_json::json!({ "mode": "done", "response": r })
                    }
                    RankPrep::Call(job) => {
                        self.rank_seq += 1;
                        let id = self.rank_seq;
                        let out = serde_json::json!({
                            "mode": "call",
                            "jobId": id,
                            "system": job.system_prompt(),
                            "user": job.user_message(),
                            "model": job.model_id(),
                            "maxTokens": job.max_tokens(),
                        });
                        self.pending_rank = Some((id, job));
                        out
                    }
                };
                Ok((false, to_js(&msg)?))
            }
            // `next_rank_apply` takes `{ jobId, content }`: the host-run model's
            // raw reply plus the id its OWN prepare handed it. It validates the
            // reply through the exact same firewall the native provider path uses
            // (apply_rank_reply), but ONLY when the pending job's id matches —
            // otherwise a second, overlapping rank has replaced it, so this apply
            // degrades to a clean heuristic rather than validating its content
            // against the wrong job. No/mismatched pending job → clean heuristic,
            // never an error to the UI.
            "next_rank_apply" => {
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Args {
                    job_id: u64,
                    content: String,
                }
                let arg: Args = from_js(args)?;
                let matched = matches!(&self.pending_rank, Some((id, _)) if *id == arg.job_id);
                let resp = if matched {
                    let (_, job) = self.pending_rank.take().expect("just matched");
                    ai::apply_rank_reply(job, &arg.content)
                } else {
                    match ai::prepare_rank(&mut self.inner) {
                        RankPrep::Done(r) => r,
                        RankPrep::Call(job) => ai::execute_rank(job),
                    }
                };
                Ok((false, to_js(&resp)?))
            }
            "set_next_preferences" => {
                let prefs: NextPreferences = from_js(args)?;
                Ok((
                    true,
                    to_js(&self.inner.set_next_preferences(prefs).map_err(err)?)?,
                ))
            }

            // ---- AI model config (in-memory only; never persisted) ----
            "ai_config_get" => Ok((false, to_js(&ai::config_public(&self.inner))?)),
            "ai_config_set" => {
                let update: ai::AiConfigUpdate = from_js(args)?;
                Ok((false, to_js(&ai::set_config(&mut self.inner, update))?))
            }

            // ---- import ----
            "import_run" => {
                let snapshot: app::import::ImportSnapshot = from_js(args)?;
                Ok((
                    true,
                    to_js(&self.inner.import_save(snapshot).map_err(err)?)?,
                ))
            }

            // ---- advisor ----
            "advisor_dismiss" => {
                let id = string_arg(args, "id")?;
                Ok((true, to_js(&self.inner.advisor_dismiss(&id))?))
            }
            "advisor_unmute" => {
                let rule = string_arg(args, "rule")?;
                Ok((true, to_js(&self.inner.advisor_unmute(&rule))?))
            }
            "advisor_pause" => {
                #[derive(serde::Deserialize)]
                struct Args {
                    paused: bool,
                }
                let a: Args = from_js(args)?;
                Ok((true, to_js(&self.inner.advisor_set_paused(a.paused))?))
            }

            // ---- chat ----
            // chat_send IS mutating: an intent-drafted proposal is materialized
            // via `s.edit(CreateProposal)` (chat.rs), which writes the store + an
            // undo entry. This is the arm the hand-kept allowlist missed (M1).
            "chat_send" => {
                #[derive(serde::Deserialize)]
                struct Args {
                    #[serde(default = "empire_scope")]
                    scope: ContextScope,
                    #[serde(default)]
                    message: String,
                }
                fn empire_scope() -> ContextScope {
                    ContextScope::Empire
                }
                let a: Args = from_js(args)?;
                Ok((
                    true,
                    to_js(&chat::chat(&mut self.inner, &a.scope, &a.message))?,
                ))
            }
            "chat_context" => {
                let scope: ContextScope = from_js(args).unwrap_or(ContextScope::Empire);
                Ok((false, to_js(&chat::compact_state(&mut self.inner, &scope))?))
            }

            // ---- prospective train answer (creates nothing) ----
            "route_calc" => {
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Args {
                    from: String,
                    to: String,
                    kind: RouteKind,
                    demand_per_min: f64,
                    item: Option<String>,
                }
                let a: Args = from_js(args)?;
                Ok((
                    false,
                    to_js(&self.inner.route_calc(
                        &a.from,
                        &a.to,
                        &a.kind,
                        a.demand_per_min,
                        a.item.as_deref(),
                    ))?,
                ))
            }

            other => Err(JsValue::from_str(&format!("unknown command: {other}"))),
        }
    }

    /// Run a wizard goal to completion INLINE (the v1 synchronous solve) and
    /// park the finished job so `wizard_progress` can return it. The solve is
    /// never cancellable — it has already run by the time the id returns — so
    /// the cancel flag is a permanently-false `AtomicBool`.
    fn run_wizard(&mut self, goal: WizardGoal) -> String {
        let id = planner_core::entities::new_id();
        let mut log: Vec<LogLine> = Vec::new();
        let outcome = app::wizard::global_solve(
            &self.inner.state,
            &self.inner.gamedata,
            &self.inner.world,
            &goal,
            &self.inner.unlocked,
            self.inner.plan_hash(),
            now_rfc3339(),
            |phase, line| {
                log.push(LogLine {
                    phase: phase.into(),
                    line: line.into(),
                });
            },
            &std::sync::atomic::AtomicBool::new(false),
        );
        self.jobs.insert(
            id.clone(),
            JobProgress {
                log,
                done: true,
                outcome: Some(serde_json::to_value(&outcome).unwrap_or_default()),
            },
        );
        id
    }
}

/// Session errors marshal to their string message — the renderer surfaces the
/// text on its status-bar chip (same as a dev-bridge `{ "error": … }` body).
fn err(e: app::session::SessionError) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// Pull a single named string field out of a dispatch args object (the common
/// `{ "id": … }` / `{ "factory": … }` / `{ "recipe": … }` shape).
fn string_arg(args: JsValue, key: &str) -> Result<String, JsValue> {
    let v: serde_json::Value = from_js(args)?;
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .ok_or_else(|| JsValue::from_str(&format!("missing string argument `{key}`")))
}

// The smoke test that PROVES Session runs in wasm (not just compiles): build a
// WebSession over the fixture, hydrate, apply one edit, and assert the derived
// state actually changed. Runs under `wasm-pack test --node` / the
// wasm-bindgen test runner. Guarded to wasm so a native `cargo test` skips it.
#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_smoke {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn websession_hydrates_edits_and_state_changes() {
        let mut s = WebSession::new(None, None).expect("construct WebSession over the fixture");

        // Hydrate: the initial plan has no factories.
        let before = s.hydrate().expect("hydrate");
        let before: serde_json::Value =
            serde_wasm_bindgen::from_value(before).expect("hydrate → json");
        let factories_before = before["plan"]["factories"]
            .as_object()
            .map(|m| m.len())
            .unwrap_or(0);
        assert_eq!(factories_before, 0, "fixture plan starts empty");

        // Apply one edit: create a factory (ULID minting exercises getrandom +
        // the wasm clock — the whole point of the proof).
        let cmd = serde_json::json!([{
            "type": "create_factory",
            "name": "WASM WORKS",
            "position": { "x": 1.0, "y": 2.0, "z": 0.0 },
            "region": "GRASS FIELDS"
        }]);
        let cmds = serde_wasm_bindgen::to_value(&cmd).expect("cmd → js");
        let resp = s.edit(cmds).expect("edit applies");
        let resp: serde_json::Value =
            serde_wasm_bindgen::from_value(resp).expect("edit response → json");
        assert_eq!(
            resp["created"].as_array().map(|a| a.len()).unwrap_or(0),
            1,
            "the edit minted one entity (a ULID id)"
        );

        // Re-hydrate: the derived state changed — one factory now exists.
        let after = s.hydrate().expect("re-hydrate");
        let after: serde_json::Value =
            serde_wasm_bindgen::from_value(after).expect("hydrate → json");
        let factories_after = after["plan"]["factories"]
            .as_object()
            .map(|m| m.len())
            .unwrap_or(0);
        assert_eq!(
            factories_after, 1,
            "the plan gained a factory after the edit"
        );
    }

    // The Phase-3 durability proof: an edit, an export_blob, a fresh WebSession
    // reconstructed FROM that blob, and the factory is still there — the
    // IndexedDB round-trip the worker relies on, exercised end to end in wasm.
    #[wasm_bindgen_test]
    fn websession_export_blob_round_trips_through_dispatch() {
        let mut s = WebSession::new(None, None).expect("construct");
        // Build args as a plain JS object (json_compatible), exactly as the
        // renderer's WasmBackend does — the default serde-wasm serializer emits
        // ES `Map`s, which the struct deserializer would not read.
        let cmds = to_js(&serde_json::json!({
            "cmds": [{
                "type": "create_factory",
                "name": "PERSISTED",
                "position": { "x": 5.0, "y": 6.0, "z": 0.0 },
                "region": "GRASS FIELDS"
            }]
        }))
        .unwrap();
        let env = s.dispatch("edit", cmds).expect("edit via dispatch");
        // The envelope flags `edit` as a store mutation (M1) — the signal the
        // worker snapshots on.
        let env: serde_json::Value = serde_wasm_bindgen::from_value(env).unwrap();
        assert_eq!(env["mutated"], serde_json::json!(true), "edit is mutating");
        let blob = s.export_blob();
        assert!(!blob.is_empty(), "a mutated store exports a non-empty blob");

        // Reconstruct from the blob (as the worker does on reload) and hydrate.
        let mut restored = WebSession::new(None, Some(blob)).expect("reconstruct from blob");
        let after = restored
            .dispatch("hydrate", JsValue::UNDEFINED)
            .expect("hydrate restored");
        // dispatch now returns the `{ mutated, result }` envelope (M1); the
        // projection lives under `result`.
        let after: serde_json::Value = serde_wasm_bindgen::from_value(after).unwrap();
        let after = &after["result"];
        let names: Vec<String> = after["plan"]["factories"]
            .as_object()
            .map(|m| {
                m.values()
                    .filter_map(|f| f["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            names.iter().any(|n| n == "PERSISTED"),
            "the factory survived the export/import round-trip"
        );
    }
}
