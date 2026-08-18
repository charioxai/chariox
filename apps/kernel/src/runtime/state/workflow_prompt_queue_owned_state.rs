//! Workflow prompt queue advancement.

use super::*;

const LIVE_WORKFLOW_ORPHAN_GRACE_PERIOD_MS: u64 = 5_000;
const EVENT_PROVIDER_RETRY_DELAYS_MS: [u64; 3] = [30 * 60_000, 2 * 60 * 60_000, 8 * 60 * 60_000];

#[derive(Clone)]
struct WorkflowEventRetryPlan {
    session_id: String,
    source_run_id: String,
    source_created_at_ms: u64,
    due_at_ms: u64,
    workflow_id: String,
    endpoint_id: String,
    prompt: Option<String>,
    queue_ref: Option<String>,
    invocation: crate::session::WorkflowPublicationInvocationEnvelope,
}

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_collect_due_event_retry_dispatches(
        &self,
        now_ms: u64,
    ) -> WorkflowPromptDispatches {
        let mut plans = Vec::new();
        for session in self.session_store.read().list_sessions() {
            let mut runs_by_invocation =
                BTreeMap::<String, Vec<&crate::session::WorkflowRun>>::new();
            for run in session.workflow_runs() {
                let Some(invocation) = run
                    .publication_invocation()
                    .filter(|invocation| invocation.transport == "event")
                else {
                    continue;
                };
                runs_by_invocation
                    .entry(invocation.invocation_id.clone())
                    .or_default()
                    .push(run);
            }
            for runs in runs_by_invocation.values_mut() {
                runs.sort_by_key(|run| run.created_at_ms());
                let Some(latest) = runs.last().copied() else {
                    continue;
                };
                let Some(delay_ms) =
                    EVENT_PROVIDER_RETRY_DELAYS_MS.get(runs.len().saturating_sub(1))
                else {
                    continue;
                };
                if latest.status() != crate::session::WorkflowRunStatus::Failed {
                    continue;
                }
                let Some(resource_failure) = latest.failure_events().iter().rev().find(|event| {
                    event.kind() == crate::session::WorkflowFailureKind::ProviderFailure
                        && crate::provider::provider_text_reports_resource_limit(event.message())
                }) else {
                    continue;
                };
                if resource_failure.timestamp_ms().saturating_add(*delay_ms) > now_ms {
                    continue;
                }
                let Some(invocation) = latest.publication_invocation().cloned() else {
                    continue;
                };
                if !workflow_event_retry_binding_active(&session, &invocation) {
                    continue;
                }
                let already_queued = session.workflow_queued_prompts().iter().any(|prompt| {
                    prompt
                        .publication_invocation()
                        .is_some_and(|queued| queued.invocation_id == invocation.invocation_id)
                });
                if already_queued {
                    continue;
                }
                plans.push(WorkflowEventRetryPlan {
                    session_id: session.id().to_string(),
                    source_run_id: latest.id().to_string(),
                    source_created_at_ms: latest.created_at_ms(),
                    due_at_ms: resource_failure.timestamp_ms().saturating_add(*delay_ms),
                    workflow_id: latest.workflow_id().to_string(),
                    endpoint_id: latest.endpoint_id().to_string(),
                    prompt: latest.invocation_prompt().map(str::to_string),
                    queue_ref: latest.queue_ref().map(str::to_string),
                    invocation,
                });
            }
        }
        plans.sort_by(|left, right| {
            left.due_at_ms
                .cmp(&right.due_at_ms)
                .then_with(|| left.source_created_at_ms.cmp(&right.source_created_at_ms))
                .then_with(|| {
                    left.invocation
                        .invocation_id
                        .cmp(&right.invocation.invocation_id)
                })
        });

        let mut dispatches = WorkflowPromptDispatches::default();
        for plan in plans {
            let queued = {
                let mut sessions = self.session_store.write();
                let current = match sessions.get_session(&plan.session_id) {
                    Ok(session) => session,
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "daemon.event_delivery",
                            "failed to inspect event provider retry",
                            serde_json::json!({
                                "session_id": plan.session_id,
                                "delivery_id": plan.invocation.invocation_id,
                                "error": error.to_string(),
                            }),
                        );
                        continue;
                    }
                };
                let still_latest = current
                    .workflow_runs()
                    .iter()
                    .rev()
                    .find(|run| {
                        run.publication_invocation().is_some_and(|invocation| {
                            invocation.invocation_id == plan.invocation.invocation_id
                        })
                    })
                    .is_some_and(|run| run.id() == plan.source_run_id);
                let already_queued = current.workflow_queued_prompts().iter().any(|prompt| {
                    prompt.publication_invocation().is_some_and(|invocation| {
                        invocation.invocation_id == plan.invocation.invocation_id
                    })
                });
                if !still_latest
                    || already_queued
                    || !workflow_event_retry_binding_active(&current, &plan.invocation)
                {
                    continue;
                }
                sessions.enqueue_workflow_prompt_with_publication_invocation(
                    &plan.session_id,
                    &plan.workflow_id,
                    &plan.endpoint_id,
                    plan.prompt,
                    plan.queue_ref.as_deref(),
                    crate::session::WorkflowQueuedPromptSource::Event,
                    None,
                    Some(plan.invocation.clone()),
                )
            };
            match queued {
                Ok(queued) => {
                    self.record_notice(
                        &plan.session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(&plan.session_id),
                        format!(
                            "Retrying event delivery `{}` after provider resource exhaustion.",
                            plan.invocation.invocation_id
                        ),
                    );
                    if let Err(error) = self.persist_workflow_runtime_session(
                        &plan.session_id,
                        "workflow_event_provider_retry_queued",
                    ) {
                        crate::logging::warn_with_fields(
                            "daemon.event_delivery",
                            "failed to persist queued event provider retry",
                            serde_json::json!({
                                "session_id": plan.session_id,
                                "queued_prompt_id": queued.id(),
                                "delivery_id": plan.invocation.invocation_id,
                                "error": error.to_string(),
                            }),
                        );
                        let _ = self
                            .session_store
                            .write()
                            .remove_queued_workflow_prompt(&plan.session_id, queued.id());
                        continue;
                    }
                    dispatches
                        .extend(self.workflow_maybe_start_next_queued_prompt(&plan.session_id));
                    if let Err(error) = self.persist_workflow_runtime_session(
                        &plan.session_id,
                        "workflow_event_provider_retry_started",
                    ) {
                        crate::logging::warn_with_fields(
                            "daemon.event_delivery",
                            "failed to persist started event provider retry",
                            serde_json::json!({
                                "session_id": plan.session_id,
                                "delivery_id": plan.invocation.invocation_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                }
                Err(error) => crate::logging::warn_with_fields(
                    "daemon.event_delivery",
                    "failed to queue event provider retry",
                    serde_json::json!({
                        "session_id": plan.session_id,
                        "delivery_id": plan.invocation.invocation_id,
                        "error": error.to_string(),
                    }),
                ),
            }
        }
        dispatches
    }

    fn workflow_reconcile_live_orphans(&self, session_id: &str) {
        let reconciled = self
            .session_store
            .write()
            .reconcile_live_orphaned_workflow_runs(
                session_id,
                crate::session::unix_epoch_ms(),
                LIVE_WORKFLOW_ORPHAN_GRACE_PERIOD_MS,
            );
        let count = match reconciled {
            Ok(count) => count,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime",
                    "live workflow orphan reconciliation failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        if count == 0 {
            return;
        }
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!("Stopped {count} orphaned workflow run(s) so queued prompts can advance."),
        );
        if let Err(error) =
            self.persist_workflow_runtime_session(session_id, "workflow_live_orphan_reconciled")
        {
            crate::logging::warn_with_fields(
                "daemon.runtime",
                "live workflow orphan reconciliation persistence failed",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(super) fn workflow_collect_due_watchdog_dispatches(
        &self,
        now_ms: u64,
    ) -> WorkflowPromptDispatches {
        let collection = match self
            .session_store
            .write()
            .collect_due_workflow_watchdog_invocations_with_changes(now_ms)
        {
            Ok(collection) => collection,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime",
                    "workflow watchdog collection failed",
                    serde_json::json!({ "error": error.to_string() }),
                );
                return WorkflowPromptDispatches::default();
            }
        };

        let mut changed_session_ids = collection.changed_session_ids;
        let mut dispatches = WorkflowPromptDispatches::default();
        for plan in collection.plans {
            let session_id = plan.session_id.clone();
            let watchdog_id = plan.watchdog_id.clone();
            changed_session_ids.insert(session_id.clone());
            match self.workflow_start_watchdog_tick_plan(plan) {
                Ok(next_dispatches) => dispatches.extend(next_dispatches),
                Err(error) => {
                    self.mark_workflow_watchdog_launch_failed(&session_id, &watchdog_id, &error);
                    self.record_notice(
                        &session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(&session_id),
                        format!("Workflow watchdog `{watchdog_id}` failed to launch: {error}"),
                    );
                }
            }
        }
        self.persist_workflow_watchdog_sessions(changed_session_ids);
        dispatches
    }

    fn workflow_start_watchdog_tick_plan(
        &self,
        plan: crate::session::WorkflowWatchdogTickPlan,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        self.workflow_reconcile_live_orphans(&plan.session_id);
        let should_start = self
            .session_store
            .read()
            .workflow_watchdog_can_start(&plan.session_id, &plan.watchdog_id)?;
        if !should_start {
            return Ok(WorkflowPromptDispatches::default());
        }
        if plan.enqueue_prompt {
            self.session_store.write().enqueue_workflow_prompt(
                &plan.session_id,
                &plan.workflow_id,
                &plan.endpoint_id,
                Some(plan.invocation_prompt.clone()),
                plan.queue_id.as_deref(),
                crate::session::WorkflowQueuedPromptSource::Scheduled,
                Some(plan.watchdog_id.clone()),
            )?;
        }
        if self
            .session_store
            .read()
            .session_has_active_workflow_run(&plan.session_id)?
        {
            let _ = self
                .session_store
                .write()
                .mark_workflow_watchdog_queued(&plan.session_id, &plan.watchdog_id);
            return Ok(WorkflowPromptDispatches::default());
        }
        let outcome = self.workflow_start_next_queued_prompt_for_response(&plan.session_id)?;
        if self
            .session_store
            .read()
            .has_queued_workflow_prompt_for_watchdog(&plan.session_id, &plan.watchdog_id)?
        {
            let _ = self
                .session_store
                .write()
                .mark_workflow_watchdog_queued(&plan.session_id, &plan.watchdog_id);
        }
        Ok(outcome
            .map(|(_, dispatches)| dispatches)
            .unwrap_or_default())
    }

    pub(super) fn workflow_enqueue_prompt_and_maybe_start(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
        queue_ref: Option<&str>,
        publication_invocation: Option<crate::session::WorkflowPublicationInvocationEnvelope>,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        self.workflow_reconcile_live_orphans(session_id);
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            workflow.id(),
            endpoint_ref,
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        let (queued_prompt, claimed_run) = self
            .session_store
            .write()
            .enqueue_workflow_prompt_and_maybe_create_run(
                session_id,
                workflow.id(),
                endpoint.id(),
                prompt,
                queue_ref,
                crate::session::WorkflowQueuedPromptSource::Manual,
                None,
                publication_invocation,
            )?;
        let Some((claimed_prompt, workflow_run, claimed_workflow, claimed_endpoint)) = claimed_run
        else {
            return Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued {
                    queued_prompt: Box::new(queued_prompt),
                    workflow,
                    endpoint,
                },
                WorkflowPromptDispatches::default(),
            ));
        };
        let claimed_requested_prompt = claimed_prompt.id() == queued_prompt.id();
        let (claimed_outcome, dispatches) = match self.workflow_schedule_queued_prompt_run(
            session_id,
            claimed_prompt.clone(),
            workflow_run,
            claimed_workflow,
            claimed_endpoint,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.record_queued_workflow_prompt_launch_failure(
                    session_id,
                    &claimed_prompt,
                    &error,
                );
                return Err(error);
            }
        };
        if claimed_requested_prompt {
            Ok((claimed_outcome, dispatches))
        } else {
            Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued {
                    queued_prompt: Box::new(queued_prompt),
                    workflow,
                    endpoint,
                },
                dispatches,
            ))
        }
    }

    fn workflow_schedule_queued_prompt_run(
        &self,
        session_id: &str,
        queued_prompt: crate::session::WorkflowQueuedPrompt,
        workflow_run: crate::session::WorkflowRun,
        workflow: crate::session::WorkflowDefinition,
        endpoint: crate::session::WorkflowEndpointDefinition,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        if let Some(watchdog_id) = queued_prompt.watchdog_id() {
            let _ = self.session_store.write().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        let dispatches = match self
            .workflow_validate_agents(session_id, &workflow)
            .and_then(|()| self.workflow_schedule_entry_node(session_id, &workflow_run))
        {
            Ok(dispatches) => dispatches,
            Err(error) => {
                if let Some(node_run) = workflow_run.node_runs().first() {
                    let _ = self.session_store.write().record_workflow_failure_event(
                        session_id,
                        workflow_run.id(),
                        crate::session::WorkflowFailureEvent::new(
                            crate::session::WorkflowFailureKind::TransportFailure,
                            node_run.id(),
                            Vec::new(),
                            error.to_string(),
                        ),
                    );
                    let _ = self.session_store.write().fail_workflow_node_run(
                        session_id,
                        workflow_run.id(),
                        node_run.id(),
                    );
                }
                return Err(error);
            }
        };
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((
            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                workflow_run: Box::new(workflow_run),
                workflow,
                endpoint,
            },
            dispatches,
        ))
    }

    pub(super) fn workflow_start_next_queued_prompt_for_response(
        &self,
        session_id: &str,
    ) -> Result<
        Option<(
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        )>,
        DaemonError,
    > {
        self.workflow_reconcile_live_orphans(session_id);
        loop {
            let Some((queued_prompt, workflow_run, workflow, endpoint)) = self
                .session_store
                .write()
                .dequeue_next_workflow_prompt_and_create_run(session_id)?
            else {
                return Ok(None);
            };
            match self.workflow_schedule_queued_prompt_run(
                session_id,
                queued_prompt.clone(),
                workflow_run,
                workflow,
                endpoint,
            ) {
                Ok(outcome) => return Ok(Some(outcome)),
                Err(error) => {
                    self.record_queued_workflow_prompt_launch_failure(
                        session_id,
                        &queued_prompt,
                        &error,
                    );
                }
            }
        }
    }

    pub(super) fn workflow_maybe_start_next_queued_prompt(
        &self,
        session_id: &str,
    ) -> WorkflowPromptDispatches {
        self.workflow_reconcile_live_orphans(session_id);
        loop {
            let next_workflow = {
                self.session_store
                    .write()
                    .dequeue_next_workflow_prompt_and_create_run(session_id)
            };
            let (queued_prompt, workflow_run, workflow, endpoint) = match next_workflow {
                Ok(Some(claimed)) => claimed,
                Ok(None) => {
                    let queued_metaagent_is_busy = self
                        .session_store
                        .get_session(session_id)
                        .ok()
                        .and_then(|session| {
                            let metaagent_id = session
                                .queued_metaagent_tasks()
                                .front()
                                .map(|task| task.metaagent_id().to_string())?;
                            let (active_prompt, queued_prompts) =
                                self.prompt_state_owner.state_parts(&session, &metaagent_id);
                            Some(active_prompt.is_some() || !queued_prompts.is_empty())
                        })
                        .unwrap_or(false);
                    if queued_metaagent_is_busy {
                        return WorkflowPromptDispatches::default();
                    }
                    let queued_metaagent_task = self
                        .session_store
                        .write()
                        .pop_next_queued_metaagent_task(session_id);
                    return match queued_metaagent_task {
                        Ok(Some(task)) => WorkflowPromptDispatches {
                            starting_metaagent_tasks: vec![task],
                            ..WorkflowPromptDispatches::default()
                        },
                        Ok(None) => WorkflowPromptDispatches::default(),
                        Err(error) => {
                            self.record_notice(
                                session_id,
                                None,
                                self.attachment_store
                                    .list_session_attachment_ids(session_id),
                                format!("Failed to start queued Meta task: {error}"),
                            );
                            WorkflowPromptDispatches::default()
                        }
                    };
                }
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!("Failed to start queued workflow prompt: {error}"),
                    );
                    return WorkflowPromptDispatches::default();
                }
            };
            match self.workflow_schedule_queued_prompt_run(
                session_id,
                queued_prompt.clone(),
                workflow_run,
                workflow,
                endpoint,
            ) {
                Ok((
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                        workflow_run,
                        workflow,
                        endpoint,
                    },
                    dispatches,
                )) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Started queued workflow run `{}` for workflow `{}` endpoint `{}`.",
                            workflow_run.id(),
                            workflow.id(),
                            endpoint.id()
                        ),
                    );
                    return dispatches;
                }
                Ok((crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { .. }, _)) => {}
                Err(error) => {
                    self.record_queued_workflow_prompt_launch_failure(
                        session_id,
                        &queued_prompt,
                        &error,
                    );
                }
            }
        }
    }

    fn record_queued_workflow_prompt_launch_failure(
        &self,
        session_id: &str,
        queued_prompt: &crate::session::WorkflowQueuedPrompt,
        error: &DaemonError,
    ) {
        crate::logging::warn_with_fields(
            "daemon.runtime",
            "queued workflow prompt launch failed",
            serde_json::json!({
                "session_id": session_id,
                "queued_prompt_id": queued_prompt.id(),
                "workflow_id": queued_prompt.workflow_id(),
                "endpoint_id": queued_prompt.endpoint_id(),
                "error": error.to_string(),
            }),
        );
        if let Some(watchdog_id) = queued_prompt.watchdog_id() {
            self.mark_workflow_watchdog_launch_failed(session_id, watchdog_id, error);
        }
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!(
                "Queued workflow prompt `{}` failed: {error}",
                queued_prompt.id()
            ),
        );
    }

    fn mark_workflow_watchdog_launch_failed(
        &self,
        session_id: &str,
        watchdog_id: &str,
        error: &DaemonError,
    ) {
        let mut sessions = self.session_store.write();
        if workflow_watchdog_failure_is_terminal(error) {
            let _ = sessions.mark_workflow_watchdog_failed_and_disable(
                session_id,
                watchdog_id,
                error.to_string(),
            );
        } else {
            let _ =
                sessions.mark_workflow_watchdog_failed(session_id, watchdog_id, error.to_string());
        }
    }

    fn persist_workflow_watchdog_sessions(&self, session_ids: BTreeSet<String>) {
        for session_id in session_ids {
            let session = match self.session_snapshot(&session_id) {
                Ok(session) => session,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.runtime",
                        "workflow schedule session projection failed",
                        serde_json::json!({
                            "session_id": session_id,
                            "error": error.to_string(),
                        }),
                    );
                    continue;
                }
            };
            if let Err(error) = self.durable_state_store.append_event(
                "session.updated",
                Some(session_id.clone()),
                serde_json::json!({
                    "session": &session,
                    "reason": "workflow_schedule_tick",
                }),
            ) {
                crate::logging::warn_with_fields(
                    "daemon.runtime",
                    "workflow schedule session persistence failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }
}

fn workflow_event_retry_binding_active(
    session: &crate::session::RuntimeSession,
    invocation: &crate::session::WorkflowPublicationInvocationEnvelope,
) -> bool {
    let Some(binding) = invocation
        .hook_id
        .as_deref()
        .and_then(|binding_id| session.workflow_event_binding(binding_id))
    else {
        return false;
    };
    binding.active()
        && binding.publication_id == invocation.publication_id
        && binding.endpoint_id == invocation.endpoint_id
        && session.workflow_publications().iter().any(|publication| {
            publication.id() == invocation.publication_id && publication.enabled()
        })
}

fn workflow_watchdog_failure_is_terminal(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::WorkflowNotFound { .. }
            | DaemonError::WorkflowEndpointNotFound { .. }
            | DaemonError::WorkflowNodeNotFound { .. }
            | DaemonError::WorkflowNodeAgentMissing { .. }
            | DaemonError::InvalidWorkflowGraphReference { .. }
            | DaemonError::AgentNotFound { .. }
    )
}

#[cfg(test)]
mod tests;
