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
        let queued_prompt = self
            .session_store
            .write()
            .enqueue_workflow_prompt_with_publication_invocation(
                session_id,
                workflow.id(),
                endpoint.id(),
                prompt,
                queue_ref,
                crate::session::WorkflowQueuedPromptSource::Manual,
                None,
                publication_invocation,
            )?;
        if self
            .session_store
            .get_session(session_id)?
            .has_active_workflow_run()
        {
            return Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued {
                    queued_prompt,
                    workflow,
                    endpoint,
                },
                WorkflowPromptDispatches::default(),
            ));
        }
        if let Some(outcome) = self.workflow_start_next_queued_prompt_for_response(session_id)? {
            return Ok(outcome);
        }
        if self
            .session_store
            .read()
            .list_queued_workflow_prompts(session_id)?
            .iter()
            .any(|candidate| candidate.id() == queued_prompt.id())
        {
            return Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued {
                    queued_prompt,
                    workflow,
                    endpoint,
                },
                WorkflowPromptDispatches::default(),
            ));
        }
        Err(DaemonError::WorkflowLaunchRejected {
            session_id: session_id.to_string(),
            workflow_id: workflow.id().to_string(),
            endpoint_id: endpoint.id().to_string(),
            message: "workflow prompt was enqueued but no dispatchable queue item was found"
                .to_string(),
        })
    }

    pub(super) fn workflow_invoke_queued_prompt(
        &self,
        session_id: &str,
        queued_prompt: crate::session::WorkflowQueuedPrompt,
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
            .resolve_workflow_ref(session_id, queued_prompt.workflow_id())?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            queued_prompt.workflow_id(),
            queued_prompt.endpoint_id(),
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        let workflow_run = self
            .session_store
            .write()
            .invoke_workflow_endpoint_with_publication_invocation(
                session_id,
                workflow.id(),
                endpoint.id(),
                queued_prompt.prompt().map(str::to_string),
                queued_prompt.publication_invocation().cloned(),
            )?;
        if let Some(watchdog_id) = queued_prompt.watchdog_id() {
            let _ = self.session_store.write().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        let dispatches = match self.workflow_schedule_entry_node(session_id, &workflow_run) {
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
                workflow_run,
                workflow,
                endpoint,
            },
            dispatches,
        ))
    }

    fn workflow_start_next_queued_prompt_for_response(
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
            let Some(queued_prompt) = self
                .session_store
                .write()
                .dequeue_next_workflow_prompt(session_id)?
            else {
                return Ok(None);
            };
            if let Some(watchdog_id) = queued_prompt.watchdog_id() {
                let allowed = self
                    .session_store
                    .write()
                    .prepare_workflow_watchdog_queued_start(session_id, watchdog_id)?;
                if !allowed {
                    continue;
                }
            }
            match self.workflow_invoke_queued_prompt(session_id, queued_prompt.clone()) {
                Ok(outcome) => return Ok(Some(outcome)),
                Err(error) => {
                    if let Some(watchdog_id) = queued_prompt.watchdog_id() {
                        self.mark_workflow_watchdog_launch_failed(session_id, watchdog_id, &error);
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
            }
        }
    }

    pub(super) fn workflow_maybe_start_next_queued_prompt(
        &self,
        session_id: &str,
    ) -> WorkflowPromptDispatches {
        loop {
            let queued_prompt = match self
                .session_store
                .write()
                .dequeue_next_workflow_prompt(session_id)
            {
                Ok(Some(queued_prompt)) => queued_prompt,
                Ok(None) => return WorkflowPromptDispatches::default(),
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
            if let Some(watchdog_id) = queued_prompt.watchdog_id() {
                match self
                    .session_store
                    .write()
                    .prepare_workflow_watchdog_queued_start(session_id, watchdog_id)
                {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(error) => {
                        self.record_notice(
                            session_id,
                            None,
                            self.attachment_store
                                .list_session_attachment_ids(session_id),
                            format!(
                                "Queued workflow watchdog prompt `{}` failed: {error}",
                                queued_prompt.id()
                            ),
                        );
                        continue;
                    }
                }
            }
            match self.workflow_invoke_queued_prompt(session_id, queued_prompt.clone()) {
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
                    if let Some(watchdog_id) = queued_prompt.watchdog_id() {
                        self.mark_workflow_watchdog_launch_failed(session_id, watchdog_id, &error);
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
            }
        }
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
