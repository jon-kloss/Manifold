//! dev-bridge — drives the real Rust core over HTTP for headless development
//! and Playwright verification (see DECISIONS.md). The renderer's Backend
//! abstraction talks to this exactly like it talks to Tauri commands; state
//! stays canonical in Rust either way.

use std::sync::Mutex;

use app::jobs::{now_rfc3339, JobRegistry};
use app::wizard::WizardGoal;
use app::Session;
use planner_core::commands::Command;
use tiny_http::{Header, Method, Response, Server};

fn json_response(status: u16, body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut r = Response::from_string(body).with_status_code(status);
    for (k, v) in [
        ("Content-Type", "application/json"),
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
        ("Access-Control-Allow-Headers", "Content-Type"),
    ] {
        r.add_header(Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap());
    }
    r
}

fn ok<T: serde::Serialize>(value: &T) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(200, serde_json::to_string(value).unwrap())
}

fn err(status: u16, message: impl std::fmt::Display) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(
        status,
        serde_json::json!({ "error": message.to_string() }).to_string(),
    )
}

fn main() -> anyhow::Result<()> {
    let plan_path = std::env::var("FICSIT_PLAN").unwrap_or_else(|_| "dev-world.ficsit".into());
    let docs = std::env::var("FICSIT_DOCS_JSON")
        .ok()
        .and_then(|p| std::fs::read(p).ok());
    let build = std::env::var("FICSIT_GAME_BUILD").unwrap_or_else(|_| "fixture".into());
    let port: u16 = std::env::var("FICSIT_BRIDGE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8791);

    let session = Mutex::new(Session::open(&plan_path, docs, &build)?);
    let jobs = JobRegistry::default();
    let server =
        Server::http(("127.0.0.1", port)).map_err(|e| anyhow::anyhow!("bind failed: {e}"))?;
    eprintln!("dev-bridge listening on http://127.0.0.1:{port} (plan: {plan_path})");

    for mut request in server.incoming_requests() {
        let url = request.url().to_string();
        let method = request.method().clone();
        let mut body = String::new();
        let _ = request.as_reader().read_to_string(&mut body);

        // PR 10: /api/next/rank is the ONLY endpoint allowed off the serial
        // loop — the provider round-trip can take whole seconds and must
        // never block hydrate/edit behind it. Prepare (candidates + config
        // snapshot) runs under the same single-lock discipline as everything
        // else; only the blocking HTTP call moves to a throwaway thread,
        // which responds by itself (tiny_http's documented pattern —
        // `Request` is Send and `respond` consumes it).
        if method == Method::Post && url == "/api/next/rank" {
            let prep = {
                let mut s = session.lock().unwrap();
                app::ai::prepare_rank(&mut s)
            };
            match prep {
                app::ai::RankPrep::Done(resp) => {
                    let _ = request.respond(ok(&resp));
                }
                app::ai::RankPrep::Call(job) => {
                    std::thread::spawn(move || {
                        let _ = request.respond(ok(&app::ai::execute_rank(job)));
                    });
                }
            }
            continue;
        }

        let response = if method == Method::Options {
            json_response(204, String::new())
        } else {
            let mut s = session.lock().unwrap();
            match (method, url.as_str()) {
                (Method::Get, "/api/hydrate") => ok(&s.hydrate()),
                (Method::Post, "/api/edit") => match serde_json::from_str::<Vec<Command>>(&body) {
                    Ok(cmds) => match s.edit(cmds) {
                        Ok(resp) => ok(&resp),
                        Err(e) => err(422, e),
                    },
                    Err(e) => err(400, e),
                },
                (Method::Post, "/api/undo") => match s.undo() {
                    Ok(resp) => ok(&resp),
                    Err(e) => err(500, e),
                },
                (Method::Post, "/api/new_empire") => match s.new_empire() {
                    Ok(resp) => ok(&resp),
                    Err(e) => err(500, e),
                },
                (Method::Post, "/api/redo") => match s.redo() {
                    Ok(resp) => ok(&resp),
                    Err(e) => err(500, e),
                },
                (Method::Post, "/api/view") => match s.set_view_state(&body) {
                    Ok(()) => ok(&serde_json::json!({ "ok": true })),
                    Err(e) => err(500, e),
                },
                // ---- desktop save-sync mirror (Tauri IPC isn't scriptable by
                // Playwright, so the bridge exposes the same ops for e2e) ----
                (Method::Get, "/api/sync/meta") => {
                    ok(&serde_json::json!({ "meta": s.sync_meta() }))
                }
                (Method::Post, "/api/sync/meta") => match s.set_sync_meta(&body) {
                    Ok(()) => ok(&serde_json::json!({ "ok": true })),
                    Err(e) => err(500, e),
                },
                // Native picker stand-in: return the fixture path the harness
                // wired via FICSIT_SYNC_SAVE (no OS dialog headless).
                (Method::Post, "/api/sync/pick") => match std::env::var("FICSIT_SYNC_SAVE") {
                    Ok(path) if !path.is_empty() => {
                        let name = std::path::Path::new(&path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ok(&serde_json::json!({ "path": path, "name": name }))
                    }
                    _ => ok(&serde_json::json!({ "path": null })),
                },
                // Read raw save bytes at a path → the renderer worker parses them.
                (Method::Post, "/api/sync/read") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let path = req["path"].as_str().unwrap_or_default();
                    match std::fs::read(path) {
                        Ok(bytes) => {
                            let mut r = Response::from_data(bytes).with_status_code(200);
                            for (k, v) in [
                                ("Content-Type", "application/octet-stream"),
                                ("Access-Control-Allow-Origin", "*"),
                            ] {
                                r.add_header(
                                    Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap(),
                                );
                            }
                            r
                        }
                        Err(e) => err(404, e),
                    }
                }
                // ---- wizard jobs (SDD §5.5): solve off-thread, poll the log ----
                (Method::Post, "/api/wizard/solve") => {
                    match serde_json::from_str::<WizardGoal>(&body) {
                        Ok(goal) => {
                            let id = jobs.start(
                                s.state.clone(),
                                s.gamedata.clone(),
                                s.world.clone(),
                                goal,
                                s.unlocked.clone(),
                                s.plan_hash(),
                                now_rfc3339(),
                            );
                            ok(&serde_json::json!({ "jobId": id }))
                        }
                        Err(e) => err(400, e),
                    }
                }
                (Method::Post, "/api/wizard/progress") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let id = req["jobId"].as_str().unwrap_or_default();
                    let after = req["after"].as_u64().unwrap_or(0) as usize;
                    match jobs.progress(id, after) {
                        Some(p) => ok(&p),
                        None => err(404, "unknown job"),
                    }
                }
                (Method::Post, "/api/wizard/cancel") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let id = req["jobId"].as_str().unwrap_or_default();
                    ok(&serde_json::json!({ "cancelled": jobs.cancel(id) }))
                }
                (Method::Post, "/api/t2/optimize") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let fid = req["factory"].as_str().unwrap_or_default().to_string();
                    let mut proposal =
                        app::wizard::t2_optimize(&s.state, &s.gamedata, &s.unlocked, &fid);
                    if let Some(p) = proposal.as_mut() {
                        p.input_hash = s.plan_hash();
                        p.snapshot_time = now_rfc3339();
                    }
                    ok(&serde_json::json!({ "proposal": proposal }))
                }
                (Method::Post, "/api/advisor/dismiss") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    ok(&s.advisor_dismiss(req["id"].as_str().unwrap_or_default()))
                }
                (Method::Post, "/api/advisor/unmute") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    ok(&s.advisor_unmute(req["rule"].as_str().unwrap_or_default()))
                }
                (Method::Post, "/api/advisor/pause") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    ok(&s.advisor_set_paused(req["paused"].as_bool().unwrap_or(false)))
                }
                (Method::Post, "/api/chat") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let scope: app::chat::ContextScope =
                        serde_json::from_value(req["scope"].clone())
                            .unwrap_or(app::chat::ContextScope::Empire);
                    let message = req["message"].as_str().unwrap_or_default();
                    ok(&app::chat::chat(&mut s, &scope, message))
                }
                (Method::Post, "/api/context") => {
                    match serde_json::from_str::<app::chat::ContextScope>(&body) {
                        Ok(scope) => ok(&app::chat::compact_state(&mut s, &scope)),
                        Err(_) => ok(&app::chat::compact_state(
                            &mut s,
                            &app::chat::ContextScope::Empire,
                        )),
                    }
                }
                (Method::Post, "/api/import/run") => {
                    match serde_json::from_str::<app::import::ImportSnapshot>(&body) {
                        Ok(snapshot) => match s.import_save(snapshot) {
                            Ok(outcome) => ok(&outcome),
                            Err(e) => err(422, e),
                        },
                        Err(e) => err(400, e),
                    }
                }
                (Method::Post, "/api/proposal/accept") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    match s.accept_proposal(req["id"].as_str().unwrap_or_default()) {
                        Ok(resp) => ok(&resp),
                        Err(e) => err(422, e),
                    }
                }
                // ---- W2a refactor/cutover ----
                // Plan a whole-factory replacement → store the Draft proposal and
                // return { response, proposal } so the renderer opens review.
                (Method::Post, "/api/cutover/plan") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let fid = req["factory"].as_str().unwrap_or_default().to_string();
                    match s.plan_replacement(fid, None) {
                        Ok(proposal) => match s.edit(vec![Command::CreateProposal { proposal }]) {
                            Ok(resp) => {
                                let pid = resp.created.first().cloned().unwrap_or_default();
                                ok(&serde_json::json!({ "response": resp, "proposal": pid }))
                            }
                            Err(e) => err(422, e),
                        },
                        Err(e) => err(422, e),
                    }
                }
                // Price the downtime of a cutover on demand (scratch-solved).
                (Method::Post, "/api/cutover/downtime") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let fid = req["factory"].as_str().unwrap_or_default().to_string();
                    match s.cutover_plan(fid) {
                        Ok(plan) => ok(&plan),
                        Err(e) => err(422, e),
                    }
                }
                // ---- PR 9 opportunity engine ----
                // Read-only ranked next moves, computed on demand over a fresh
                // solve (same species as the advisor feed) — nothing persisted.
                (Method::Get, "/api/next") => {
                    ok(&serde_json::json!({ "opportunities": s.next_moves() }))
                }
                // PR 3: set plan-scoped NEXT preferences (persisted, not undoable,
                // outside plan_hash). Returns the updated view; the renderer bumps
                // its rank epoch to re-rank.
                (Method::Post, "/api/next/preferences") => {
                    match serde_json::from_str::<planner_core::state::NextPreferences>(&body) {
                        Ok(prefs) => match s.set_next_preferences(prefs) {
                            Ok(view) => ok(&view),
                            Err(e) => err(500, e),
                        },
                        Err(e) => err(400, e),
                    }
                }
                // ---- PR 10 bring-your-own-model ranking ----
                // Config lives in memory on the Session; the GET view never
                // carries the key (hasKey boolean only — key hygiene).
                (Method::Get, "/api/ai/config") => ok(&app::ai::config_public(&s)),
                (Method::Post, "/api/ai/config") => {
                    match serde_json::from_str::<app::ai::AiConfigUpdate>(&body) {
                        Ok(update) => ok(&app::ai::set_config(&mut s, update)),
                        Err(e) => err(400, e),
                    }
                }
                // ---- W2b-D empire alternate-recipe optimizer ----
                // Read-only ranked opportunities (empty in the fixture — no
                // unlocked alternates, honest degradation).
                (Method::Get, "/api/optimize/empire") => ok(&app::altopt::empire_optimize(
                    &s.state,
                    &s.gamedata,
                    &s.unlocked,
                )),
                // Adopt one alternate empire-wide → draft the review proposal(s)
                // (T2 for ◇, W2a Refactor for ◆; ◆ never mutated).
                (Method::Post, "/api/optimize/adopt") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let recipe = req["recipe"].as_str().unwrap_or_default().to_string();
                    match s.optimize_adopt(&recipe) {
                        Ok(outcome) => ok(&outcome),
                        Err(e) => err(422, e),
                    }
                }
                // ---- task #49 train answer-sheet ----
                // Read-only trains-needed calc for a PROSPECTIVE route (no
                // route is created). Mirrors the Tauri `route_calc` command.
                (Method::Post, "/api/route/calc") => {
                    #[derive(serde::Deserialize)]
                    #[serde(rename_all = "camelCase")]
                    struct Req {
                        from: String,
                        to: String,
                        kind: planner_core::entities::RouteKind,
                        demand_per_min: f64,
                        item: Option<String>,
                    }
                    match serde_json::from_str::<Req>(&body) {
                        Ok(req) => ok(&s.route_calc(
                            &req.from,
                            &req.to,
                            &req.kind,
                            req.demand_per_min,
                            req.item.as_deref(),
                        )),
                        Err(e) => err(400, e),
                    }
                }
                (Method::Post, "/api/proposal/eval") => {
                    let req: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    match s.eval_proposal(req["id"].as_str().unwrap_or_default()) {
                        Ok(c) => ok(&c),
                        Err(e) => err(422, e),
                    }
                }
                _ => err(404, "not found"),
            }
        };
        let _ = request.respond(response);
    }
    Ok(())
}
