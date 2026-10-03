//! Claim cleanup for completed workflow runs and archived provider settlement.

use super::*;

impl KernelRuntimeOwnedState {
    /// Recover retained dispatch holds after their completed owner has settled.
    /// Completion alone is insufficient: providers may still be finishing a turn.
    pub(super) fn reconcile_completed_workflow_write_claims(&self) -> usize {
        let mut released = 0;
        for claim in self.workspace_coordinator.active_claims() {
            if claim.operation != "workflow_node_dispatch" {
                continue;
            }
            let Some((run_id, node_id)) = claim
                .attachment_id
                .as_deref()
                .and_then(|owner| owner.split_once(':'))
            else {
                continue;
            };
            let Ok(session) = self.session_store.get_session(&claim.session_id) else {
                continue;
            };
            // Prefer current state; archived history is evidence only when the run
            // is no longer resident. Failed reads never authorize releasing a hold.
            let run = session.workflow_run(run_id).cloned().or_else(|| {
                self.durable_state_store
                    .resolve_workflow_run(session.host_daemon_id(), session.id(), run_id)
                    .ok()
                    .flatten()
            });
            let Some(run) =
                run.filter(|run| run.status() == crate::session::WorkflowRunStatus::Completed)
            else {
                continue;
            };
            let Some(node) = run.node_runs().iter().find(|node| {
                node.id() == node_id
                    && node.status() == crate::session::WorkflowNodeRunStatus::Completed
            }) else {
                continue;
            };
            if self
                .prompt_state_owner
                .active_prompt_for_agent(&session, node.agent_id())
                .is_some()
                || self
                    .active_turns
                    .snapshot()
                    .values()
                    .any(|turn| turn.session_id == session.id() && turn.agent_id == node.agent_id())
            {
                continue;
            }
            // Match the unique coordinator claim, not all claims for this run:
            // other nodes can still own live provider work after run completion.
            released += self
                .prompt_workspace_claims
                .remove_matching(|current| current == &claim);
        }
        released
    }

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
            .resolve_workflow_run(session.host_daemon_id(), session.id(), workflow_run_id)
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
