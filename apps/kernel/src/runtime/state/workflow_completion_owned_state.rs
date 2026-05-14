//! Workflow completion, cancellation, and failure state transitions.
//!
//! This module owns provider-turn settlement for workflow prompts, completion snapshots,
//! output checks, and failure recording. Downstream dispatch construction stays in
//! `workflow_dispatch`.

use super::*;

impl KernelRuntimeOwnedState {
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
            let message = "provider completed workflow turn without a validated workflow output";
            let provider_diagnostic =
                provider_run_id.and_then(|run_id| self.provider_run_terminal_diagnostic(run_id));
            let (failure_kind, failure_message, notice_message) = if let Some(diagnostic) =
                provider_diagnostic
            {
                (
                    crate::session::WorkflowFailureKind::ProviderFailure,
                    diagnostic.clone(),
                    format!(
                        "Workflow run `{workflow_run_id}` failed after provider turn failure: {diagnostic}"
                    ),
                )
            } else {
                (
                    crate::session::WorkflowFailureKind::MissingStructuredOutput,
                    message.to_string(),
                    format!("Workflow run `{workflow_run_id}` failed: {message}."),
                )
            };
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    failure_kind,
                    workflow_node_run_id,
                    Vec::new(),
                    failure_message,
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
                notice_message,
            );
            self.workflow_maybe_start_next_queued_launch(session_id);
            let _ = self.session_snapshot(session_id)?;
            return Ok(WorkflowPromptDispatches::default());
        }
        let max_turns = self.workflow_max_turns(session_id);
        let completion_result = self.session_store.write().complete_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            completion_snapshot.clone(),
            max_turns,
        );
        let update = match completion_result {
            Ok(update) => update,
            Err(crate::error::DaemonError::WorkflowOutputValidationFailed {
                edge_id,
                message,
                ..
            }) => {
                self.workflow_record_failure(
                    session_id,
                    workflow_run_id,
                    &crate::session::WorkflowFailureEvent::new(
                        crate::session::WorkflowFailureKind::OutputValidationFailed,
                        workflow_node_run_id,
                        vec![edge_id.clone()],
                        message.clone(),
                    ),
                );
                self.session_store.write().stop_workflow_node_run(
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
                    None,
                    self.attachment_store.list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{workflow_run_id}` stopped after validation failed on edge `{edge_id}`: {message}"
                    ),
                );
                self.workflow_maybe_start_next_queued_launch(session_id);
                let _ = self.session_snapshot(session_id)?;
                return Ok(WorkflowPromptDispatches::default());
            }
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
                    "Workflow output validation warning on edge `{}`: {}",
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
        if update.workflow_run.final_output_valid() == Some(false) {
            self.workflow_record_failure(
                session_id,
                workflow_run_id,
                &crate::session::WorkflowFailureEvent::new(
                    crate::session::WorkflowFailureKind::WorkflowRunOutputValidationFailed,
                    workflow_node_run_id,
                    Vec::new(),
                    update
                        .workflow_run
                        .final_output_warning()
                        .unwrap_or("workflow run output validation failed"),
                ),
            );
        }
        if update.validation_warnings.is_empty() {
            let _ = self
                .session_store
                .write()
                .mark_workflow_turn_validated_completed(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
        }
        let claim_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, prompt.target_agent_id())
                .map(|run| run.id().to_string())
        });
        let released_claim = claim_provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false);
        let released_workflow_claim = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        let mut dispatches =
            self.workflow_prepare_dispatches(session_id, workflow_run_id, &update.dispatches);
        if released_claim || released_workflow_claim {
            dispatches.extend(self.workflow_retry_blocked_claims());
        }
        let state_suffix = match update.workflow_run.status() {
            crate::session::WorkflowRunStatus::Waiting => "waiting for downstream handoffs",
            crate::session::WorkflowRunStatus::Completing => "is completing",
            crate::session::WorkflowRunStatus::Completed => "completed",
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
            self.workflow_maybe_start_next_queued_launch(session_id);
        }
        let _ = self.session_snapshot(session_id)?;
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
