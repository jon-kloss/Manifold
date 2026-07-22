//! FICSIT Planner — Tauri 2 shell. Custom titlebar (decorations off), commands
//! per SDD §4, `state://patch` events after every committed mutation.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use app::jobs::{now_rfc3339, JobProgress, JobRegistry};
use app::session::{EditResponse, ProposalConsequence, Session, SessionError};
use app::wizard::WizardGoal;
use planner_core::commands::Command;
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

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
fn new_empire(window: tauri::Window, state: State<AppState>) -> Result<EditResponse, SessionError> {
    let resp = state.0.lock().unwrap().new_empire()?;
    let _ = window.emit("state://patch", &resp);
    Ok(resp)
}

// ---- multi-empire switcher (1.0): named plan files in the app data dir ----

/// The ACTIVE empire's plan path + the catalog needed to reopen sessions.
struct EmpireEnv {
    plan_path: Mutex<std::path::PathBuf>,
    docs: Option<Vec<u8>>,
    build: String,
}

fn plans_dir(p: &std::path::Path) -> std::path::PathBuf {
    match p.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    }
}

#[tauri::command]
fn empires_list(env: State<EmpireEnv>) -> app::empires::EmpireList {
    let path = env.plan_path.lock().unwrap();
    app::empires::list(&plans_dir(&path), &path)
}

#[tauri::command]
fn empire_create(
    state: State<AppState>,
    env: State<EmpireEnv>,
    name: String,
) -> Result<app::empires::EmpireList, String> {
    let mut path = env.plan_path.lock().unwrap();
    let dir = plans_dir(&path);
    let n = app::empires::sanitize_name(&name)?;
    let p = app::empires::ensure_absent(&dir, &n)?;
    let sess = Session::open(&p, env.docs.clone(), &env.build).map_err(|e| e.to_string())?;
    *state.0.lock().unwrap() = sess;
    *path = p;
    Ok(app::empires::list(&dir, &path))
}

#[tauri::command]
fn empire_switch(
    state: State<AppState>,
    env: State<EmpireEnv>,
    name: String,
) -> Result<app::empires::EmpireList, String> {
    let mut path = env.plan_path.lock().unwrap();
    let dir = plans_dir(&path);
    let n = app::empires::sanitize_name(&name)?;
    let p = app::empires::ensure_present(&dir, &n)?;
    let sess = Session::open(&p, env.docs.clone(), &env.build).map_err(|e| e.to_string())?;
    *state.0.lock().unwrap() = sess;
    *path = p;
    Ok(app::empires::list(&dir, &path))
}

#[tauri::command]
fn empire_rename(
    state: State<AppState>,
    env: State<EmpireEnv>,
    from: String,
    to: String,
) -> Result<app::empires::EmpireList, String> {
    let mut path = env.plan_path.lock().unwrap();
    let dir = plans_dir(&path);
    let f = app::empires::sanitize_name(&from)?;
    let t = app::empires::sanitize_name(&to)?;
    let src = app::empires::ensure_present(&dir, &f)?;
    let dst = app::empires::ensure_absent(&dir, &t)?;
    if src == *path {
        // Drop the live SQLite handle before moving the file (in-memory
        // placeholder), then reopen at the new path — reverting the move if
        // the reopen fails so the app never strands the plan.
        let mut s = state.0.lock().unwrap();
        *s = Session::in_memory(None).map_err(|e| e.to_string())?;
        app::empires::rename_files(&src, &dst)?;
        match Session::open(&dst, env.docs.clone(), &env.build) {
            Ok(sess) => {
                *s = sess;
                *path = dst;
            }
            Err(e) => {
                let _ = app::empires::rename_files(&dst, &src);
                *s = Session::open(&src, env.docs.clone(), &env.build)
                    .map_err(|e2| e2.to_string())?;
                return Err(e.to_string());
            }
        }
    } else {
        app::empires::rename_files(&src, &dst)?;
    }
    Ok(app::empires::list(&dir, &path))
}

#[tauri::command]
fn empire_delete(env: State<EmpireEnv>, name: String) -> Result<app::empires::EmpireList, String> {
    let path = env.plan_path.lock().unwrap();
    let dir = plans_dir(&path);
    let n = app::empires::sanitize_name(&name)?;
    let p = app::empires::ensure_present(&dir, &n)?;
    if p == *path {
        return Err("switch to another empire before deleting this one".into());
    }
    app::empires::delete_files(&p)?;
    Ok(app::empires::list(&dir, &path))
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

// --- desktop save-sync: remembered save path + native re-read (web parity) ---

#[tauri::command]
fn set_sync_meta(state: State<AppState>, json: String) -> Result<(), SessionError> {
    state.0.lock().unwrap().set_sync_meta(&json)
}

#[tauri::command]
fn sync_meta(state: State<AppState>) -> Option<String> {
    state.0.lock().unwrap().sync_meta()
}

/// Read the raw bytes of a save at a native path. The renderer's worker parses
/// them into an ImportSnapshot exactly as for a picked file — we only fetch
/// bytes, so auto-sync's silent timer re-read needs no OS gesture.
#[tauri::command]
fn read_save(path: String) -> Result<tauri::ipc::Response, String> {
    std::fs::read(&path)
        .map(tauri::ipc::Response::new)
        .map_err(|e| format!("read {path}: {e}"))
}

/// Open the native file picker for a `.sav` and return the chosen absolute path
/// (None on cancel). MUST be `(async)`: a plain `#[tauri::command]` runs on the
/// MAIN thread (see next_rank), and `blocking_pick_file` parks the caller on the
/// dialog result while the dialog itself needs the main event loop — a deadlock.
/// `(async)` runs it off-thread so the loop stays free to pump the picker.
#[tauri::command(async)]
fn pick_save(app: tauri::AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .add_filter("Satisfactory save", &["sav"])
        .blocking_pick_file()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
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

/// PR 9: ranked next-move opportunities — a read-only projection computed on
/// demand over a fresh solve (nothing persisted, nothing undoable).
#[tauri::command]
fn next_moves(state: State<AppState>) -> Vec<app::opportunities::Opportunity> {
    state.0.lock().unwrap().next_moves()
}

/// PR 3: set plan-scoped NEXT preferences — persisted, not undoable, outside
/// plan_hash. Returns the updated view (preferences + fresh heuristic list).
#[tauri::command]
fn set_next_preferences(
    state: State<AppState>,
    prefs: planner_core::state::NextPreferences,
) -> Result<app::session::PreferencesView, SessionError> {
    state.0.lock().unwrap().set_next_preferences(prefs)
}

/// PR 10: public view of the in-memory model config — hasKey, never the key.
#[tauri::command]
fn ai_config_get(state: State<AppState>) -> app::ai::AiConfigPublic {
    app::ai::config_public(&state.0.lock().unwrap())
}

/// PR 10: set the in-memory model config (nothing persisted in v1).
#[tauri::command]
fn ai_config_set(
    state: State<AppState>,
    update: app::ai::AiConfigUpdate,
) -> app::ai::AiConfigPublic {
    app::ai::set_config(&mut state.0.lock().unwrap(), update)
}

/// PR 10: rank-and-narrate NEXT MOVES over the same candidates as next_moves.
/// Always answers — unconfigured/failed calls fall back to the heuristic list.
///
/// `async` is LOAD-BEARING: a plain `#[tauri::command]` runs on the MAIN
/// thread, so the blocking provider round-trip would freeze the UI event
/// loop itself. With it, the command runs off-main; the session lock is held
/// only for the prepare statement below, so edits/hydrate/solves never queue
/// behind a slow or hung model endpoint.
#[tauri::command(async)]
fn next_rank(state: State<AppState>) -> app::ai::RankResponse {
    // Lock scope = this ONE statement (the guard is a temporary): prepare
    // snapshots candidates + config, execute runs with the lock released.
    let prep = app::ai::prepare_rank(&mut state.0.lock().unwrap());
    match prep {
        app::ai::RankPrep::Done(resp) => resp,
        app::ai::RankPrep::Call(job) => app::ai::execute_rank(job),
    }
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
        .plugin(tauri_plugin_dialog::init())
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
            let session = Session::open(&plan_path, docs.clone(), &build).expect("session open");
            app.manage(AppState(Mutex::new(session)));
            app.manage(EmpireEnv {
                plan_path: Mutex::new(plan_path),
                docs,
                build,
            });
            app.manage(Jobs(JobRegistry::default()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            hydrate,
            plan_edit,
            plan_undo,
            plan_redo,
            new_empire,
            empires_list,
            empire_create,
            empire_switch,
            empire_rename,
            empire_delete,
            set_view_state,
            set_sync_meta,
            sync_meta,
            read_save,
            pick_save,
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
            next_moves,
            next_rank,
            set_next_preferences,
            ai_config_get,
            ai_config_set,
            route_calc
        ])
        .run(tauri::generate_context!())
        .expect("error while running FICSIT Planner");
}
