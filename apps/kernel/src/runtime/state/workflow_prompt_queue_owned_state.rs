//! Workflow prompt queue advancement.

use super::*;

impl KernelRuntimeOwnedState {
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
