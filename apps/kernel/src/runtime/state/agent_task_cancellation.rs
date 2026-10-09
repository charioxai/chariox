//! MP-08 / MP-09 / MP-10 / MP-11 A02: exact task cancellation via normal runtime paths.
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, ExecutionState, Operation, Outcome,
};
impl KernelRuntimeOwnedState {
    /// Read current resource state: task cancellation is intent, not physical
    /// settlement. A matched process can outlive its satisfied obligation.
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
        let mut pending = vec![root.clone()];
        let mut seen = BTreeSet::new();
        let mut first_error = None;
        while let Some(task) = pending.pop() {
            if !seen.insert(task.task_id.clone()) {
                continue;
            }
            let session = self.owned.session_store.get_session(&task.room_id)?;
            let active = self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &task.agent_id)
                .filter(|p| p.id() == task.prompt_id);
            let mut physical_pending = false;
            if let Some(prompt) = active {
                physical_pending = true;
                if let Some(cancellation) = self.owned.cancel_local_prompt_if_matches(
                    &task.room_id,
                    &task.agent_id,
                    prompt.source_attachment_id(),
                    Some(prompt.id()),
                )? {
                    if let Some(dispatch) = cancellation.dispatch {
                        self.spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
                    }
                } else {
                    first_error.get_or_insert_with(|| {
                        crate::durable_state::agent_lifecycle::error(
                            "leased cancellation requires PR10 exact task/lease settlement",
                        )
                    });
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
            for obligation in task
                .obligations
                .iter()
                .filter(|o| o.dispatch_state == "cancel_requested")
            {
                let Some(resource) = &obligation.resource_id else {
                    continue;
                };
                match obligation.kind.as_str() {
                    "delegate" => {
                        let child = match self.owned.agent_store.get_agent(resource) {
                            Ok(child) => child,
                            Err(_) => {
                                self.owned.durable_state_store.agent_lifecycle(
                                    Operation::SourceOutcome {
                                        public_answer: None,
                                        room: task.room_id.clone(),
                                        source: obligation
                                            .completion_source()
                                            .unwrap_or(resource)
                                            .into(),
                                        occurrence: format!("cancel-missing-{}", obligation.id),
                                        success: false,
                                        now: crate::session::unix_epoch_ms(),
                                    },
                                )?;
                                continue;
                            }
                        };
                        if child.session_id() != task.room_id
                            || child.owner_user_id() != session.owner_user_id()
                            || child.spawned_by_agent_id() != Some(&task.agent_id)
                        {
                            first_error.get_or_insert_with(|| {
                                crate::durable_state::agent_lifecycle::error(
                                    "delegate cancellation creator binding changed",
                                )
                            });
                            continue;
                        }
                        if let Some(id) = &obligation.completion_task_id {
                            if let Some(child_task) = self
                                .owned
                                .durable_state_store
                                .agent_tasks(Some(&task.room_id), Some(resource))?
                                .into_iter()
                                .find(|t| &t.task_id == id)
                            {
                                if child_task.state != ExecutionState::Done {
                                    if let Outcome::Task(cancelled) = self
                                        .owned
                                        .durable_state_store
                                        .agent_lifecycle(Operation::CancelTask {
                                            task: id.clone(),
                                            owner: session.owner_user_id().into(),
                                            revision: child_task.revision,
                                        })?
                                    {
                                        pending.push(cancelled);
                                    }
                                }
                            }
                        } else if self
                            .owned
                            .durable_state_store
                            .agent_tasks(Some(&task.room_id), Some(resource))?
                            .is_empty()
                            && self
                                .owned
                                .prompt_state_owner
                                .active_prompt_for_agent(&session, resource)
                                .is_none()
                            && session
                                .queued_prompts_for_agent(resource)
                                .is_none_or(|q| q.is_empty())
                        {
                            self.owned.durable_state_store.agent_lifecycle(
                                Operation::SourceOutcome {
                                    public_answer: None,
                                    room: task.room_id.clone(),
                                    source: resource.clone(),
                                    occurrence: format!("cancel-unused-{}", obligation.id),
                                    success: false,
                                    now: crate::session::unix_epoch_ms(),
                                },
                            )?;
                        }
                    }
                    _ if obligation.tracks_workflow_run() => {
                        if let Some(run) =
                            session.workflow_runs().iter().find(|r| r.id() == resource)
                        {
                            if run.created_by_agent_id() != Some(&task.agent_id) {
                                first_error.get_or_insert(
                                    crate::durable_state::agent_lifecycle::error(
                                        "workflow cancellation creator binding changed",
                                    ),
                                );
                                continue;
                            }
                            let (result, _) = self
                                .execute_workflow_cancel_run_request(
                                    crate::local::CancelWorkflowRunRequest {
                                        session_id: task.room_id.clone(),
                                        workflow_run_ref: resource.clone(),
                                    },
                                )
                                .await;
                            if let Err(error) = result {
                                first_error.get_or_insert(error);
                            }
                        }
                    }
                    "timer" | "process" => {
                        if let Err(error) =
                            self.owned
                                .durable_state_store
                                .agent_lifecycle(Operation::CancelWake {
                                    id: resource.clone(),
                                    task: task.task_id.clone(),
                                    prompt: None,
                                })
                        {
                            first_error.get_or_insert(error);
                            continue;
                        }
                        self.owned.agent_wakes.processes.terminate(resource);
                    }
                    _ => {}
                }
            }
            // A matched output may have satisfied its obligation while the
            // process still runs. Cancellation owns that resource too.
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
                if let Err(error) =
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::CancelWake {
                            id: wake.id.clone(),
                            task: task.task_id.clone(),
                            prompt: None,
                        })
                {
                    first_error.get_or_insert(error);
                } else {
                    self.owned.agent_wakes.processes.terminate(&wake.id);
                }
            }
            if !physical_pending && !self.owned.agent_task_resources_unsettled(&task)? {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::SourceOutcome {
                        public_answer: None,
                        room: task.room_id.clone(),
                        source: task.task_id.clone(),
                        occurrence: format!("task-terminal-{}", task.task_id),
                        success: false,
                        now: crate::session::unix_epoch_ms(),
                    })?;
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
