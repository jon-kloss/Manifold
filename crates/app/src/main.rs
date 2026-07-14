//! FICSIT Planner — Tauri 2 shell. Custom titlebar (decorations off), commands
//! per SDD §4, `state://patch` events after every committed mutation.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use app::jobs::{now_rfc3339, JobProgress, JobRegistry};
use app::session::{EditResponse, ProposalConsequence, Session, SessionError};
use app::wizard::WizardGoal;
use planner_core::commands::Command;
use tauri::{Emitter, Manager, State};

struct AppState(Mutex<Session>);
struct Jobs(JobRegistry);

#[tauri::command]
fn hydrate(state: State<AppState>) -> serde_json::Value {
    state.0.lock().unwrap().hydrate()
}

#[tauri::command]
fn plan_edit(
    window: tauri::Window,
    state: State<AppState>,
    cmds: Vec<Command>,
) -> Result<EditResponse, SessionError> {
    let resp = state.0.lock().unwrap().edit(cmds)?;
    let _ = window.emit("state://patch", &resp);
    Ok(resp)
}

#[tauri::command]
fn plan_undo(
    window: tauri::Window,
    state: State<AppState>,
) -> Result<Option<EditResponse>, SessionError> {
    let resp = state.0.lock().unwrap().undo()?;
    if let Some(r) = &resp {
        let _ = window.emit("state://patch", r);
    }
    Ok(resp)
}

#[tauri::command]
fn plan_redo(
    window: tauri::Window,
    state: State<AppState>,
) -> Result<Option<EditResponse>, SessionError> {
    let resp = state.0.lock().unwrap().redo()?;
    if let Some(r) = &resp {
        let _ = window.emit("state://patch", r);
    }
    Ok(resp)
}

#[tauri::command]
fn set_view_state(state: State<AppState>, json: String) -> Result<(), SessionError> {
    state.0.lock().unwrap().set_view_state(&json)
}

#[tauri::command]
fn wizard_solve(state: State<AppState>, jobs: State<Jobs>, goal: WizardGoal) -> String {
    let s = state.0.lock().unwrap();
    jobs.0.start(
        s.state.clone(),
        s.gamedata.clone(),
        s.world.clone(),
        goal,
        s.unlocked.clone(),
        s.plan_hash(),
        now_rfc3339(),
    )
}

#[tauri::command]
fn wizard_progress(jobs: State<Jobs>, job_id: String, after: usize) -> Option<JobProgress> {
    jobs.0.progress(&job_id, after)
}

#[tauri::command]
fn wizard_cancel(jobs: State<Jobs>, job_id: String) -> bool {
    jobs.0.cancel(&job_id)
}

#[tauri::command]
fn t2_optimize(
    state: State<AppState>,
    factory: String,
) -> Option<planner_core::proposals::Proposal> {
    let s = state.0.lock().unwrap();
    let mut p = app::wizard::t2_optimize(&s.state, &s.gamedata, &s.unlocked, &factory);
    if let Some(pr) = p.as_mut() {
        pr.input_hash = s.plan_hash();
        pr.snapshot_time = now_rfc3339();
    }
    p
}

#[tauri::command]
fn advisor_dismiss(state: State<AppState>, id: String) -> app::advisor::AdvisorFeed {
    state.0.lock().unwrap().advisor_dismiss(&id)
}

#[tauri::command]
fn advisor_unmute(state: State<AppState>, rule: String) -> app::advisor::AdvisorFeed {
    state.0.lock().unwrap().advisor_unmute(&rule)
}

#[tauri::command]
fn advisor_pause(state: State<AppState>, paused: bool) -> app::advisor::AdvisorFeed {
    state.0.lock().unwrap().advisor_set_paused(paused)
}

#[tauri::command]
fn chat_send(
    state: State<AppState>,
    scope: app::chat::ContextScope,
    message: String,
) -> app::chat::ChatReply {
    app::chat::chat(&mut state.0.lock().unwrap(), &scope, &message)
}

#[tauri::command]
fn chat_context(
    state: State<AppState>,
    scope: app::chat::ContextScope,
) -> app::chat::ContextSnapshot {
    app::chat::compact_state(&mut state.0.lock().unwrap(), &scope)
}

#[tauri::command]
fn import_run(
    window: tauri::Window,
    state: State<AppState>,
    snapshot: app::import::ImportSnapshot,
) -> Result<app::session::ImportOutcome, SessionError> {
    let outcome = state.0.lock().unwrap().import_save(snapshot)?;
    if let app::session::ImportOutcome::Imported { response, .. }
    | app::session::ImportOutcome::Drift { response, .. } = &outcome
    {
        let _ = window.emit("state://patch", response);
    }
    Ok(outcome)
}

#[tauri::command]
fn proposal_accept(
    window: tauri::Window,
    state: State<AppState>,
    id: String,
) -> Result<EditResponse, SessionError> {
    let resp = state.0.lock().unwrap().accept_proposal(&id)?;
    let _ = window.emit("state://patch", &resp);
    Ok(resp)
}

#[tauri::command]
fn proposal_eval(state: State<AppState>, id: String) -> Result<ProposalConsequence, SessionError> {
    state.0.lock().unwrap().eval_proposal(&id)
}

/// W2a: plan a whole-factory replacement → store the Draft proposal and return
/// { response, proposal } so the renderer opens the review surface.
#[tauri::command]
fn cutover_plan(
    window: tauri::Window,
    state: State<AppState>,
    factory: String,
) -> Result<serde_json::Value, SessionError> {
    let mut s = state.0.lock().unwrap();
    let proposal = s.plan_replacement(factory, None)?;
    let resp = s.edit(vec![Command::CreateProposal { proposal }])?;
    let _ = window.emit("state://patch", &resp);
    let pid = resp.created.first().cloned().unwrap_or_default();
    Ok(serde_json::json!({ "response": resp, "proposal": pid }))
}

/// W2a: price a cutover's downtime on demand (scratch-solved, ripple-inclusive).
#[tauri::command]
fn cutover_downtime(
    state: State<AppState>,
    factory: String,
) -> Result<app::cutover::CutoverPlan, SessionError> {
    state.0.lock().unwrap().cutover_plan(factory)
}

/// W2b-D: empire-wide alternate-recipe optimizer — a derived, read-only ranking
/// of adopt-everywhere opportunities (no mutation).
#[tauri::command]
fn optimize_empire(state: State<AppState>) -> Vec<app::altopt::AltOpportunity> {
    let s = state.0.lock().unwrap();
    app::altopt::empire_optimize(&s.state, &s.gamedata, &s.unlocked)
}

/// W2b-D: adopt an alternate empire-wide → draft the review proposal(s) (T2 for
/// ◇, W2a Refactor for ◆). The ◆ built layer is never mutated.
#[tauri::command]
fn optimize_adopt(
    state: State<AppState>,
    recipe: String,
) -> Result<app::session::AdoptOutcome, SessionError> {
    state.0.lock().unwrap().optimize_adopt(&recipe)
}

/// Task #49: read-only trains-needed answer for a PROSPECTIVE route (no route
/// is created). Reuses the canonical transport math from the two factory pins.
#[tauri::command]
fn route_calc(
    state: State<AppState>,
    from: String,
    to: String,
    kind: planner_core::entities::RouteKind,
    demand_per_min: f64,
    item: Option<String>,
) -> Option<planner_core::transport::TrainAnswer> {
    state
        .0
        .lock()
        .unwrap()
        .route_calc(&from, &to, &kind, demand_per_min, item.as_deref())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&dir)?;
            let plan_path = dir.join("world.ficsit");
            // Real installs point FICSIT_DOCS_JSON at <install>/CommunityResources/Docs/Docs.json;
            // without it the bundled fixture keeps the app fully functional offline.
            let docs = std::env::var("FICSIT_DOCS_JSON")
                .ok()
                .and_then(|p| std::fs::read(p).ok());
            let build = std::env::var("FICSIT_GAME_BUILD").unwrap_or_else(|_| "fixture".into());
            let session = Session::open(&plan_path, docs, &build).expect("session open");
            app.manage(AppState(Mutex::new(session)));
            app.manage(Jobs(JobRegistry::default()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            hydrate,
            plan_edit,
            plan_undo,
            plan_redo,
            set_view_state,
            wizard_solve,
            wizard_progress,
            wizard_cancel,
            t2_optimize,
            import_run,
            advisor_dismiss,
            advisor_unmute,
            advisor_pause,
            chat_send,
            chat_context,
            proposal_accept,
            proposal_eval,
            cutover_plan,
            cutover_downtime,
            optimize_empire,
            optimize_adopt,
            route_calc
        ])
        .run(tauri::generate_context!())
        .expect("error while running FICSIT Planner");
}
