//! FICSIT Planner — Tauri 2 shell. Custom titlebar (decorations off), commands
//! per SDD §4, `state://patch` events after every committed mutation.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use app::session::{EditResponse, Session, SessionError};
use planner_core::commands::Command;
use tauri::{Emitter, Manager, State};

struct AppState(Mutex<Session>);

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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            hydrate,
            plan_edit,
            plan_undo,
            plan_redo,
            set_view_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running FICSIT Planner");
}
