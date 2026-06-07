use super::*;

const PUBLICATION_WATCHDOG_STARTUP_GRACE_MS: u64 = 300_000;

impl SessionService {
    pub fn create_workflow_watchdog(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        interval_seconds: u64,
        invocation_prompt: String,
        policy: WorkflowWatchdogPolicy,
        max_wakeups: Option<Option<u64>>,
    ) -> Result<WorkflowWatchdogDefinition, DaemonError> {
        let workflow_id = self
            .resolve_workflow_ref(session_id, workflow_ref)?
            .id()
            .to_string();
        let endpoint_id = self
            .resolve_workflow_endpoint_ref(session_id, workflow_ref, endpoint_ref)?
            .id()
            .to_string();
        let watchdog = WorkflowWatchdogDefinition::new(
            self.next_workflow_watchdog_id(),
            workflow_id,
            endpoint_id,
            interval_seconds,
            invocation_prompt,
            policy,
            max_wakeups.unwrap_or(Some(crate::session::DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS)),
        );
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.add_workflow_watchdog(watchdog))
    }

    pub fn list_workflow_watchdogs(
        &self,
        session_id: &str,
        workflow_ref: Option<&str>,
    ) -> Result<Vec<WorkflowWatchdogDefinition>, DaemonError> {
        let workflow_id = workflow_ref
            .map(|reference| self.resolve_workflow_ref(session_id, reference))
            .transpose()?
            .map(|workflow| workflow.id().to_string());
        let session = self.get_session(session_id)?;
        Ok(session
            .workflow_watchdogs()
            .iter()
            .filter(|watchdog| {
                workflow_id
                    .as_deref()
                    .is_none_or(|id| watchdog.workflow_id() == id)
            })
            .cloned()
            .collect())
    }

    pub fn resolve_workflow_watchdog_ref(
        &self,
        session_id: &str,
        watchdog_ref: &str,
    ) -> Result<WorkflowWatchdogDefinition, DaemonError> {
        let normalized_ref = watchdog_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        if let Some(watchdog) = session
            .workflow_watchdogs()
            .iter()
            .find(|watchdog| watchdog.id() == normalized_ref)
        {
            return Ok(watchdog.clone());
        }
        let matches = session
            .workflow_watchdogs()
            .iter()
            .filter(|watchdog| watchdog.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return Ok(matches[0].clone());
        }
        Err(DaemonError::InvalidWorkflowGraphReference {
            session_id: session_id.to_string(),
            workflow_id: String::new(),
            reference: watchdog_ref.to_string(),
            message: "workflow watchdog was not found",
        })
    }

    pub fn set_workflow_watchdog_enabled(
        &mut self,
        session_id: &str,
        watchdog_ref: &str,
        enabled: bool,
    ) -> Result<WorkflowWatchdogDefinition, DaemonError> {
        let watchdog_id = self
            .resolve_workflow_watchdog_ref(session_id, watchdog_ref)?
            .id()
            .to_string();
        let now = unix_epoch_ms();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let watchdog = session.workflow_watchdog_mut(&watchdog_id).ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: String::new(),
                reference: watchdog_id.clone(),
                message: "workflow watchdog was not found",
            }
        })?;
        watchdog.set_enabled(enabled);
        watchdog.set_last_error(None);
        watchdog.set_last_status(Some(if enabled {
            "enabled".to_string()
        } else {
            "disabled".to_string()
        }));
        if enabled {
            watchdog.set_next_run_at_ms(now.saturating_add(watchdog.interval_seconds() * 1000));
        }
        Ok(watchdog.clone())
    }

    pub fn remove_workflow_watchdog(
        &mut self,
        session_id: &str,
        watchdog_ref: &str,
    ) -> Result<WorkflowWatchdogDefinition, DaemonError> {
        let watchdog_id = self
            .resolve_workflow_watchdog_ref(session_id, watchdog_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        session
            .remove_workflow_watchdog(&watchdog_id)
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: String::new(),
                reference: watchdog_id.clone(),
                message: "workflow watchdog was not found",
            })
    }

    pub fn collect_due_workflow_watchdog_invocations(
        &mut self,
        now_ms: u64,
    ) -> Result<Vec<WorkflowWatchdogTickPlan>, DaemonError> {
        let mut plans = Vec::new();
        let session_ids = self
            .store
            .non_ended_sessions()
            .map(|s| s.id().to_string())
            .collect::<Vec<_>>();
        for session_id in session_ids {
            let queued_prompt_specs = {
                let mut queued_prompt_specs = Vec::new();
                let session = match self.store.get_mut(&session_id) {
                    Some(session) => session,
                    None => continue,
                };
                let active_run_exists = session.workflow_runs().iter().any(|run| {
                    !matches!(
                        run.status(),
                        WorkflowRunStatus::Completed
                            | WorkflowRunStatus::Failed
                            | WorkflowRunStatus::Stopped
                    )
                });
                let session_hidden = session.is_hidden();
                let session_created_at_ms = session.created_at_ms();
                let completed_statuses = session
                    .workflow_runs()
                    .iter()
                    .map(|run| (run.id().to_string(), run.status()))
                    .collect::<BTreeMap<_, _>>();
                for watchdog in session.workflow_watchdogs_mut().iter_mut() {
                    if let Some(run_status) = watchdog
                        .last_workflow_run_id()
                        .and_then(|run_id| completed_statuses.get(run_id).copied())
                    {
                        if matches!(
                            run_status,
                            WorkflowRunStatus::Completed
                                | WorkflowRunStatus::Failed
                                | WorkflowRunStatus::Stopped
                        ) {
                            watchdog.set_last_status(Some(
                                match run_status {
                                    WorkflowRunStatus::Completed => "last_run_completed",
                                    WorkflowRunStatus::Failed => "last_run_failed",
                                    WorkflowRunStatus::Stopped => "last_run_stopped",
                                    _ => "last_run_finished",
                                }
                                .to_string(),
                            ));
                        }
                    }
                    if !watchdog.enabled() {
                        continue;
                    }
                    if watchdog
                        .max_wakeups()
                        .is_some_and(|limit| watchdog.wakeups_executed() >= limit)
                    {
                        watchdog.set_enabled(false);
                        watchdog.set_pending_run(false);
                        watchdog.set_last_status(Some("completed_budget".to_string()));
                        continue;
                    }
                    let should_run_pending = watchdog.pending_run() && !active_run_exists;
                    let due_now = now_ms >= watchdog.next_run_at_ms();
                    if should_run_pending {
                        watchdog.set_pending_run(false);
                        watchdog.set_last_status(Some("invoking_pending".to_string()));
                        plans.push(WorkflowWatchdogTickPlan {
                            watchdog_id: watchdog.id().to_string(),
                            session_id: session_id.clone(),
                            workflow_id: watchdog.workflow_id().to_string(),
                            endpoint_id: watchdog.endpoint_id().to_string(),
                            invocation_prompt: watchdog.invocation_prompt().to_string(),
                        });
                        continue;
                    }
                    if !due_now {
                        continue;
                    }
                    if session_hidden
                        && watchdog.wakeups_executed() == 0
                        && now_ms
                            < session_created_at_ms
                                .saturating_add(PUBLICATION_WATCHDOG_STARTUP_GRACE_MS)
                    {
                        watchdog.set_last_status(Some("warming_up".to_string()));
                        watchdog.set_next_run_at_ms(
                            session_created_at_ms
                                .saturating_add(PUBLICATION_WATCHDOG_STARTUP_GRACE_MS),
                        );
                        continue;
                    }
                    let next_run = now_ms.saturating_add(watchdog.interval_seconds() * 1000);
                    if active_run_exists {
                        match watchdog.policy() {
                            WorkflowWatchdogPolicy::Skip => {
                                watchdog.set_last_status(Some("skipped_running".to_string()));
                                watchdog.set_next_run_at_ms(next_run);
                            }
                            WorkflowWatchdogPolicy::Queue => {
                                if !watchdog.pending_run() {
                                    queued_prompt_specs.push((
                                        watchdog.workflow_id().to_string(),
                                        watchdog.endpoint_id().to_string(),
                                        watchdog.invocation_prompt().to_string(),
                                        watchdog.id().to_string(),
                                    ));
                                }
                                watchdog.set_pending_run(true);
                                watchdog.set_last_status(Some("queued_running".to_string()));
                                watchdog.set_next_run_at_ms(next_run);
                            }
                        }
                        continue;
                    }
                    watchdog.set_last_status(Some("invoking".to_string()));
                    watchdog.set_next_run_at_ms(next_run);
                    plans.push(WorkflowWatchdogTickPlan {
                        watchdog_id: watchdog.id().to_string(),
                        session_id: session_id.clone(),
                        workflow_id: watchdog.workflow_id().to_string(),
                        endpoint_id: watchdog.endpoint_id().to_string(),
                        invocation_prompt: watchdog.invocation_prompt().to_string(),
                    });
                }
                queued_prompt_specs
            };
            for (workflow_id, endpoint_id, invocation_prompt, watchdog_id) in queued_prompt_specs {
                let _ = self.enqueue_workflow_prompt(
                    &session_id,
                    &workflow_id,
                    &endpoint_id,
                    Some(invocation_prompt),
                    None,
                    WorkflowQueuedPromptSource::Watchdog,
                    Some(watchdog_id),
                );
            }
        }
        Ok(plans)
    }

    pub fn mark_workflow_watchdog_invoked(
        &mut self,
        session_id: &str,
        watchdog_id: &str,
        workflow_run_id: &str,
    ) -> Result<WorkflowWatchdogDefinition, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let watchdog = session.workflow_watchdog_mut(watchdog_id).ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: String::new(),
                reference: watchdog_id.to_string(),
                message: "workflow watchdog was not found",
            }
        })?;
        watchdog.set_last_run_at_ms(Some(unix_epoch_ms()));
        watchdog.set_wakeups_executed(watchdog.wakeups_executed().saturating_add(1));
        watchdog.set_pending_run(false);
        if watchdog
            .max_wakeups()
            .is_some_and(|limit| watchdog.wakeups_executed() >= limit)
        {
            watchdog.set_enabled(false);
            watchdog.set_pending_run(false);
            watchdog.set_last_status(Some("completed_budget".to_string()));
        } else {
            watchdog.set_last_status(Some("started".to_string()));
        }
        watchdog.set_last_error(None);
        watchdog.set_last_workflow_run_id(Some(workflow_run_id.to_string()));
        Ok(watchdog.clone())
    }

    pub fn mark_workflow_watchdog_queued(
        &mut self,
        session_id: &str,
        watchdog_id: &str,
    ) -> Result<WorkflowWatchdogDefinition, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let watchdog = session.workflow_watchdog_mut(watchdog_id).ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: String::new(),
                reference: watchdog_id.to_string(),
                message: "workflow watchdog was not found",
            }
        })?;
        watchdog.set_pending_run(true);
        watchdog.set_last_status(Some("queued_running".to_string()));
        watchdog.set_last_error(None);
        Ok(watchdog.clone())
    }

    pub fn mark_workflow_watchdog_pending_started(
        &mut self,
        session_id: &str,
        watchdog_id: &str,
    ) -> Result<WorkflowWatchdogDefinition, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let watchdog = session.workflow_watchdog_mut(watchdog_id).ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: String::new(),
                reference: watchdog_id.to_string(),
                message: "workflow watchdog was not found",
            }
        })?;
        watchdog.set_pending_run(false);
        watchdog.set_last_status(Some("invoking_pending".to_string()));
        Ok(watchdog.clone())
    }

    pub fn mark_workflow_watchdog_failed(
        &mut self,
        session_id: &str,
        watchdog_id: &str,
        error: impl Into<String>,
    ) -> Result<WorkflowWatchdogDefinition, DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let watchdog = session.workflow_watchdog_mut(watchdog_id).ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: String::new(),
                reference: watchdog_id.to_string(),
                message: "workflow watchdog was not found",
            }
        })?;
        watchdog.set_last_status(Some("invoke_failed".to_string()));
        watchdog.set_last_error(Some(error.into()));
        Ok(watchdog.clone())
    }
}
