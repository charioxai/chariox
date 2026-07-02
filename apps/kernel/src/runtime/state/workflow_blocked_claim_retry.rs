//! Retry scheduling for workflow nodes blocked on workspace claims.

use super::*;

struct BlockedWorkflowClaimRetry {
    session_id: String,
    workflow_run_id: String,
    workflow_node_run_id: String,
    agent_id: String,
    node_id: String,
    prompt_text: String,
}

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_retry_blocked_claims(&self) -> WorkflowPromptDispatches {
        let mut dispatches = WorkflowPromptDispatches::default();
        for retry in self.collect_blocked_workflow_claim_retries() {
            if let Some(prepared) = self.retry_blocked_workflow_claim(retry) {
                dispatches.extend(prepared);
            }
        }
        dispatches
    }

    fn collect_blocked_workflow_claim_retries(&self) -> Vec<BlockedWorkflowClaimRetry> {
        let mut retries = Vec::new();
        for session in self
            .session_store
            .list_non_ended_sessions_including_hidden()
        {
            for workflow_run in session.workflow_runs() {
                for node_run in workflow_run.node_runs() {
                    if node_run.status()
                        != crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
                    {
                        continue;
                    }
                    let Some(prompt_text) = node_run
                        .turn_envelope()
                        .and_then(|envelope| envelope.rendered_prompt())
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    retries.push(BlockedWorkflowClaimRetry {
                        session_id: session.id().to_string(),
                        workflow_run_id: workflow_run.id().to_string(),
                        workflow_node_run_id: node_run.id().to_string(),
                        agent_id: node_run.agent_id().to_string(),
                        node_id: node_run.node_id().to_string(),
                        prompt_text,
                    });
                }
            }
        }
        retries
    }

    fn retry_blocked_workflow_claim(
        &self,
        retry: BlockedWorkflowClaimRetry,
    ) -> Option<WorkflowPromptDispatches> {
        let claim_id = match self.workflow_dispatch_claim_id(&retry.session_id, &retry.agent_id) {
            Ok(claim_id) => claim_id,
            Err(error) => {
                self.record_blocked_workflow_claim_retry_error(&retry, error);
                return None;
            }
        };
        match self.acquire_workflow_node_workspace_claim(
            &retry.session_id,
            &claim_id,
            &retry.agent_id,
            &retry.workflow_run_id,
            &retry.workflow_node_run_id,
        ) {
            Ok(()) => {
                let _ = self
                    .session_store
                    .write()
                    .ready_workflow_node_after_workspace_claim(
                        &retry.session_id,
                        &retry.workflow_run_id,
                        &retry.workflow_node_run_id,
                    );
            }
            Err(DaemonError::WorkspaceClaimConflict { .. }) => return None,
            Err(error) => {
                self.record_blocked_workflow_claim_retry_error(&retry, error);
                return None;
            }
        }
        let prompt = crate::session::PromptQueueItem::new(
            format!(
                "pending-draft:workflow-retry:{}:{}",
                retry.workflow_run_id, retry.workflow_node_run_id
            ),
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(&retry.workflow_run_id),
            retry.agent_id.clone(),
            retry.prompt_text.clone(),
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(&retry.workflow_run_id, &retry.workflow_node_run_id);
        match self.workflow_submit_prepared_prompt(
            crate::app::KernelPreparedPromptSubmission {
                session_id: retry.session_id.clone(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            },
            &retry.workflow_run_id,
            &retry.workflow_node_run_id,
        ) {
            Ok(prepared) => Some(prepared),
            Err(error) => {
                self.record_blocked_workflow_claim_retry_error(&retry, error);
                None
            }
        }
    }

    fn record_blocked_workflow_claim_retry_error(
        &self,
        retry: &BlockedWorkflowClaimRetry,
        error: DaemonError,
    ) {
        self.record_notice(
            &retry.session_id,
            None,
            self.attachment_store
                .list_session_attachment_ids(&retry.session_id),
            format!(
                "Workflow run `{}` could not retry blocked node `{}`: {}",
                retry.workflow_run_id, retry.node_id, error
            ),
        );
    }
}
