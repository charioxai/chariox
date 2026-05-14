//! Workflow prompt cancellation and provider-failure transitions.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_cancel_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        let workflow_run = self.session_store.write().stop_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        let _ = self.release_workflow_node_workspace_claim(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::RunStopped,
                workflow_node_run_id,
                Vec::new(),
                "workflow node run was stopped before validated completion",
            ),
        );
        self.record_notice(
            session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(session_id),
            format!("Workflow run `{}` was stopped.", workflow_run.id()),
        );
        self.workflow_maybe_start_next_queued_launch(session_id);
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    pub(super) fn workflow_fail_provider_prompt(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        provider_run_id: Option<&str>,
        message: &str,
    ) -> Result<(), DaemonError> {
        let (Some(workflow_run_id), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            return Ok(());
        };
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::ProviderFailure,
                workflow_node_run_id,
                Vec::new(),
                message,
            ),
        );
        let workflow_run = self.session_store.write().fail_workflow_node_run(
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
                "Workflow run `{}` failed after provider turn failure: {}",
                workflow_run.id(),
                message
            ),
        );
        self.workflow_maybe_start_next_queued_launch(session_id);
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }
}
