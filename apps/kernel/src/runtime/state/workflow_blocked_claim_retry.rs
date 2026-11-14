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

impl KernelRuntimeState {
    /// A blocked node waits for the claim it was refused for, whoever holds it.
    /// Retry as soon as such a claim is released.
    pub(crate) async fn run_contended_claim_retries(&self) {
        loop {
            self.owned
                .workspace_coordinator
                .contended_claim_released()
                .await;
            self.spawn_workflow_prompt_dispatches(self.owned.workflow_retry_blocked_claims());
        }
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_retry_blocked_claims(&self) -> WorkflowPromptDispatches {
        self.reconcile_completed_workflow_write_claims();
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
        // A provider launch holds the prompt owner's lock while it takes the session store's
        // write lock, so the owner must never be asked while a read guard is held. Copy the
        // Rooms with a running agent, release the guard, then ask the owner.
        let running: Vec<_> = {
            let sessions = self.session_store.read();
            sessions
                .non_ended_session_refs()
                .filter_map(|session| {
                    let agents: Vec<String> = self
                        .agent_store
                        .get_session_agents(session.id())
                        .into_iter()
                        .filter(|agent| {
                            self.provider_store
                                .get_run_for_agent(session.id(), agent.id())
                                .is_some_and(|run| {
                                    run.state() == crate::provider::ProviderRunState::Running
                                })
                        })
                        .map(|agent| agent.id().to_string())
                        .collect();
                    (!agents.is_empty()).then(|| (session.clone(), agents))
                })
                .collect()
        };
        let mut candidates = Vec::new();
        for (session, agents) in &running {
            for agent_id in agents {
                if self
                    .prompt_state_owner
                    .active_prompt_for_agent(session, agent_id)
                    .is_some()
                {
                    continue;
                }
                let Some(next_prompt) = self
                    .prompt_state_owner
                    .peek_next_queued_prompt(session, agent_id)
                else {
                    continue;
                };
                if next_prompt.workflow_run_id().is_none() {
                    continue;
                }
                candidates.push((session.id().to_string(), agent_id.clone()));
            }
        }
        // A claim retry must prepare the workflow provider before consuming the head,
        // using the same promotion owner as cancellation and provider readiness.
        WorkflowPromptDispatches {
            project_queue_promotions: candidates,
            ..WorkflowPromptDispatches::default()
        }
    }

    fn collect_blocked_workflow_claim_retries(&self) -> Vec<BlockedWorkflowClaimRetry> {
        let mut retries = Vec::new();
        let sessions = self.session_store.read();
        for session in sessions.non_ended_session_refs() {
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
                        prompt_text:
                            crate::scheduler::prompt_injection::refresh_recovered_workflow_prompt(
                                session,
                                workflow_run.id(),
                                node_run.id(),
                                &prompt_text,
                            ),
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
        match self.workflow_retry_durable_entry(
            &retry.session_id,
            &retry.workflow_run_id,
            &retry.workflow_node_run_id,
        ) {
            Ok(Some(dispatches)) => return Some(dispatches),
            Ok(None) => {} // Existing downstream-node path below.
            Err(error) => {
                self.record_blocked_workflow_claim_retry_error(&retry, error);
                return None;
            }
        }
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
            Ok(prepared) => {
                // A prompt admitted to a busy provider queue must not keep its claim:
                // the retained worktree hold would deadlock earlier queued work for the
                // same agent, which can only be promoted after this prompt runs.
                if prepared.queued_workflow_prompt {
                    self.release_workflow_node_workspace_claim(
                        &retry.session_id,
                        &retry.workflow_run_id,
                        &retry.workflow_node_run_id,
                    );
                }
                Some(prepared)
            }
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
