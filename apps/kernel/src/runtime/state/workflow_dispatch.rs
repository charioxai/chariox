//! Workflow scheduling and node-dispatch state transitions.
//!
//! This module advances queued/running workflow nodes, applies retry policy, records completion,
//! and schedules provider prompts for executable workflow nodes.

use super::*;

impl KernelRuntimeOwnedState {
    #[allow(dead_code)]
    pub(super) fn workflow_prepare_dispatches(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        dispatches: &[crate::session::WorkflowDispatch],
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let mut prepared = WorkflowPromptDispatches::default();
        for dispatch in dispatches {
            self.record_notice(
                session_id,
                None,
                self.attachment_store
                    .list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` routed {} upstream message(s) to node `{}`.",
                    dispatch.messages.len(),
                    dispatch.node_run.node_id()
                ),
            );
            let handoff_payloads_json =
                serde_json::to_string(&dispatch.messages).unwrap_or_else(|_| "[]".to_string());
            let control_mailbox = self.workflow_control_mailbox_text(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
            );
            let prompt_text = match self.workflow_turn_prompt_text(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                dispatch.node_run.node_id(),
                dispatch.endpoint_prompt.as_deref().unwrap_or(""),
                Some(&handoff_payloads_json),
                control_mailbox.as_deref(),
            ) {
                Ok(prompt_text) => prompt_text,
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not prepare downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                    continue;
                }
            };
            let _ = self.session_store.write().prepare_workflow_turn(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                format!("workflow-ack:{}", dispatch.node_run.id()),
                prompt_text.clone(),
                control_mailbox,
                Some(handoff_payloads_json),
            );
            let claim_id = self.workflow_dispatch_claim_id(
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
            );
            let agent_busy = match self
                .workflow_agent_has_prompt_work(session_id, dispatch.node_run.agent_id())
            {
                Ok(agent_busy) => agent_busy,
                Err(error) => {
                    // Never default to "idle" on error: scheduling against a busy
                    // agent must stay conservative or the claim/queue invariants break.
                    self.record_notice(
                            session_id,
                            None,
                            self.attachment_store.list_session_attachment_ids(session_id),
                            format!(
                                "Workflow run `{workflow_run_id}` could not schedule downstream node `{}` after an agent busyness error: {}",
                                dispatch.node_run.node_id(),
                                error
                            ),
                        );
                    continue;
                }
            };
            if !agent_busy {
                match self.acquire_workflow_node_workspace_claim(
                    session_id,
                    &claim_id,
                    dispatch.node_run.agent_id(),
                    workflow_run_id,
                    dispatch.node_run.id(),
                ) {
                    Ok(()) => {
                        let _ = self
                            .session_store
                            .write()
                            .ready_workflow_node_after_workspace_claim(
                                session_id,
                                workflow_run_id,
                                dispatch.node_run.id(),
                            );
                    }
                    Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
                        let _ = self
                            .session_store
                            .write()
                            .block_workflow_node_on_workspace_claim(
                                session_id,
                                workflow_run_id,
                                dispatch.node_run.id(),
                            );
                        self.record_notice(
                            session_id,
                            None,
                            self.attachment_store.list_session_attachment_ids(session_id),
                            format!(
                                "Workflow run `{workflow_run_id}` blocked node `{}` on a workspace claim: {error}",
                                dispatch.node_run.node_id()
                            ),
                        );
                        continue;
                    }
                    Err(error) => {
                        self.record_notice(
                            session_id,
                            None,
                            self.attachment_store.list_session_attachment_ids(session_id),
                            format!(
                                "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                                dispatch.node_run.node_id(),
                                error
                            ),
                        );
                        continue;
                    }
                }
            }
            let prompt = crate::session::PromptQueueItem::new(
                format!(
                    "pending-draft:workflow-dispatch:{}:{}",
                    workflow_run_id,
                    dispatch.node_run.id()
                ),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run_id),
                dispatch.node_run.agent_id(),
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run_id, dispatch.node_run.id());
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.to_string(),
                    prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                workflow_run_id,
                dispatch.node_run.id(),
            ) {
                Ok(dispatches) => {
                    // Only a prompt that started immediately may keep its workspace claim.
                    // A queued prompt releases it so the per-agent queue head can be
                    // promoted later without conflicting with its own worktree claim.
                    if !dispatches.admitted_workflow_prompt || dispatches.queued_workflow_prompt {
                        self.release_workflow_node_workspace_claim(
                            session_id,
                            workflow_run_id,
                            dispatch.node_run.id(),
                        );
                    }
                    prepared.extend(dispatches)
                }
                Err(error) => {
                    // The claim is acquired before prompt admission.  If admission fails
                    // (for example because the provider was replaced during recovery), release
                    // it here; otherwise every later event for the session is falsely blocked
                    // behind a node that never reached the provider queue.
                    self.release_workflow_node_workspace_claim(
                        session_id,
                        workflow_run_id,
                        dispatch.node_run.id(),
                    );
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store.list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                            dispatch.node_run.node_id(),
                            error
                        ),
                    );
                    self.workflow_fail_node_after_dispatch_error(
                        session_id,
                        workflow_run_id,
                        dispatch.node_run.id(),
                        &error,
                    );
                }
            }
        }
        Ok(prepared)
    }

    fn workflow_fail_node_after_dispatch_error(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        error: &DaemonError,
    ) {
        self.workflow_record_failure(
            session_id,
            workflow_run_id,
            &crate::session::WorkflowFailureEvent::new(
                crate::session::WorkflowFailureKind::TransportFailure,
                workflow_node_run_id,
                Vec::new(),
                error.to_string(),
            ),
        );
        let _ = self.session_store.write().fail_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        );
        self.workflow_maybe_start_next_queued_prompt(session_id);
        let _ = self.session_snapshot(session_id);
    }
}
