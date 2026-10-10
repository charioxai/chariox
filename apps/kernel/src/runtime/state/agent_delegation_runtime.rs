//! MP-08/MP-10/MP-11 F4: correlate accepted child prompts with the submitting task.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, Operation};
impl KernelRuntimeState {
    pub(super) fn delegated_prompt_parent(
        &self,
        room: &str,
    ) -> Result<Option<String>, DaemonError> {
        if !self.owned.config_projection.snapshot().room_agent_tools {
            return Ok(None);
        }
        let Some((actor, run)) = &self.room_provider_origin else {
            return Ok(None);
        };
        let session = self.owned.session_store.get_session(room)?;
        let active = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, actor);
        Ok(self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(actor))?
            .into_iter()
            .find(|t| {
                active.as_ref().is_some_and(|p| t.prompt_id == p.id())
                    && t.provider_run_id.as_deref() == Some(run.as_str())
            })
            .map(|t| t.task_id))
    }
    pub(super) fn reconcile_delegated_prompt(
        &self,
        parent: Option<String>,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) {
        let Some(parent_task) = parent else {
            return;
        };
        let result = (|| {
            // A resumed child turn keeps its original logical task ID.
            let child = self
                .owned
                .durable_state_store
                .agent_tasks(
                    Some(&prepared.session_id),
                    Some(prepared.prompt.target_agent_id()),
                )?
                .into_iter()
                .find(|t| t.prompt_id == prepared.prompt.id())
                .ok_or_else(|| ledger::error("accepted delegation child task unavailable"))?;
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::BindDelegate {
                    parent_task,
                    child_task: child.task_id,
                })?;
            Ok::<(), DaemonError>(())
        })();
        if let Err(error) = result {
            // Provider admission already succeeded; a ledger error cannot report it rejected.
            crate::logging::error_with_fields(
                "daemon.agent",
                "Accepted delegation correlation failed",
                serde_json::json!({"session_id":prepared.session_id,"prompt_id":prepared.prompt.id(),"error":error.to_string()}),
            );
        }
    }
}

// MP-08/MP-10/MP-11 F4/G11: workflow dispatch bypasses ordinary room prompt admission.
impl KernelRuntimeOwnedState {
    pub(super) fn reconcile_workflow_delegation(
        &self,
        room: &str,
        agent: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let Some(workflow_run) = prompt.workflow_run_id() else {
            return Ok(());
        };
        let Some(child) = self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .into_iter()
            .find(|t| t.prompt_id == prompt.id())
        else {
            return Ok(());
        };
        // Both references are kernel assigned: the child's prompt context and the
        // parent's accepted receipt name the same invocation, not merely an agent ID.
        for parent in self.durable_state_store.agent_tasks(Some(room), None)? {
            if parent.owner_user_id == child.owner_user_id
                && parent.obligations.iter().any(|o| {
                    o.tracks_workflow_run()
                        && o.dispatch_state == "accepted"
                        && o.resource_id.as_deref() == Some(workflow_run)
                })
            {
                self.durable_state_store
                    .agent_lifecycle(Operation::BindDelegate {
                        parent_task: parent.task_id,
                        child_task: child.task_id.clone(),
                    })?;
            }
        }
        Ok(())
    }
}
