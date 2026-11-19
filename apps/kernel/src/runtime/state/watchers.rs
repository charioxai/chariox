//! Watchers on this kernel: the owner's requests and agents' tools change them, the transport
//! tick runs what is due, and a change of health reaches whoever is responsible for one.
use super::*;
use crate::durable_state::workflow_notifications::error;
use crate::durable_state::workflow_notifications::watcher::{
    RunCommit, WatcherOperation, WatcherRecord, WatcherWritten,
};
use crate::local::{
    WatcherDryRun, WatcherHealth, WatcherManifest, WatcherMode, WatcherRequest, WatcherScript,
    WatcherState,
};
use crate::watcher::contract::{self, Input, Report};
use crate::watcher::run::{self, RunEnd, RunSpec};
use std::time::Duration;
use tokio::sync::watch;

/// The longest a poll run or a dry run may take.
const MAX_RUN: Duration = Duration::from_secs(120);
const DRY_RUN_HISTORY_MS: u64 = 24 * 60 * 60 * 1000;
/// A stream that ended waits this long before it starts again, doubling up to `MAX_BACKOFF_MS`.
const FIRST_BACKOFF_MS: u64 = 5_000;
const MAX_BACKOFF_MS: u64 = 10 * 60 * 1000;
/// Periods without a good run before a poll source reports a gap.
const GAP_PERIODS: u64 = 3;

/// What this kernel life is running, by watcher.
#[derive(Default)]
pub(crate) struct WatcherRuns {
    live: std::sync::Mutex<BTreeMap<String, LiveRun>>,
    /// Tells each run apart, so a run that ends removes only itself.
    next_run: std::sync::atomic::AtomicU64,
    /// Check Now asks for a poll run before its time.
    check_now: std::sync::Mutex<BTreeSet<String>>,
    /// When an ended stream may start again, and how many times in a row it ended.
    stream_backoff: std::sync::Mutex<BTreeMap<String, (u64, u32)>>,
}

struct LiveRun {
    run: u64,
    revision: u64,
    stop: watch::Sender<bool>,
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl WatcherRuns {
    /// A stopped run stays live until it has ended and ended what it left: no new run of the
    /// watcher starts beside it, so its cleanup, which ends every process of the watcher's
    /// claim, can't reach a newer run.
    fn stop(&self, watcher_id: &str) {
        if let Some(run) = lock(&self.live).get(watcher_id) {
            let _ = run.stop.send(true);
        }
    }
}

fn claim(watcher_id: &str) -> String {
    format!("watcher:{watcher_id}")
}

/// Beside the kernel's own state, so each kernel keeps its watchers' packages apart.
fn package_dir(store: &DurableKernelStateStore, watcher_id: &str) -> std::path::PathBuf {
    store
        .path()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("watchers")
        .join(watcher_id)
}

/// Writes the script the record holds; the record, not the directory, is the source of truth.
fn materialize(
    store: &DurableKernelStateStore,
    record: &WatcherRecord,
) -> Result<std::path::PathBuf, String> {
    let dir = package_dir(store, &record.watcher_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("package directory: {e}"))?;
    let path = dir.join(&record.script.file_name);
    std::fs::write(&path, &record.script.text).map_err(|e| format!("script: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("script: {e}"))?;
    }
    Ok(dir)
}

fn random_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl KernelRuntimeState {
    pub(crate) async fn execute_watcher_request(
        &self,
        request: WatcherRequest,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.watcher_request(request, caller_user_id, None).await
    }

    /// `agent` is the agent that asked through its tools: it becomes responsible for what it makes.
    pub(in crate::runtime::state) async fn watcher_request(
        &self,
        request: WatcherRequest,
        caller: &str,
        agent: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let store = &self.owned.durable_state_store;
        let summary = |written: WatcherWritten| LocalDaemonResponse::Watcher {
            watcher: written.record.summary(),
        };
        match request {
            WatcherRequest::Create {
                session_id,
                manifest,
                script,
            } => {
                contract::validate(&manifest, &script).map_err(error)?;
                let session = self
                    .owned
                    .session_store
                    .read()
                    .get_session(&session_id)
                    .map_err(|_| error("Room not found"))?;
                if session.status() == crate::session::SessionStatus::Ended
                    || self.notification_guest(session.owner_user_id(), caller)
                {
                    return Err(error(
                        "a watcher is a source of the Room's owner; only the owner and the owner's agents make one",
                    ));
                }
                let watcher_id = random_id();
                let record = WatcherRecord {
                    source_id: format!("watcher_{watcher_id}"),
                    watcher_id,
                    owner_id: session.owner_user_id().into(),
                    kernel_id: self.owned.config_projection.snapshot().daemon_id,
                    session_id,
                    creator_agent_id: agent.map(str::to_owned),
                    manifest,
                    script,
                    state: WatcherState::Draft,
                    health: WatcherHealth::Ok,
                    revision: 1,
                    dry_run: None,
                    cursor: None,
                    reading: None,
                    last_check_ms: None,
                    last_ok_ms: None,
                    active_since_ms: None,
                    last_fired_ms: None,
                    fired_count: 0,
                    fired_window: Vec::new(),
                    held: Vec::new(),
                    held_release_ms: None,
                    last_failure_output: None,
                    created_ms: crate::session::unix_epoch_ms(),
                };
                Ok(summary(store.watcher_write(WatcherOperation::Create(
                    Box::new(record),
                ))?))
            }
            WatcherRequest::Update {
                watcher_id,
                manifest,
                script,
            } => {
                contract::validate(&manifest, &script).map_err(error)?;
                let record = self.caller_watcher(&watcher_id, caller)?;
                let written = store.watcher_write(WatcherOperation::Change {
                    owner: record.owner_id,
                    watcher_id: watcher_id.clone(),
                    manifest,
                    script,
                })?;
                self.owned.watcher_runs.stop(&watcher_id);
                Ok(summary(written))
            }
            WatcherRequest::DryRun { watcher_id } => {
                let record = self.caller_watcher(&watcher_id, caller)?;
                let result = dry_run(store, &record).await?;
                store.watcher_write(WatcherOperation::DryRunPassed {
                    owner: record.owner_id,
                    watcher_id,
                    revision: record.revision,
                    result: result.clone(),
                })?;
                Ok(LocalDaemonResponse::WatcherDryRun { result })
            }
            WatcherRequest::Activate { watcher_id } => {
                self.set_watcher_state(&watcher_id, caller, WatcherState::Active)
            }
            WatcherRequest::Pause { watcher_id, paused } => {
                let state = if paused {
                    WatcherState::Paused
                } else {
                    WatcherState::Active
                };
                self.set_watcher_state(&watcher_id, caller, state)
            }
            WatcherRequest::CheckNow { watcher_id } => {
                let record = self.caller_watcher(&watcher_id, caller)?;
                if record.state != WatcherState::Active
                    || !matches!(record.manifest.mode, WatcherMode::Poll { .. })
                {
                    return Err(error("only a watcher that is on and polls can check now"));
                }
                lock(&self.owned.watcher_runs.check_now).insert(watcher_id);
                self.schedule_watchers();
                Ok(LocalDaemonResponse::Watcher {
                    watcher: record.summary(),
                })
            }
            WatcherRequest::Rename { watcher_id, name } => {
                let record = self.caller_watcher(&watcher_id, caller)?;
                Ok(summary(store.watcher_write(WatcherOperation::Rename {
                    owner: record.owner_id,
                    watcher_id,
                    name: contract::name(&name).map_err(error)?,
                })?))
            }
            WatcherRequest::Remove { watcher_id } => {
                let record = self.caller_watcher(&watcher_id, caller)?;
                store.watcher_write(WatcherOperation::Remove {
                    owner: record.owner_id,
                    watcher_id: watcher_id.clone(),
                })?;
                self.owned.watcher_runs.stop(&watcher_id);
                let _ = std::fs::remove_dir_all(package_dir(store, &watcher_id));
                Ok(LocalDaemonResponse::WatcherRemoved { watcher_id })
            }
            WatcherRequest::List {} => {
                let watchers: Vec<_> = store
                    .watchers(Some(caller))?
                    .iter()
                    .map(WatcherRecord::summary)
                    .collect();
                // A follower is held by the source kernel wherever it is, so it stays listed after
                // the Room that made the source is deleted and when it follows from another kernel.
                let followers = store
                    .notification_inventory(caller)?
                    .1
                    .into_iter()
                    .filter(|follower| {
                        watchers
                            .iter()
                            .any(|watcher| watcher.source_id == follower.source_id)
                    })
                    .collect();
                Ok(LocalDaemonResponse::Watchers {
                    watchers,
                    followers,
                })
            }
        }
    }

    pub(in crate::runtime::state) fn caller_watcher(
        &self,
        watcher_id: &str,
        caller: &str,
    ) -> Result<WatcherRecord, DaemonError> {
        self.owned
            .durable_state_store
            .watcher(watcher_id)?
            .filter(|record| !self.notification_guest(&record.owner_id, caller))
            .ok_or_else(|| error("watcher not found"))
    }

    fn set_watcher_state(
        &self,
        watcher_id: &str,
        caller: &str,
        state: WatcherState,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let record = self.caller_watcher(watcher_id, caller)?;
        let written = self
            .owned
            .durable_state_store
            .watcher_write(WatcherOperation::SetState {
                owner: record.owner_id,
                watcher_id: watcher_id.into(),
                state,
                at: crate::session::unix_epoch_ms(),
            })?;
        if state == WatcherState::Active {
            self.schedule_watchers();
        } else {
            self.owned.watcher_runs.stop(watcher_id);
        }
        Ok(LocalDaemonResponse::Watcher {
            watcher: written.record.summary(),
        })
    }

    /// Starts each run that is due, releases held events and notices gaps. Runs on the transport
    /// tick, so watchers start again after a kernel restart without a request.
    pub(crate) fn schedule_watchers(&self) {
        let store = &self.owned.durable_state_store;
        let runs = &self.owned.watcher_runs;
        let records = match store.watchers(None) {
            Ok(records) => records,
            Err(e) => {
                tracing::warn!(error = %e, "watchers could not be read");
                return;
            }
        };
        let now = crate::session::unix_epoch_ms();
        for (watcher_id, run) in lock(&runs.live).iter() {
            let current = records.iter().any(|record| {
                &record.watcher_id == watcher_id
                    && record.state == WatcherState::Active
                    && record.revision == run.revision
            });
            if !current {
                let _ = run.stop.send(true);
            }
        }
        for record in records
            .into_iter()
            .filter(|record| record.state == WatcherState::Active)
        {
            if record.held_release_ms.is_some_and(|due| due <= now) {
                write_and_tell(
                    store,
                    WatcherOperation::Release {
                        watcher_id: record.watcher_id.clone(),
                        at: now,
                    },
                );
            }
            if lock(&runs.live).contains_key(&record.watcher_id) {
                continue;
            }
            let due = match record.manifest.mode {
                WatcherMode::Poll { every_s } => {
                    let period = every_s * 1000;
                    let good = record.last_ok_ms.max(record.active_since_ms).unwrap_or(now);
                    if record.health == WatcherHealth::Ok
                        && now.saturating_sub(good) > GAP_PERIODS * period
                    {
                        write_and_tell(
                            store,
                            WatcherOperation::Gap {
                                watcher_id: record.watcher_id.clone(),
                                at: now,
                            },
                        );
                    }
                    lock(&runs.check_now).remove(&record.watcher_id)
                        || record
                            .last_check_ms
                            .is_none_or(|last| now >= last.saturating_add(period))
                }
                WatcherMode::Stream => lock(&runs.stream_backoff)
                    .get(&record.watcher_id)
                    .is_none_or(|(after, _)| now >= *after),
            };
            if due {
                self.start_watcher_run(record);
            }
        }
    }

    fn start_watcher_run(&self, record: WatcherRecord) {
        let (stop, stopped) = watch::channel(false);
        let run = self
            .owned
            .watcher_runs
            .next_run
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        lock(&self.owned.watcher_runs.live).insert(
            record.watcher_id.clone(),
            LiveRun {
                run,
                revision: record.revision,
                stop,
            },
        );
        let store = self.owned.durable_state_store.clone();
        let runs = Arc::clone(&self.owned.watcher_runs);
        tokio::spawn(async move {
            let watcher_id = record.watcher_id.clone();
            let stream = record.manifest.mode == WatcherMode::Stream;
            let end = run_once(&store, record, stopped).await;
            {
                let mut live = lock(&runs.live);
                if live.get(&watcher_id).is_some_and(|live| live.run == run) {
                    live.remove(&watcher_id);
                }
            }
            if stream {
                let mut backoff = lock(&runs.stream_backoff);
                match end {
                    RunEnd::Stopped => {
                        backoff.remove(&watcher_id);
                    }
                    _ => {
                        let ended = backoff.get(&watcher_id).map_or(0, |(_, ended)| *ended);
                        let wait = (FIRST_BACKOFF_MS << ended.min(16)).min(MAX_BACKOFF_MS);
                        backoff.insert(
                            watcher_id,
                            (crate::session::unix_epoch_ms() + wait, ended + 1),
                        );
                    }
                }
            }
        });
    }
}

/// One run on the schedule. A poll commits once when it exits with 0; a stream commits at each
/// cursor line and fails when it ends.
async fn run_once(
    store: &DurableKernelStateStore,
    record: WatcherRecord,
    stopped: watch::Receiver<bool>,
) -> RunEnd {
    let watcher_id = record.watcher_id.clone();
    let revision = record.revision;
    let fail = |reason: String, output: Option<String>| {
        write_and_tell(
            store,
            WatcherOperation::Failed {
                watcher_id: watcher_id.clone(),
                revision,
                reason,
                output,
                at: crate::session::unix_epoch_ms(),
            },
        )
    };
    let dir = match materialize(store, &record) {
        Ok(dir) => dir,
        Err(reason) => {
            fail(reason.clone(), None);
            return RunEnd::Failed {
                reason,
                output: String::new(),
            };
        }
    };
    let claim = claim(&watcher_id);
    // What an earlier kernel life left of this watcher ends before a new run starts.
    let _ =
        crate::runtime::owned_process_signals::OwnedProcessSignals::end_claimed_tree(&claim, None);
    let stream = record.manifest.mode == WatcherMode::Stream;
    let run_id = random_id();
    let mut pending = RunCommit {
        watcher_id: watcher_id.clone(),
        revision,
        run_id: String::new(),
        reading: None,
        cursor: None,
        events: Vec::new(),
        at: 0,
    };
    let mut commits = 0u64;
    let spec = RunSpec {
        dir,
        manifest: record.manifest,
        input: Input {
            cursor: record.cursor,
            now_ms: crate::session::unix_epoch_ms(),
            dry_run: false,
            since_ms: None,
        },
        claim,
        limit: (!stream).then_some(MAX_RUN),
    };
    let mut commit = |pending: &mut RunCommit| -> Result<(), String> {
        let mut commit = std::mem::replace(
            pending,
            RunCommit {
                watcher_id: watcher_id.clone(),
                revision,
                run_id: String::new(),
                reading: None,
                cursor: None,
                events: Vec::new(),
                at: 0,
            },
        );
        commit.run_id = format!("{run_id}.{commits}");
        commit.at = crate::session::unix_epoch_ms();
        commits += 1;
        tell(store, store.watcher_write(WatcherOperation::Commit(commit)))
            .map_err(|e| e.to_string())
    };
    let end = run::run(spec, stopped, |report| {
        match report {
            Report::Reading(reading) => pending.reading = Some(reading),
            Report::Event(event) => pending.events.push(event),
            Report::History(_) => {}
            Report::Cursor(cursor) => {
                pending.cursor = Some(cursor);
                if stream {
                    return commit(&mut pending);
                }
            }
        }
        Ok(())
    })
    .await;
    match &end {
        RunEnd::Finished if !stream => {
            if let Err(reason) = commit(&mut pending) {
                tracing::debug!(watcher = %watcher_id, %reason, "watcher run was not committed");
            }
        }
        RunEnd::Finished => fail("the stream ended".into(), None),
        RunEnd::Failed { reason, output } => {
            fail(reason.clone(), (!output.is_empty()).then(|| output.clone()))
        }
        RunEnd::Stopped => {}
    }
    end
}

/// The same contract with `dry_run` true and a day of history: nothing is committed or delivered.
async fn dry_run(
    store: &DurableKernelStateStore,
    record: &WatcherRecord,
) -> Result<WatcherDryRun, DaemonError> {
    let dir = materialize(store, record).map_err(error)?;
    let now = crate::session::unix_epoch_ms();
    let mut result = WatcherDryRun {
        reading: None,
        history: true,
        events: Vec::new(),
    };
    let (_stop, stopped) = watch::channel(false);
    let spec = RunSpec {
        dir,
        manifest: record.manifest.clone(),
        input: Input {
            cursor: None,
            now_ms: now,
            dry_run: true,
            since_ms: Some(now.saturating_sub(DRY_RUN_HISTORY_MS)),
        },
        claim: format!("{}:dry", claim(&record.watcher_id)),
        limit: Some(MAX_RUN),
    };
    let end = run::run(spec, stopped, |report| {
        match report {
            Report::Reading(reading) => result.reading = Some(reading),
            Report::Event(event) => result.events.push(event),
            Report::History(history) => result.history = history,
            Report::Cursor(_) => {}
        }
        Ok(())
    })
    .await;
    match end {
        RunEnd::Finished => Ok(result),
        RunEnd::Failed { reason, output } if output.is_empty() => {
            Err(error(format!("the dry run failed: {reason}")))
        }
        RunEnd::Failed { reason, output } => {
            Err(error(format!("the dry run failed: {reason}\n{output}")))
        }
        RunEnd::Stopped => Err(error("the dry run was stopped")),
    }
}

fn write_and_tell(store: &DurableKernelStateStore, operation: WatcherOperation) {
    if let Err(e) = tell(store, store.watcher_write(operation)) {
        tracing::debug!(error = %e, "watcher write was refused");
    }
}

/// A watcher whose health just turned bad wakes the agent responsible for it, once.
fn tell(
    store: &DurableKernelStateStore,
    written: Result<WatcherWritten, DaemonError>,
) -> Result<(), DaemonError> {
    let written = written?;
    let (Some(health), Some(agent)) = (&written.entered, &written.record.creator_agent_id) else {
        return Ok(());
    };
    let record = &written.record;
    let (state, since_ms, message) = match health {
        WatcherHealth::Ok => return Ok(()),
        WatcherHealth::Failing { since_ms, reason } => (
            "failing",
            *since_ms,
            format!(
                "Your watcher {:?} is failing: {reason}. Read chariox.watcher.status, repair it with chariox.watcher.create and its watcher_id, dry-run and activate it again.",
                record.manifest.name
            ),
        ),
        WatcherHealth::Gap { since_ms } => (
            "gap",
            *since_ms,
            format!(
                "Your watcher {:?} has had no good run for {GAP_PERIODS} periods.",
                record.manifest.name
            ),
        ),
        WatcherHealth::Noisy { since_ms } => (
            "noisy",
            *since_ms,
            format!(
                "Your watcher {:?} fired more than {} times in 10 minutes and is slowed to one event a minute. Make its events rarer or keyed.",
                record.manifest.name,
                crate::durable_state::workflow_notifications::watcher::RATE_LIMIT
            ),
        ),
    };
    store.agent_lifecycle(crate::durable_state::agent_lifecycle::Operation::Occur(
        crate::durable_state::agent_lifecycle::occurrence(
            &record.session_id,
            agent,
            &record.source_id,
            &format!("{}:{state}:{since_ms}", record.source_id),
            "watcher_health",
            serde_json::json!({
                "message": message,
                "watcher_id": record.watcher_id,
                "health": health,
            }),
        ),
    ))?;
    Ok(())
}

/// What a caller sees of a watcher it may repair: its summary and the end of its last failure.
pub(in crate::runtime::state) fn watcher_status(record: &WatcherRecord) -> serde_json::Value {
    serde_json::json!({
        "watcher": record.summary(),
        "last_failure_output": record.last_failure_output,
    })
}

pub(in crate::runtime::state) fn manifest_and_script(
    arguments: serde_json::Value,
) -> Result<(Option<String>, WatcherManifest, WatcherScript), DaemonError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Create {
        watcher_id: Option<String>,
        manifest: WatcherManifest,
        script: WatcherScript,
    }
    let create: Create = serde_json::from_value(arguments).map_err(|e| error(e.to_string()))?;
    Ok((create.watcher_id, create.manifest, create.script))
}
