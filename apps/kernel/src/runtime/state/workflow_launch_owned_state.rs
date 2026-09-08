//! Workflow launch, queued-start, and resume state transitions.
//!
//! This module owns entry-node scheduling and manual/queued workflow-run admission. Completion,
//! failure, and node fan-out remain in `workflow_dispatch`.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_schedule_entry_node(
        &self,
        session_id: &str,
        workflow_run: &crate::session::WorkflowRun,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        self.durable_state_store.require_writer_healthy()?;
        let intent = self.workflow_entry_intent(session_id, workflow_run.id())?;
        if intent.as_ref().is_some_and(|intent| intent.submitted) {
            // Existing durable prompt recovery owns this entry even after its
            // live prompt has completed and disappeared from the prompt queue.
            return Ok(WorkflowPromptDispatches::default());
        }
        let endpoint_prompt = workflow_run
            .invocation_prompt()
            .map(str::trim)
            .unwrap_or("");
        let node_run = workflow_run.node_runs().first().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_run.workflow_id().to_string(),
                reference: workflow_run.id().to_string(),
                message: "workflow run has no entry node run",
            }
        })?;
        // Repeated contention checks must not rewrite the turn or repeat its
        // notice every maintenance pass. Only a newly acquired claim progresses.
        let claim_id =
            self.workflow_dispatch_claim_id(session_id, workflow_run.id(), node_run.id());
        let mut preclaimed = false;
        if node_run.status() == crate::session::WorkflowNodeRunStatus::BlockedOnWorkspaceClaim
            && !self.workflow_agent_has_prompt_work(session_id, node_run.agent_id())?
        {
            match self.acquire_workflow_node_workspace_claim(
                session_id,
                &claim_id,
                node_run.agent_id(),
                workflow_run.id(),
                node_run.id(),
            ) {
                Ok(()) => preclaimed = true,
                Err(DaemonError::WorkspaceClaimConflict { .. }) => {
                    return Ok(WorkflowPromptDispatches::default())
                }
                Err(error) => return Err(error),
            }
        }
        let prompt_text = self.workflow_turn_prompt_text(
            session_id,
            workflow_run.id(),
            node_run.id(),
            node_run.node_id(),
            endpoint_prompt,
            None,
            None,
        )?;
        let _ = self.session_store.write().prepare_workflow_turn(
            session_id,
            workflow_run.id(),
            node_run.id(),
            format!("workflow-ack:{}", node_run.id()),
            prompt_text.clone(),
            None,
            None,
        )?;
        // A durable accepted prompt must have an authoritative turn envelope
        // available to the existing restart recovery renderer.
        self.persist_workflow_runtime_session(session_id, "workflow_entry_prepared")?;
        let agent_busy = self.workflow_agent_has_prompt_work(session_id, node_run.agent_id())?;
        if !agent_busy {
            let claim = if preclaimed {
                Ok(())
            } else {
                self.acquire_workflow_node_workspace_claim(
                    session_id,
                    &claim_id,
                    node_run.agent_id(),
                    workflow_run.id(),
                    node_run.id(),
                )
            };
            match claim {
                Ok(()) => {}
                Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
                    let _ = self
                        .session_store
                        .write()
                        .block_workflow_node_on_workspace_claim(
                            session_id,
                            workflow_run.id(),
                            node_run.id(),
                        );
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{}` blocked node `{}` on a workspace claim: {error}",
                            workflow_run.id(),
                            node_run.node_id()
                        ),
                    );
                    let _ = self.session_snapshot(session_id)?;
                    return Ok(WorkflowPromptDispatches::default());
                }
                Err(error) => return Err(error),
            }
            let _ = self
                .session_store
                .write()
                .ready_workflow_node_after_workspace_claim(
                    session_id,
                    workflow_run.id(),
                    node_run.id(),
                );
        }
        let prompt = crate::session::PromptQueueItem::new(
            format!(
                "pending-draft:workflow-launch:{}:{}",
                workflow_run.id(),
                node_run.id()
            ),
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
            node_run.agent_id(),
            prompt_text,
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run.id(), node_run.id());
        let prompt = match intent {
            Some(intent) => prompt.with_durable_operation(intent.operation_id, intent.fingerprint),
            None => prompt,
        };
        let mut dispatches = self.workflow_submit_prepared_prompt(
            crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            },
            workflow_run.id(),
            node_run.id(),
        )?;
        // Only a prompt that started immediately may keep its workspace claim. A prompt
        // queued behind existing agent work releases the claim so the queue head can be
        // promoted later without hitting its own worktree conflict.
        if !dispatches.admitted_workflow_prompt || dispatches.queued_workflow_prompt {
            self.release_workflow_node_workspace_claim(
                session_id,
                workflow_run.id(),
                node_run.id(),
            );
        }
        let source_attachment_id =
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id());
        dispatches.extend(self.metaagent_workflow_event_prompt_dispatches(
            session_id,
            "workflow.run.started",
            Some(node_run.agent_id()),
            &source_attachment_id,
            format!("Workflow run `{}` started", workflow_run.id()),
            format!(
                "Workflow run `{}` started on entry node `{}`.",
                workflow_run.id(),
                node_run.node_id()
            ),
            serde_json::json!({
                "workflow_run_id": workflow_run.id(),
                "workflow_id": workflow_run.workflow_id(),
                "endpoint_id": workflow_run.endpoint_id(),
                "entry_node_run_id": node_run.id(),
                "entry_node_id": node_run.node_id(),
                "entry_agent_id": node_run.agent_id(),
            }),
        ));
        Ok(dispatches)
    }
}
