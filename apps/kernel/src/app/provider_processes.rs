use std::collections::BTreeSet;

use crate::app::{DaemonApp, TrackedProviderProcess};
use crate::error::DaemonError;
use crate::provider::{
    AgentEndpointMode, ProviderProcessInfo, ProviderRunState, RuntimeProviderRun,
};

use super::provider_liveness::poll_provider_run_process_running;

pub(crate) struct ProviderLaunchProcessRuntime<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderLaunchProcessRuntime<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn spawn_for_launch(&mut self, run: &RuntimeProviderRun) -> Result<(), DaemonError> {
        if run.endpoint_mode() != AgentEndpointMode::Managed {
            return Ok(());
        }
        self.app.pty.spawn_for_run(run)?;
        ProviderProcessTracker::new(self.app).register_managed_run(run)
    }

    pub(crate) fn remove_run(
        &mut self,
        provider_run_id: &str,
    ) -> Result<(bool, Option<String>), DaemonError> {
        let process_key = self.app.pty.process_key(provider_run_id).ok();
        let removed = self.app.pty.remove_process(provider_run_id)?;
        Ok((removed, process_key))
    }

    pub(crate) fn poll_running(&mut self, provider_run_id: &str) -> Result<bool, DaemonError> {
        poll_provider_run_process_running(self.app, provider_run_id)
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
        let mut tracking = self.app.provider_process_tracking.write();
        let entry = tracking
            .processes
            .entry(process_key.clone())
            .or_insert_with(|| TrackedProviderProcess {
                process_id: process_id.clone(),
                pid,
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
            .insert(run.id().to_string(), process_key);
        Ok(())
    }

    pub(crate) fn remove_run(&mut self, provider_run_id: &str) -> Result<bool, DaemonError> {
        let process_key = self
            .app
            .provider_process_tracking
            .read()
            .run_processes
            .get(provider_run_id)
            .cloned()
            .or_else(|| self.app.pty.process_key(provider_run_id).ok());
        let removed = self.app.pty.remove_process(provider_run_id)?;
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
                        .map(|session| session.workflow_runs().iter().cloned().collect::<Vec<_>>())
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
                    .and_then(|session| session.active_prompt().cloned())
                    .is_some()
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
                process.process_label = tracked.process_label.clone();
                process.endpoint_mode = tracked.endpoint_mode;
                process.started_at_ms = tracked.started_at_ms;
                processes.push(process);
            }
        }
        processes
    }
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
}
