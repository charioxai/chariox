//! Claim cleanup for completed workflow runs and archived provider settlement.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn release_completed_workflow_write_claims(
        &self,
        session_id: &str,
        workflow_run: &crate::session::WorkflowRun,
    ) -> usize {
        if workflow_run.status() != crate::session::WorkflowRunStatus::Completed {
            return 0;
        }
        let owner_prefix = format!("{}:", workflow_run.id());
        self.prompt_workspace_claims.remove_matching(|claim| {
            claim.session_id == session_id
                && claim.operation == "workflow_node_dispatch"
                && claim
                    .attachment_id
                    .as_deref()
                    .is_some_and(|owner| owner.starts_with(&owner_prefix))
        })
    }

    pub(super) fn release_archived_completed_workflow_claims(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Option<bool> {
        let session = self.session_store.get_session(session_id).ok()?;
        if session.workflow_run(workflow_run_id).is_some() {
            return None;
        }
        let workflow_run = self
            .durable_state_store
            .resolve_workflow_run(
                session.host_daemon_id(),
                session.id(),
                workflow_run_id,
            )
            .ok()??;
        let completed_node = workflow_run.node_runs().iter().any(|node_run| {
            node_run.id() == workflow_node_run_id
                && node_run.status() == crate::session::WorkflowNodeRunStatus::Completed
        });
        if workflow_run.status() != crate::session::WorkflowRunStatus::Completed
            || workflow_run.final_output_valid() != Some(true)
            || !completed_node
        {
            return None;
        }

        Some(self.release_completed_workflow_write_claims(session_id, &workflow_run) > 0)
    }
}
