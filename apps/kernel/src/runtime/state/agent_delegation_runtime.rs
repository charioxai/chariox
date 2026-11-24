//! MP-08/MP-10/MP-11 F4: correlate accepted child prompts with the submitting task.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, Operation};
impl KernelRuntimeState {
    pub(super) fn delegated_prompt_parent(&self) -> Result<Option<String>, DaemonError> {
        if !self.owned.config_projection.snapshot().room_agent_tools {
            return Ok(None);
        }
        let Some((actor, run)) = &self.room_provider_origin else {
            return Ok(None);
        };
        let origin = self.owned.agent_store.get_agent(actor)?;
        let room = origin.session_id();
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
            let operation = if self.is_cross_room_spawn_prompt(prepared) {
                Operation::BindCrossRoomDelegate {
                    parent_task,
                    child_task: child.task_id,
                }
            } else {
                Operation::BindDelegate {
                    parent_task,
                    child_task: child.task_id,
                }
            };
            self.owned.durable_state_store.agent_lifecycle(operation)?;
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

    fn is_cross_room_spawn_prompt(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> bool {
        let (Some((actor, _)), Some((caller, LocalDaemonRequest::SpawnAgent(spawn)))) =
            (&self.room_provider_origin, &self.room_request_origin)
        else {
            return false;
        };
        // The admitted dispatch and the persisted parent edge establish authority independently
        // of the prompt author's display name.
        actor == caller
            && spawn.session_id == prepared.session_id
            && spawn.initial_task.is_some()
            && self
                .owned
                .agent_store
                .get_agent(prepared.prompt.target_agent_id())
                .is_ok_and(|child| {
                    child.parent_agent_id() == Some(actor.as_str())
                        && child.parent_room_id().is_some()
                })
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
        let session = self.session_store.get_session(room)?;
        for parent in self.durable_state_store.agent_tasks(Some(room), None)? {
            if parent.owner_user_id != child.owner_user_id {
                continue;
            }
            let mut matches = false;
            for obligation in parent
                .obligations
                .iter()
                .filter(|o| o.tracks_workflow_run() && o.dispatch_state == "accepted")
            {
                if let Some(resource) = obligation.resource_id.as_deref() {
                    if resource == workflow_run
                        || (resource.starts_with("gate_")
                            && self
                                .agent_workflow_source(&session, resource)?
                                .is_some_and(|run| run.id() == workflow_run))
                    {
                        matches = true;
                        break;
                    }
                }
            }
            if matches {
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
