//! MP-08 / MP-09 / MP-10 / MP-11 A02: exact task cancellation via normal runtime paths.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, Operation};
impl KernelRuntimeOwnedState {
    /// MP-08 / MP-09 / MP-10 / MP-11 A10/G03: the worker has settled this
    /// exact prompt; cancel its home task before promoting a queued successor.
    #[allow(dead_code, reason = "called by leased-agent settlement")]
    pub(super) fn cancel_settled_remote_prompt_task(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<(), DaemonError> {
        let owner = self
            .session_store
            .get_session(room)?
            .owner_user_id()
            .to_string();
        let task = self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .into_iter()
            .find(|task| {
                task.prompt_id == prompt || task.pending_prompt_id.as_deref() == Some(prompt)
            });
        if let Some(task) = task {
            self.durable_state_store
                .agent_lifecycle(Operation::CancelTask {
                    task: task.task_id,
                    owner,
                    revision: task.revision,
                })?;
        }
        Ok(())
    }

    /// Read current resource state: task cancellation is intent, not physical
    /// settlement. A matched process can outlive its satisfied obligation.
    #[allow(dead_code, reason = "called by leased-agent settlement")]
    pub(super) fn agent_task_resources_unsettled(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<bool, DaemonError> {
        let current = self
            .durable_state_store
            .agent_tasks(Some(&task.room_id), Some(&task.agent_id))?
            .into_iter()
            .find(|t| t.task_id == task.task_id)
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::error("task resource authority unavailable")
            })?;
        let session = self.session_store.get_session(&task.room_id)?;
        Ok(current.obligations.iter().any(|o| o.status == "open")
            || self
                .durable_state_store
                .agent_wakes(Some(&task.room_id), Some(&task.agent_id))?
                .iter()
                .any(|w| {
                    w.task_id == task.task_id
                        && matches!(
                            w.state.as_str(),
                            "starting" | "running" | "cancelling" | "scheduled"
                        )
                })
            || self
                .prompt_state_owner
                .active_prompt_for_agent(&session, &task.agent_id)
                .is_some_and(|p| {
                    p.id() == current.prompt_id
                        || current.pending_prompt_id.as_deref() == Some(p.id())
                })
            || session
                .queued_prompts_for_agent(&task.agent_id)
                .is_some_and(|q| {
                    q.iter().any(|p| {
                        p.id() == current.prompt_id
                            || current.pending_prompt_id.as_deref() == Some(p.id())
                    })
                }))
    }
}
impl KernelRuntimeState {
    pub(super) async fn cancel_agent_task_resources(
        &self,
        root: &AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        let task = root;
        let mut first_error = None;
        let session = self.owned.session_store.get_session(&task.room_id)?;
        let active = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &task.agent_id)
            .filter(|p| {
                p.id() == task.prompt_id || task.pending_prompt_id.as_deref() == Some(p.id())
            });
        if let Some(prompt) = active {
            let cancellation = Box::pin(self.cancel_remote_agent_prompt_if_matches(
                &task.room_id,
                &task.agent_id,
                prompt.source_attachment_id(),
                Some(prompt.id()),
                None,
            ))
            .await;
            match cancellation {
                Err(error) => {
                    first_error.get_or_insert(error);
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    if let Some(cancellation) = self.owned.cancel_local_prompt_if_matches(
                        &task.room_id,
                        &task.agent_id,
                        prompt.source_attachment_id(),
                        Some(prompt.id()),
                    )? {
                        if let Some(dispatch) = cancellation.dispatch {
                            self.spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
                        }
                    }
                }
            }
        }
        if let Some(queued) = session.queued_prompts_for_agent(&task.agent_id) {
            for prompt in queued.iter().filter(|p| {
                p.id() == task.prompt_id || task.pending_prompt_id.as_deref() == Some(p.id())
            }) {
                if let Err(error) = self.owned.cancel_queued_prompt(
                    &task.room_id,
                    &task.agent_id,
                    prompt.source_attachment_id(),
                    prompt.id(),
                ) {
                    first_error.get_or_insert(error);
                }
            }
        }
        // A source may have satisfied its obligation while it still runs.
        // Cancellation owns that resource too.
        for wake in self
            .owned
            .durable_state_store
            .agent_wakes(Some(&task.room_id), Some(&task.agent_id))?
            .into_iter()
            .filter(|w| {
                w.task_id == task.task_id
                    && matches!(
                        w.state.as_str(),
                        "scheduled" | "starting" | "running" | "cancelling"
                    )
            })
        {
            if let Err(error) = self.cancel_wake_source(&wake, &task.task_id, None) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
