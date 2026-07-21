//! Workflow completion, cancellation, and failure state transitions.
//!
//! This module owns provider-turn settlement for workflow prompts, completion snapshots,
//! output checks, and failure recording. Downstream dispatch construction stays in
//! `workflow_dispatch`.

use super::*;

impl KernelRuntimeOwnedState {
    fn persist_workflow_completion_session(
        &self,
        session_id: &str,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let session = self.session_snapshot(session_id)?;
        self.durable_state_store.append_event(
            "session.updated",
            Some(session_id.to_string()),
            serde_json::json!({
                "session": &session,
                "reason": "workflow_prompt_completed",
            }),
        )?;
        Ok(session)
    }

    #[allow(dead_code)]
    pub(super) fn workflow_complete_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(WorkflowPromptDispatches::default());
        };
        let completion_snapshot = self.workflow_completion_snapshot(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            provider_run_id,
        );
        let has_valid_pending_final_output = self.workflow_node_run_has_valid_pending_final_output(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        if completion_snapshot.is_none() && !has_valid_pending_final_output {
            if let Some(provider_diagnostic) =
                provider_run_id.and_then(|run_id| self.provider_run_terminal_diagnostic(run_id))
            {
                self.workflow_record_failure(
                    session_id,
                    workflow_run_id,
                    &crate::session::WorkflowFailureEvent::new(
                        crate::session::WorkflowFailureKind::ProviderFailure,
                        workflow_node_run_id,
                        Vec::new(),
                        provider_diagnostic.clone(),
                    ),
                );
                self.session_store.write().fail_workflow_node_run(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
                let _ = self.release_workflow_node_workspace_claim(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                );
                self.record_notice(
                    session_id,
                    provider_run_id,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{workflow_run_id}` failed after provider turn failure: {provider_diagnostic}"
                    ),
                );
                self.workflow_maybe_start_next_queued_prompt(session_id);
                self.persist_workflow_completion_session(session_id)?;
                return Ok(WorkflowPromptDispatches::default());
            }
        }
        let max_turns = self.workflow_max_turns(session_id);
        let completion_result = self
            .session_store
            .write()
            .complete_workflow_node_run_after_provider_turn(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
                completion_snapshot.clone(),
                max_turns,
            );
        let update = match completion_result {
            Ok(update) => update,
            Err(error) => return Err(error),
        };
        for warning in &update.validation_warnings {
            let failure = crate::session::WorkflowFailureEvent::new(
                crate::session::classify_workflow_failure_kind(
                    &completion_snapshot,
                    &warning.message,
                ),
                workflow_node_run_id,
                vec![warning.edge_id.clone()],
                warning.message.clone(),
            );
            self.workflow_record_failure(session_id, workflow_run_id, &failure);
            self.record_notice(
                session_id,
                None,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Workflow handoff validation warning on edge `{}`: {}",
                    warning.edge_id, warning.message
                ),
            );
        }
        if update.workflow_run.status() == crate::session::WorkflowRunStatus::Stopped
            && update.workflow_run.final_output().is_none()
            && update.workflow_run.failure_events().iter().all(|event| {
                event.kind() != crate::session::WorkflowFailureKind::NodeTurnBudgetExhausted
            })
        {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::NodeTurnBudgetExhausted,
                    workflow_node_run_id,
                    Vec::new(),
                    "workflow run stopped after a node exhausted its turn budget",
                ),
            );
        }
        if let Some(failure) = update.handoff_validation_failure.as_ref() {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::OutputValidationFailed,
                    workflow_node_run_id,
                    vec![failure.edge_id.clone()],
                    failure.message.clone(),
                ),
            );
            self.record_notice(
                session_id,
                provider_run_id,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                if failure.retry_scheduled {
                    format!(
                        "Workflow handoff on edge `{}` failed validation on attempt {}/{}; a corrective turn was scheduled: {}",
                        failure.edge_id, failure.attempt, failure.max_attempts, failure.message
                    )
                } else {
                    format!(
                        "Workflow run `{workflow_run_id}` failed handoff validation on edge `{}` after attempt {}/{}: {}",
                        failure.edge_id, failure.attempt, failure.max_attempts, failure.message
                    )
                },
            );
        }
        if let Some(failure) = update.run_output_validation_failure.as_ref() {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::WorkflowRunOutputValidationFailed,
                    workflow_node_run_id,
                    Vec::new(),
                    failure.message.clone(),
                ),
            );
            self.record_notice(
                session_id,
                provider_run_id,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                if failure.retry_scheduled {
                    format!(
                        "Workflow run `{workflow_run_id}` final output failed validation on attempt {}/{}; a corrective turn was scheduled: {}",
                        failure.attempt, failure.max_attempts, failure.message
                    )
                } else {
                    format!(
                        "Workflow run `{workflow_run_id}` failed final output validation after attempt {}/{}: {}",
                        failure.attempt, failure.max_attempts, failure.message
                    )
                },
            );
        }
        if let Some(failure) = update.missing_output_failure.as_ref() {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::MissingStructuredOutput,
                    workflow_node_run_id,
                    Vec::new(),
                    failure.message.clone(),
                ),
            );
            self.record_notice(
                session_id,
                provider_run_id,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                if failure.retry_scheduled {
                    format!(
                        "Workflow run `{workflow_run_id}` produced no structured output on attempt {}/{}; a corrective turn was scheduled.",
                        failure.attempt, failure.max_attempts
                    )
                } else {
                    format!(
                        "Workflow run `{workflow_run_id}` failed after producing no structured output on attempt {}/{}.",
                        failure.attempt, failure.max_attempts
                    )
                },
            );
        }
        if update.validation_warnings.is_empty()
            && update.handoff_validation_failure.is_none()
            && update.missing_output_failure.is_none()
            && update.run_output_validation_failure.is_none()
        {
            let _ = self
                .session_store
                .write()
                .mark_workflow_turn_validated_completed(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
        }
        let released_workflow_claim = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        let mut dispatches =
            self.workflow_prepare_dispatches(session_id, workflow_run_id, &update.dispatches);
        if released_workflow_claim {
            dispatches.extend(self.workflow_retry_blocked_claims());
        }
        let state_suffix = match update.workflow_run.status() {
            crate::session::WorkflowRunStatus::Waiting => "waiting for downstream handoffs",
            crate::session::WorkflowRunStatus::Completing => "is completing",
            crate::session::WorkflowRunStatus::Completed => "completed",
            crate::session::WorkflowRunStatus::Failed => "failed",
            crate::session::WorkflowRunStatus::Stopped => "stopped",
            _ => "updated",
        };
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!(
                "Workflow run `{}` {state_suffix}.",
                update.workflow_run.id()
            ),
        );
        if matches!(
            update.workflow_run.status(),
            crate::session::WorkflowRunStatus::Completed
                | crate::session::WorkflowRunStatus::Failed
                | crate::session::WorkflowRunStatus::Stopped
        ) {
            let event_kind = match update.workflow_run.status() {
                crate::session::WorkflowRunStatus::Completed => "workflow.run.completed",
                crate::session::WorkflowRunStatus::Failed => "workflow.run.failed",
                crate::session::WorkflowRunStatus::Stopped => "workflow.run.cancelled",
                _ => "workflow.run.updated",
            };
            let source_agent_id = update
                .workflow_run
                .completed_by_node_run_id()
                .and_then(|node_run_id| {
                    update
                        .workflow_run
                        .node_runs()
                        .iter()
                        .find(|node_run| node_run.id() == node_run_id)
                })
                .map(|node_run| node_run.agent_id().to_string());
            let source_attachment_id =
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(
                    update.workflow_run.id(),
                );
            dispatches.extend(self.metaagent_workflow_event_prompt_dispatches(
                session_id,
                event_kind,
                source_agent_id.as_deref(),
                &source_attachment_id,
                format!(
                    "Workflow run `{}` {state_suffix}",
                    update.workflow_run.id()
                ),
                format!(
                    "Workflow run `{}` {state_suffix}.",
                    update.workflow_run.id()
                ),
                serde_json::json!({
                    "workflow_run_id": update.workflow_run.id(),
                    "workflow_id": update.workflow_run.workflow_id(),
                    "endpoint_id": update.workflow_run.endpoint_id(),
                    "status": format!("{:?}", update.workflow_run.status()),
                    "final_output": update.workflow_run.final_output().map(|output| output.message()),
                    "final_output_valid": update.workflow_run.final_output_valid(),
                    "final_output_warning": update.workflow_run.final_output_warning(),
                    "failure_events": update.workflow_run.failure_events(),
                }),
            ));
        }
        if matches!(
            update.workflow_run.status(),
            crate::session::WorkflowRunStatus::Completed
                | crate::session::WorkflowRunStatus::Failed
                | crate::session::WorkflowRunStatus::Stopped
        ) {
            dispatches.extend(self.workflow_maybe_start_next_queued_prompt(session_id));
        }
        self.persist_workflow_completion_session(session_id)?;
        Ok(dispatches)
    }

    pub(super) fn workflow_record_failure(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        failure: &crate::session::WorkflowFailureEvent,
    ) {
        let _ = self.session_store.write().record_workflow_failure_event(
            session_id,
            workflow_run_id,
            failure.clone(),
        );
    }
}
