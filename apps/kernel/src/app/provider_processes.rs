use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::app::{DaemonApp, TrackedProviderProcess};
use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, ProviderProcessInfo, ProviderRunState, RuntimeProviderRun,
};

mod dead_kernel_leftovers;

use super::provider_liveness::{poll_provider_run_process_exit, ProviderProcessExit};

pub(crate) struct ProviderLaunchProcessRuntime<'a> {
    app: &'a mut DaemonApp,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProviderProcessReapSummary {
    pub(crate) tracked_processes_reaped: usize,
    pub(crate) orphan_processes_reaped: usize,
}

struct LaunchedProcess<'a> {
    app: &'a mut DaemonApp,
    run_id: String,
    committed: bool,
}

impl LaunchedProcess<'_> {
    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for LaunchedProcess<'_> {
    fn drop(&mut self) {
        if !self.committed {
            if let Err(error) = ProviderProcessTracker::new(self.app).remove_run(&self.run_id) {
                crate::logging::error_with_fields(
                    "daemon.provider",
                    "uncommitted provider spawn cleanup failed",
                    serde_json::json!({"operation": "spawn_rollback", "provider_run_id": self.run_id, "error": error.to_string()}),
                );
            }
        }
    }
}

impl<'a> ProviderLaunchProcessRuntime<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    #[cfg(test)]
    pub(crate) fn spawn_for_launch(&mut self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        self.spawn_for_launch_with_credentials(
            run,
            &crate::provider::ProviderCredentialEnvironment::default(),
        )
    }

    pub(crate) fn spawn_for_launch_with_credentials(
        &mut self,
        run: &RuntimeProviderRun,
        credentials: &crate::provider::ProviderCredentialEnvironment,
    ) -> Result<(), DaemonError> {
        let store = self.app.providers.clone();
        store
            .with_current_live_launch(run, |current| {
                let launched = LaunchedProcess {
                    app: &mut *self.app,
                    run_id: current.id().to_string(),
                    committed: false,
                };
                Self::spawn_authorized_launch(&mut *launched.app, current, credentials)?;
                Ok(launched)
            })?
            .map(LaunchedProcess::commit)
            .ok_or_else(|| DaemonError::InvalidProviderRunState {
                provider_run_id: run.id().into(),
                state: store
                    .get_run(run.id())
                    .map(|run| run.state())
                    .unwrap_or(ProviderRunState::Ended),
                operation: "spawn current provider launch",
            })
    }

    fn spawn_authorized_launch(
        app: &mut DaemonApp,
        run: &RuntimeProviderRun,
        credentials: &crate::provider::ProviderCredentialEnvironment,
    ) -> Result<(), DaemonError> {
        #[cfg(test)]
        if crate::provider::take_provider_lifecycle_failure_for_test(
            run.id(),
            crate::provider::ProviderLifecycleFailureStage::Spawn,
        ) {
            return Err(DaemonError::PtySpawn {
                provider_run_id: run.id().to_string(),
                message: "injected provider restart spawn failure".to_string(),
            });
        }
        #[cfg(test)]
        if crate::provider::record_provider_credential_delivery_for_test(
            run.id(),
            "pty_spawn",
            credentials,
        ) {
            return Ok(());
        }
        if run.endpoint_mode() != AgentEndpointMode::Managed {
            return Ok(());
        }
        app.pty.spawn_for_run_with_credentials(run, credentials)?;
        ProviderProcessTracker::new(app).register_managed_run(run)
    }

    pub(crate) fn remove_run(
        &mut self,
        provider_run_id: &str,
    ) -> Result<(bool, Option<String>), DaemonError> {
        remove_provider_pty_process(self.app, provider_run_id)
    }

    pub(crate) fn poll_exit(
        &mut self,
        provider_run_id: &str,
    ) -> Result<Option<ProviderProcessExit>, DaemonError> {
        poll_provider_run_process_exit(self.app, provider_run_id)
    }
}

pub(crate) struct ProviderProcessTracker<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderProcessTracker<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn register_managed_run(
        &mut self,
        run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let process_key = self.app.pty.process_key(run.id())?;
        let pid = self.app.pty.process_id(run.id())?;
        let process_id = format!("managed:{}:{}", run.provider(), process_key);
        let identity = pid.and_then(|pid| {
            crate::runtime::kernel_access::process::inspect(pid)
                .ok()
                .map(|(identity, _)| identity)
        });
        let mut tracking = self.app.provider_process_tracking.write();
        let entry = tracking
            .processes
            .entry(process_key.clone())
            .or_insert_with(|| TrackedProviderProcess {
                process_id: process_id.clone(),
                pid,
                identity,
                endpoint_identity: None,
                endpoint_mode: run.endpoint_mode(),
                process_label: run.process_label().to_string(),
                started_at_ms: run.started_at_ms(),
                owner_provider_run_ids: Vec::new(),
            });
        entry.pid = pid.or(entry.pid);
        if !entry.owner_provider_run_ids.iter().any(|id| id == run.id()) {
            entry.owner_provider_run_ids.push(run.id().to_string());
        }
        tracking
            .run_processes
            .insert(run.id().to_string(), process_key.clone());
        drop(tracking);
        crate::logging::info_with_fields(
            "daemon.provider_process_gc",
            "registered managed provider process",
            serde_json::json!({
                "pid": pid,
                "process_key": process_key,
                "provider_run_id": run.id(),
            }),
        );
        Ok(())
    }

    pub(crate) fn bind_endpoint_identity(
        tracking_store: &crate::app::ProviderProcessTrackingStore,
        run: &RuntimeProviderRun,
    ) {
        let mut tracking = tracking_store.write();
        let Some(key) = tracking.run_processes.get(run.id()).cloned() else {
            return;
        };
        let Some(process) = tracking.processes.get_mut(&key) else {
            return;
        };
        if process.endpoint_identity.is_none() {
            process.endpoint_identity = process
                .identity
                .as_ref()
                .zip(run.structured_endpoint())
                .and_then(|(launcher, endpoint)| {
                    super::provider_endpoint_identity::capture(launcher, endpoint)
                });
        }
    }

    pub(crate) fn remove_run(&mut self, provider_run_id: &str) -> Result<bool, DaemonError> {
        let tracked_process_key = self
            .app
            .provider_process_tracking
            .read()
            .run_processes
            .get(provider_run_id)
            .cloned();
        let (removed, process_key) = remove_provider_pty_process(self.app, provider_run_id)?;
        let process_key = tracked_process_key.or(process_key);
        let Some(process_key) = process_key else {
            return Ok(removed);
        };
        let mut tracking = self.app.provider_process_tracking.write();
        tracking.run_processes.remove(provider_run_id);
        let should_remove_entry = if let Some(entry) = tracking.processes.get_mut(&process_key) {
            entry
                .owner_provider_run_ids
                .retain(|id| id != provider_run_id);
            entry.owner_provider_run_ids.is_empty()
        } else {
            false
        };
        if should_remove_entry {
            tracking.processes.remove(&process_key);
        }
        Ok(removed)
    }

    pub(crate) fn list(
        app: &DaemonApp,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        let processes = Self::snapshot(app);
        app.update_provider_process_projection(processes.clone());
        Ok(filter_provider_processes(processes, provider))
    }

    pub(crate) fn teardown_safe_processes(
        &mut self,
        provider: Option<&str>,
        force: bool,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        let safe_processes = Self::list(self.app, provider)?
            .into_iter()
            .filter(|process| {
                process.teardown_safe
                    || (force
                        && !process.teardown_blockers.iter().any(|blocker| {
                            blocker == "active prompt"
                                || blocker.starts_with("active workflow runs:")
                        }))
            })
            .collect::<Vec<_>>();
        for process in &safe_processes {
            let run_ids: Vec<String> = self
                .app
                .provider_process_tracking
                .read()
                .processes
                .values()
                .find(|tracked| tracked.process_id == process.process_id)
                .map(|tracked| tracked.owner_provider_run_ids.clone())
                .unwrap_or_else(|| process.owner_provider_run_ids.clone());
            for run_id in run_ids {
                let run = match self.app.providers.get_run(&run_id) {
                    Ok(run) => run,
                    Err(_) => continue,
                };
                if run.state() == ProviderRunState::Ended {
                    continue;
                }
                if let Ok(outcome) = self
                    .app
                    .providers
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    clear_active_provider_run_session_pointer(
                        self.app,
                        run.session_id(),
                        outcome.run().id(),
                    )?;
                    self.app.update_provider_run_projection(outcome.into_run());
                }
                let _ = self.remove_run(run.id());
            }
        }
        self.app
            .update_provider_process_projection(Self::snapshot(self.app));
        Ok(safe_processes)
    }

    pub(crate) fn reap_idle_processes(
        &mut self,
        now_ms: u64,
        idle_ttl_ms: u64,
    ) -> Result<ProviderProcessReapSummary, DaemonError> {
        let mut summary = ProviderProcessReapSummary::default();
        let safe_processes = Self::snapshot(self.app)
            .into_iter()
            .filter(|process| process.teardown_safe)
            .filter(|process| now_ms.saturating_sub(process.last_activity_at_ms) >= idle_ttl_ms)
            .collect::<Vec<_>>();

        for process in safe_processes {
            self.reap_tracked_process(&process)?;
            summary.tracked_processes_reaped += 1;
        }

        summary.orphan_processes_reaped = stop_processes_left_by_dead_kernels();

        self.app
            .update_provider_process_projection(Self::snapshot(self.app));
        Ok(summary)
    }

    pub(super) fn reap_tracked_process(
        &mut self,
        process: &ProviderProcessInfo,
    ) -> Result<(), DaemonError> {
        let (process_key, run_ids, pid) = {
            let tracking = self.app.provider_process_tracking.read();
            let Some((process_key, tracked)) = tracking
                .processes
                .iter()
                .find(|(_, tracked)| tracked.process_id == process.process_id)
            else {
                return Ok(());
            };
            (
                process_key.clone(),
                tracked.owner_provider_run_ids.clone(),
                tracked.pid,
            )
        };

        let live_runs = run_ids
            .iter()
            .filter_map(|run_id| self.app.providers.get_run(run_id).ok())
            .filter(|run| run.state() != ProviderRunState::Ended)
            .collect::<Vec<_>>();
        let sessions = live_runs
            .iter()
            .filter_map(|run| self.app.sessions.get_session(run.session_id()).ok())
            .collect::<Vec<_>>();
        // The process looked idle when the sweep began; a prompt given to one
        // of its runs since then keeps it.
        let providers = &self.app.providers;
        let Some(ended) = self
            .app
            .prompt_state_owner
            .while_sessions_idle(&sessions, || {
                Ok(live_runs
                    .iter()
                    .filter_map(|run| {
                        providers
                            .terminate_run_provider_only(run.session_id(), run.id())
                            .ok()
                    })
                    .collect::<Vec<_>>())
            })?
        else {
            crate::logging::info_with_fields(
                "daemon.provider_process_gc",
                "kept idle managed provider process: a prompt became active",
                serde_json::json!({
                    "process_id": process.process_id,
                    "owner_provider_run_ids": run_ids,
                }),
            );
            return Ok(());
        };
        for outcome in ended {
            clear_active_provider_run_session_pointer(
                self.app,
                outcome.run().session_id(),
                outcome.run().id(),
            )?;
            self.app.update_provider_run_projection(outcome.into_run());
        }
        for run_id in &run_ids {
            let _ = self.remove_run(run_id);
        }

        let removed_by_key = self.app.pty.remove_process_by_key(&process_key, None)?;
        if !removed_by_key {
            if let Some(pid) = pid {
                let _ = crate::runtime::process_health::terminate_process_tree(pid);
            }
            self.app
                .provider_process_tracking
                .write()
                .processes
                .remove(&process_key);
        }

        crate::logging::warn_with_fields(
            "daemon.provider_process_gc",
            "reaped idle managed provider process",
            serde_json::json!({
                "process_id": process.process_id,
                "pid": pid,
                "owner_provider_run_ids": run_ids,
            }),
        );

        Ok(())
    }

    fn snapshot(app: &DaemonApp) -> Vec<ProviderProcessInfo> {
        let mut processes = Vec::new();
        let tracking = app.provider_process_tracking.read();
        for tracked in tracking.processes.values() {
            let runs = tracked
                .owner_provider_run_ids
                .iter()
                .filter_map(|run_id| app.providers.get_run(run_id).ok())
                .filter(|run| run.state() != ProviderRunState::Ended)
                .collect::<Vec<_>>();
            if runs.is_empty() {
                continue;
            }
            let owner_session_ids = runs
                .iter()
                .map(|run| run.session_id().to_string())
                .collect::<BTreeSet<_>>();
            let attached_session_ids = owner_session_ids
                .iter()
                .filter(|session_id| {
                    !app.attachments
                        .list_session_attachment_ids(session_id)
                        .is_empty()
                })
                .cloned()
                .collect::<BTreeSet<_>>();
            let active_workflow_run_ids = owner_session_ids
                .iter()
                .flat_map(|session_id| {
                    app.sessions
                        .get_session(session_id)
                        .ok()
                        .map(|session| session.workflow_runs().to_vec())
                        .into_iter()
                        .flatten()
                        .filter(|run| {
                            !matches!(
                                run.status(),
                                crate::session::WorkflowRunStatus::Completed
                                    | crate::session::WorkflowRunStatus::Failed
                                    | crate::session::WorkflowRunStatus::Stopped
                            )
                        })
                        .map(|run| run.id().to_string())
                })
                .collect::<BTreeSet<_>>();
            let has_active_prompt = owner_session_ids.iter().any(|session_id| {
                app.sessions
                    .get_session(session_id)
                    .ok()
                    .is_some_and(|session| app.prompt_state_owner.has_any_active_prompt(&session))
            });
            let mut teardown_blockers = Vec::new();
            if !attached_session_ids.is_empty() {
                teardown_blockers.push(format!(
                    "attached sessions: {}",
                    attached_session_ids
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            if has_active_prompt {
                teardown_blockers.push("active prompt".to_string());
            }
            if !active_workflow_run_ids.is_empty() {
                teardown_blockers.push(format!(
                    "active workflow runs: {}",
                    active_workflow_run_ids
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            let teardown_safe = attached_session_ids.is_empty()
                && active_workflow_run_ids.is_empty()
                && !has_active_prompt;
            if let Some(mut process) = ProviderProcessInfo::from_runs(
                tracked.process_id.clone(),
                &runs,
                attached_session_ids,
                active_workflow_run_ids,
                teardown_safe,
                teardown_blockers,
            ) {
                process.pid = tracked.pid;
                process.resident_set_bytes = tracked
                    .pid
                    .and_then(crate::runtime::process_health::resident_set_bytes_for_pid);
                process.process_label = tracked.process_label.clone();
                process.endpoint_mode = tracked.endpoint_mode;
                process.started_at_ms = tracked.started_at_ms;
                processes.push(process);
            }
        }
        processes
    }
}

fn remove_provider_pty_process(
    app: &mut DaemonApp,
    provider_run_id: &str,
) -> Result<(bool, Option<String>), DaemonError> {
    let process_key = app.pty.process_key(provider_run_id).ok();
    let cleanup_root = app
        .providers
        .get_run(provider_run_id)
        .ok()
        .and_then(|run| claude_native_tui_cleanup_root(&run));
    let removed = app.pty.remove_process(provider_run_id)?;
    let process_still_running = process_key
        .as_deref()
        .is_some_and(|key| app.pty.has_process_key(key));
    if !process_still_running {
        if let Some(root) = cleanup_root {
            let _ = std::fs::remove_dir_all(root);
        }
    }
    Ok((removed, process_key))
}

fn claude_native_tui_cleanup_root(run: &RuntimeProviderRun) -> Option<PathBuf> {
    if run.endpoint_mode() != AgentEndpointMode::Managed
        || !run.process_label().starts_with("claude:")
    {
        return None;
    }
    let events_file = PathBuf::from(run.pty_env().get("CHARIOX_CLAUDE_NATIVE_EVENTS")?);
    let root = events_file.parent()?;
    if root.parent() != Some(std::env::temp_dir().as_path())
        || !root
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("chariox-claude-remote-native-"))
    {
        return None;
    }
    for key in [
        "CHARIOX_CLAUDE_NATIVE_CONTEXT",
        "CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES",
        "CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES",
        "CHARIOX_CLAUDE_USAGE_FILE",
    ] {
        if Path::new(run.pty_env().get(key)?).parent() != Some(root) {
            return None;
        }
    }
    Some(root.to_path_buf())
}

/// End the previous kernel's abandoned provider work before resuming interrupted turns.
pub(crate) fn stop_processes_left_by_dead_kernels() -> usize {
    dead_kernel_leftovers::stop()
}

fn clear_active_provider_run_session_pointer(
    app: &mut DaemonApp,
    session_id: &str,
    provider_run_id: &str,
) -> Result<(), DaemonError> {
    if app
        .sessions
        .get_session(session_id)?
        .active_provider_run_id()
        == Some(provider_run_id)
    {
        app.sessions.set_active_provider_run(session_id, None)?;
    }
    Ok(())
}

fn filter_provider_processes(
    processes: Vec<ProviderProcessInfo>,
    provider: Option<&str>,
) -> Vec<ProviderProcessInfo> {
    let Some(provider) = provider else {
        return processes;
    };
    processes
        .into_iter()
        .filter(|process| process.provider == provider)
        .collect()
}

impl DaemonApp {
    pub fn list_provider_processes(
        &self,
        provider: Option<&str>,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        ProviderProcessTracker::list(self, provider)
    }

    pub fn teardown_provider_processes(
        &mut self,
        provider: Option<&str>,
        force: bool,
    ) -> Result<Vec<ProviderProcessInfo>, DaemonError> {
        ProviderProcessTracker::new(self).teardown_safe_processes(provider, force)
    }

    pub(crate) fn reap_idle_provider_processes(
        &mut self,
        now_ms: u64,
        idle_ttl_ms: u64,
    ) -> Result<ProviderProcessReapSummary, DaemonError> {
        ProviderProcessTracker::new(self).reap_idle_processes(now_ms, idle_ttl_ms)
    }
}
