//! MP-08 / MP-09 / MP-10 / MP-11 A02: exact task cancellation via normal runtime paths.
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, ExecutionState, Operation, Outcome,
};
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
                    _ => {}
                }
            }
            if !physical_pending {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::SourceOutcome {
                        public_answer: None,
                        room: task.room_id.clone(),
                        source: task.task_id.clone(),
                        occurrence: format!("task-terminal-{}", task.task_id),
                        success: false,
                    })?;
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
