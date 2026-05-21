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
        let claim_id = self.workflow_dispatch_claim_id(session_id, node_run.agent_id())?;
        match self.acquire_workflow_node_workspace_claim(
            session_id,
            &claim_id,
            node_run.agent_id(),
            workflow_run.id(),
            node_run.id(),
        ) {
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
        let prompt = crate::session::PromptQueueItem::new(
            self.session_store.reserve_prompt_id(),
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
            node_run.agent_id(),
            prompt_text,
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run.id(), node_run.id());
        self.workflow_submit_prepared_prompt(
            crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
            },
            workflow_run.id(),
            node_run.id(),
        )
    }
}
