//! Workflow prompt queue advancement.

use super::*;

impl KernelRuntimeOwnedState {
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
        if let Some(watchdog_id) = queued_prompt.watchdog_id() {
            let _ = self.session_store.write().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        Ok((
            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            },
            dispatches,
        ))
    }

    pub(super) fn workflow_maybe_start_next_queued_prompt(
        &self,
        session_id: &str,
    ) -> WorkflowPromptDispatches {
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
            let _ = self
                .session_store
                .write()
                .mark_workflow_watchdog_pending_started(session_id, watchdog_id);
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
                    let _ = self.session_store.write().mark_workflow_watchdog_failed(
                        session_id,
                        watchdog_id,
                        error.to_string(),
                    );
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
        WorkflowPromptDispatches::default()
    }
}
