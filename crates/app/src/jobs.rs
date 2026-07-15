//! Wizard solve jobs: the global solver runs off-thread over cloned inputs,
//! streaming log lines into a shared buffer the UI polls (SSE-equivalent for
//! the dev bridge; Tauri emits the same shape as events). Cancellation is
//! cooperative (AtomicBool checked between phases and per demand item).

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use gamedata::docs::GameData;
use gamedata::worldnodes::WorldSnapshot;
use planner_core::state::PlanState;
use serde::Serialize;

use crate::wizard::{global_solve, WizardGoal};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub phase: String,
    pub line: String,
}

#[derive(Default)]
pub struct Job {
    pub log: Mutex<Vec<LogLine>>,
    pub cancel: AtomicBool,
    pub done: AtomicBool,
    /// Serialized `WizardOutcome` once finished.
    pub outcome: Mutex<Option<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    pub log: Vec<LogLine>,
    pub done: bool,
    pub outcome: Option<serde_json::Value>,
}

#[derive(Default)]
pub struct JobRegistry {
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}

impl JobRegistry {
    /// Spawn a solve over cloned inputs; returns the job id immediately.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &self,
        state: PlanState,
        gd: GameData,
        world: WorldSnapshot,
        goal: WizardGoal,
        unlocked: BTreeSet<String>,
        plan_hash: String,
        snapshot_time: String,
    ) -> String {
        let id = planner_core::entities::new_id();
        let job = Arc::new(Job::default());
        self.jobs.lock().unwrap().insert(id.clone(), job.clone());
        std::thread::spawn(move || {
            let sink = job.clone();
            let outcome = global_solve(
                &state,
                &gd,
                &world,
                &goal,
                &unlocked,
                plan_hash,
                snapshot_time,
                |phase, line| {
                    sink.log.lock().unwrap().push(LogLine {
                        phase: phase.into(),
                        line: line.into(),
                    });
                },
                &job.cancel,
            );
            *job.outcome.lock().unwrap() = Some(serde_json::to_value(&outcome).unwrap());
            job.done.store(true, Ordering::SeqCst);
        });
        id
    }

    /// Snapshot of a job's log tail + outcome. `after` skips lines already seen.
    pub fn progress(&self, id: &str, after: usize) -> Option<JobProgress> {
        let job = self.jobs.lock().unwrap().get(id)?.clone();
        let log = job
            .log
            .lock()
            .unwrap()
            .iter()
            .skip(after)
            .cloned()
            .collect();
        let done = job.done.load(Ordering::SeqCst);
        let outcome = job.outcome.lock().unwrap().clone();
        Some(JobProgress { log, done, outcome })
    }

    /// Cooperative cancel — instant and stateless from the UI's side (5b).
    pub fn cancel(&self, id: &str) -> bool {
        match self.jobs.lock().unwrap().get(id) {
            Some(job) => {
                job.cancel.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }
}

/// Wall-clock RFC3339 without pulling a chrono dep. `std::time::SystemTime`
/// aborts on `wasm32-unknown-unknown`, so read the clock through `web-time`
/// there (it bridges to JS `Date.now()`); native is unchanged.
pub fn now_rfc3339() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    #[cfg(target_arch = "wasm32")]
    let secs = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // days-since-epoch → civil date (Howard Hinnant's algorithm)
    let days = secs / 86_400;
    let (h, m, sec) = (secs % 86_400 / 3600, secs % 3600 / 60, secs % 60);
    let z = days as i64 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{sec:02}Z")
}
