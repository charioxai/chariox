//! MP-09 A03: close wake ownership before removing an agent or Room.
use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};

impl KernelRuntimeOwnedState {
    pub(super) fn cancel_owned_agent_wakes(
        &self,
        room: &str,
        agent: Option<&str>,
    ) -> Result<(), DaemonError> {
        let store = &self.durable_state_store;
        let owner = self
            .session_store
            .get_session(room)?
            .owner_user_id()
            .to_string();
        for task in store.agent_tasks(Some(room), agent)? {
            if !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled) {
                store.agent_lifecycle(Operation::CancelTask {
                    task: task.task_id,
                    owner: owner.clone(),
                    revision: task.revision,
                })?;
            }
        }
        for wake in store.agent_wakes(Some(room), agent)? {
            if matches!(
                wake.state.as_str(),
                "scheduled" | "starting" | "running" | "cancelling"
            ) {
                store.agent_lifecycle(Operation::CancelWake {
                    id: wake.id.clone(),
                    task: wake.task_id,
                    prompt: None,
                })?;
                if wake.kind == "process" {
                    self.agent_wakes.processes.terminate(&wake.id);
                }
            }
        }
        Ok(())
    }
}
