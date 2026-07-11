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
                (Method::Post, "/api/redo") => match s.redo() {
                    Ok(resp) => ok(&resp),
                    Err(e) => err(500, e),
                },
                (Method::Post, "/api/view") => match s.set_view_state(&body) {
                    Ok(()) => ok(&serde_json::json!({ "ok": true })),
                    Err(e) => err(500, e),
                },
                // ---- wizard jobs (SDD §5.5): solve off-thread, poll the log ----
                (Method::Post, "/api/wizard/solve") => {
                    match serde_json::from_str::<WizardGoal>(&body) {
                        Ok(goal) => {
                            let id = jobs.start(
                                s.state.clone(),
                                s.gamedata.clone(),
                                s.world.clone(),
                                goal,
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
                    let mut proposal = app::wizard::t2_optimize(&s.state, &s.gamedata, &fid);
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
