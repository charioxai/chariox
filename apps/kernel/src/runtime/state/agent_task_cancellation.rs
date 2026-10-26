//! MP-08 / MP-09 / MP-10 / MP-11 A02: exact task cancellation via normal runtime paths.
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, ExecutionState, Operation, Outcome,
};
impl KernelRuntimeOwnedState {
    /// A01-S01: removed queued prompts never reached a provider. A wake or
    /// message one carried goes back to the inbox as a refused attempt: a
    /// terminal task then expires it and a live receiver gets it later, so it
    /// never strands as an unconfirmed delivery that asks the owner.
    pub(super) fn release_unsent_agent_events(
        &self,
        room: &str,
        prompts: &[crate::session::PromptQueueItem],
    ) {
        for prompt in prompts {
            let agent = prompt.target_agent_id();
            let event = match self.durable_state_store.agent_event_for_prompt(
                room,
                agent,
                prompt.id(),
            ) {
                Ok(Some(event)) if event.state == "submitting" => event,
                Ok(_) => continue,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.agent_task",
                        "unsent agent event lookup failed",
                        serde_json::json!({"session_id": room, "agent_id": agent, "prompt_id": prompt.id(), "error": error.to_string()}),
                    );
                    continue;
                }
            };
            if let Err(error) = self
                .durable_state_store
                .agent_lifecycle(Operation::Receipt {
                    room: room.into(),
                    agent: agent.into(),
                    sequence: event.sequence,
                    state: "rejected".into(),
                    now: crate::session::unix_epoch_ms(),
                })
            {
                crate::logging::warn_with_fields(
                    "daemon.agent_task",
                    "unsent agent event release failed",
                    serde_json::json!({"session_id": room, "agent_id": agent, "sequence": event.sequence, "error": error.to_string()}),
                );
            }
        }
    }
    /// MP-08 / MP-09 / MP-10 / MP-11 A10/G03: the worker has settled this
    /// exact prompt; cancel its home task before promoting a queued successor.
    pub(super) fn cancel_settled_remote_prompt_task(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
    ) -> Result<(), DaemonError> {
        if !self.config_projection.snapshot().room_agent_tools {
            return Ok(());
        }
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
    /// Stop still owns parked work when its provider turn has already ended.
    pub(crate) async fn cancel_parked_agent_task(
        &self,
        request: &crate::local::CancelActivePromptRequest,
        agent: &str,
    ) -> Result<Option<crate::local::LocalDaemonResponse>, DaemonError> {
        let session = self.owned.session_store.get_session(&request.session_id)?;
        let attachment = self
            .owned
            .ensure_attachment_in_session(session.id(), &request.attachment_id)?;
        // A native prompt-specific Stop must never cancel a different task.
        if request.expected_prompt_id.is_some()
            || self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent)
                .is_some()
            || !self.owned.config_projection.snapshot().room_agent_tools
        {
            return Ok(None);
        }
        if attachment.owner_user_id() != session.owner_user_id() {
            return Err(crate::durable_state::agent_lifecycle::error(
                "only the room owner can stop a parked task",
            ));
        }
        let tasks = self
            .owned
            .durable_state_store
            .agent_tasks(Some(session.id()), Some(agent))?;
        let mut tasks = tasks.into_iter().filter(|task| {
            !matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
                || task
                    .obligations
                    .iter()
                    .any(|obligation| obligation.status == "open")
        });
        let Some(task) = tasks.next() else {
            return Ok(None);
        };
        if tasks.next().is_some() {
            return Err(crate::durable_state::agent_lifecycle::error("multiple unfinished tasks on this agent; cancel the intended task from its task controls"));
        }
        let Outcome::Task(cancelled) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::CancelTask {
                    task: task.task_id,
                    owner: session.owner_user_id().into(),
                    revision: task.revision,
                })?
        else {
            unreachable!()
        };
        self.retract_stale_task_owner_interactions(session.id(), std::slice::from_ref(&cancelled))
            .await;
        Box::pin(self.cancel_agent_task_resources(&cancelled)).await?;
        let snapshot = self.owned.session_snapshot(session.id())?;
        self.owned
            .terminal_stream
            .notify_terminal_projection_change(session.id());
        let task = snapshot
            .agent_tasks()
            .iter()
            .find(|task| task.task_id == cancelled.task_id)
            .cloned()
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::error(
                    "cancelled task projection unavailable",
                )
            })?;
        Ok(Some(crate::local::LocalDaemonResponse::AgentTaskUpdated {
            session_id: session.id().into(),
            task: Box::new(task),
        }))
    }

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
                .filter(|p| {
                    p.id() == task.prompt_id || task.pending_prompt_id.as_deref() == Some(p.id())
                });
            let mut physical_pending = false;
            if let Some(prompt) = active {
                physical_pending = true;
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
                                self.spawn_prompt_abort(
                                    dispatch,
                                    self.provider_runtime_lanes.clone(),
                                );
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
                                        now: crate::session::unix_epoch_ms(),
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
                                    now: crate::session::unix_epoch_ms(),
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
                    "timer" | "process" => {
                        let cancelled = match self.owned.durable_state_store.agent_wake(resource) {
                            Ok(Some(wake)) => {
                                self.cancel_agent_wake(&wake, &task.task_id, None).await
                            }
                            Ok(None) => Err(crate::durable_state::agent_lifecycle::error(
                                "wake unavailable",
                            )),
                            Err(error) => Err(error),
                        };
                        if let Err(error) = cancelled {
                            first_error.get_or_insert(error);
                        }
                    }
                    // MP-11 A07: cancellation withdraws the owner popup; a
                    // hand-off already gone settles as lost.
                    super::HANDOFF_OBLIGATION_KIND => {
                        if !self.withdraw_handoff(&task.room_id, resource) {
                            self.owned.durable_state_store.agent_lifecycle(
                                Operation::SourceOutcome {
                                    public_answer: None,
                                    room: task.room_id.clone(),
                                    source: resource.clone(),
                                    occurrence: format!("cancel-missing-{}", obligation.id),
                                    success: false,
                                    now: crate::session::unix_epoch_ms(),
                                },
                            )?;
                        }
                    }
                    _ => {}
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
                if let Err(error) = self.cancel_agent_wake(&wake, &task.task_id, None).await {
                    first_error.get_or_insert(error);
                }
            }
            if !physical_pending && !self.owned.agent_task_resources_unsettled(&task)? {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::SourceOutcome {
                        now: crate::session::unix_epoch_ms(),
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
