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
        dispatches.extend(self.workflow_retry_queued_prompts_waiting_for_claims());
        dispatches
    }

    fn workflow_retry_queued_prompts_waiting_for_claims(&self) -> WorkflowPromptDispatches {
        let mut dispatches = WorkflowPromptDispatches::default();
        for session in self
            .session_store
            .list_non_ended_sessions_including_hidden()
        {
            for agent in self.agent_store.get_session_agents(session.id()) {
                if self
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, agent.id())
                    .is_some()
                {
                    continue;
                }
                let Some(next_prompt) = self
                    .prompt_state_owner
                    .peek_next_queued_prompt(&session, agent.id())
                else {
                    continue;
                };
                if next_prompt.workflow_run_id().is_none() {
                    continue;
                }
                let Some(provider_run) = self
                    .provider_store
                    .get_run_for_agent(session.id(), agent.id())
                    .filter(|run| run.state() == crate::provider::ProviderRunState::Running)
                else {
                    continue;
                };
                match self.advance_next_queued_prompt_dispatch(
                    session.id(),
                    agent.id(),
                    provider_run.id(),
                ) {
                    Ok(Some(dispatch)) => dispatches.local.push(dispatch),
                    Ok(None) => {}
                    Err(error) => self.record_notice(
                        session.id(),
                        Some(provider_run.id()),
                        self.attachment_store
                            .list_session_attachment_ids(session.id()),
                        format!(
                            "Queued workflow prompt remained pending after workspace release: {error}"
                        ),
                    ),
                }
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
        let claim_id = self.workflow_dispatch_claim_id(
            &retry.session_id,
            &retry.workflow_run_id,
            &retry.workflow_node_run_id,
        );
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
                // Admission owns the claim only after it succeeds.  A provider/runtime
                // failure must not leave the blocked node's workspace claim behind and starve
                // subsequent event deliveries.
                self.release_workflow_node_workspace_claim(
                    &retry.session_id,
                    &retry.workflow_run_id,
                    &retry.workflow_node_run_id,
                );
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
