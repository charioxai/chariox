use std::collections::{BTreeSet, HashSet};

use super::*;

pub(crate) struct ProviderProcessList {
    pub(crate) canonical_processes: Vec<crate::provider::ProviderProcessInfo>,
    pub(crate) filtered_processes: Vec<crate::provider::ProviderProcessInfo>,
    pub(crate) delay_ms: u64,
}

pub(crate) struct ProviderProcessTeardown {
    pub(crate) processes: Vec<crate::provider::ProviderProcessInfo>,
    pub(crate) sessions: Vec<crate::session::RuntimeSession>,
    pub(crate) canonical_processes: Vec<crate::provider::ProviderProcessInfo>,
}

impl KernelRuntimeState {
    pub(crate) fn list_provider_processes(&self, provider: Option<&str>) -> ProviderProcessList {
        let canonical_processes = self.provider_process_snapshot();
        let filtered_processes = filter_provider_processes(canonical_processes.clone(), provider);
        let delay_ms = self
            .owned
            .config_projection
            .snapshot()
            .provider_process_list_delay_ms;
        ProviderProcessList {
            canonical_processes,
            filtered_processes,
            delay_ms,
        }
    }

    pub(crate) async fn teardown_provider_processes(
        &self,
        provider: Option<&str>,
        force: bool,
        allowed_process_ids: Option<&HashSet<String>>,
    ) -> Result<ProviderProcessTeardown, DaemonError> {
        let processes = filter_provider_processes(self.provider_process_snapshot(), provider)
            .into_iter()
            .filter(|process| {
                allowed_process_ids.is_none_or(|allowed| allowed.contains(&process.process_id))
            })
            .filter(|process| {
                process.teardown_safe
                    || (force
                        && !process.teardown_blockers.iter().any(|blocker| {
                            blocker == "active prompt"
                                || blocker.starts_with("active workflow runs:")
                        }))
            })
            .collect::<Vec<_>>();
        let session_ids = processes
            .iter()
            .flat_map(|process| process.owner_session_ids.iter())
            .cloned()
            .collect::<HashSet<_>>();

        for process in &processes {
            let run_ids = self
                .owned
                .provider_process_tracking
                .read()
                .processes
                .values()
                .find(|tracked| tracked.process_id == process.process_id)
                .map(|tracked| tracked.owner_provider_run_ids.clone())
                .unwrap_or_else(|| process.owner_provider_run_ids.clone());
            for run_id in run_ids {
                let run = match self.owned.provider_store.get_run(&run_id) {
                    Ok(run) => run,
                    Err(_) => continue,
                };
                if run.state() == crate::provider::ProviderRunState::Ended {
                    continue;
                }
                if let Ok(outcome) = self
                    .owned
                    .provider_store
                    .terminate_run_provider_only(run.session_id(), run.id())
                {
                    self.owned.clear_active_provider_run_session_pointer(
                        run.session_id(),
                        outcome.run().id(),
                    )?;
                    self.owned
                        .provider_run_projection
                        .update(outcome.into_run());
                }
                let remove_run_id = run.id().to_string();
                let process_key = self
                    .with_app_side_effect(move |app| {
                        crate::app::ProviderLaunchProcessRuntime::new(app)
                            .remove_run(&remove_run_id)
                    })
                    .await
                    .ok()
                    .and_then(|(_, process_key)| process_key);
                self.owned
                    .remove_provider_process_tracking_for_run(run.id(), process_key);
            }
        }

        let sessions = session_ids
            .into_iter()
            .filter_map(|session_id| self.owned.session_store.get_session(&session_id).ok())
            .collect::<Vec<_>>();
        Ok(ProviderProcessTeardown {
            processes,
            sessions,
            canonical_processes: self.provider_process_snapshot(),
        })
    }

    fn provider_process_snapshot(&self) -> Vec<crate::provider::ProviderProcessInfo> {
        let mut processes = Vec::new();
        let tracking = self.owned.provider_process_tracking.read();
        for tracked in tracking.processes.values() {
            let runs = tracked
                .owner_provider_run_ids
                .iter()
                .filter_map(|run_id| self.owned.provider_store.get_run(run_id).ok())
                .filter(|run| run.state() != crate::provider::ProviderRunState::Ended)
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
                    !self
                        .owned
                        .attachment_store
                        .list_session_attachment_ids(session_id)
                        .is_empty()
                })
                .cloned()
                .collect::<BTreeSet<_>>();
            let active_workflow_run_ids = owner_session_ids
                .iter()
                .flat_map(|session_id| {
                    self.owned
                        .session_store
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
                self.owned
                    .session_store
                    .get_session(session_id)
                    .ok()
                    .is_some_and(|session| {
                        self.owned
                            .prompt_state_owner
                            .has_any_active_prompt(&session)
                    })
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
            if let Some(mut process) = crate::provider::ProviderProcessInfo::from_runs(
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

fn filter_provider_processes(
    processes: Vec<crate::provider::ProviderProcessInfo>,
    provider: Option<&str>,
) -> Vec<crate::provider::ProviderProcessInfo> {
    let Some(provider) = provider else {
        return processes;
    };
    processes
        .into_iter()
        .filter(|process| process.provider == provider)
        .collect()
}
