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
            let initiator = task.recorded_cancel_initiator();
            self.durable_state_store
                .agent_lifecycle(initiator.cancel_task(task.task_id, owner, task.revision))?;
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
            .agent_task(&task.task_id)?
            .filter(|current| current.room_id == task.room_id && current.agent_id == task.agent_id)
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
    pub(super) fn agent_cancellation_response(
        &self,
        room: &str,
        task_id: &str,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        self.owned.session_snapshot(room)?;
        self.owned
            .terminal_stream
            .notify_terminal_projection_change(room);
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), None)?
            .into_iter()
            .find(|task| task.task_id == task_id)
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::error(
                    "cancelled task projection unavailable",
                )
            })?;
        Ok(crate::local::LocalDaemonResponse::AgentTaskUpdated {
            session_id: room.into(),
            task: Box::new(task),
        })
    }

    /// Stop still owns parked work when its provider turn has already ended.
    pub(crate) async fn cancel_parked_agent_task(
        &self,
        request: &crate::local::CancelActivePromptRequest,
        agent: &str,
    ) -> Result<Option<crate::local::LocalDaemonResponse>, DaemonError> {
        let session = self.owned.session_store.get_session(&request.session_id)?;
        self.owned
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
        // AgentRuntime already checked the caller against the agent owner,
        // just as it does before cancelling an active prompt.
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
        let (task_id, owner, revision) =
            (task.task_id, session.owner_user_id().into(), task.revision);
        let cancel = self
            .owned
            .cancel_initiator(&request.attachment_id)
            .cancel_task(task_id, owner, revision);
        let Outcome::Task(cancelled) = self.owned.durable_state_store.agent_lifecycle(cancel)?
        else {
            unreachable!()
        };
        self.retract_stale_task_owner_interactions(session.id())
            .await;
        Box::pin(self.cancel_agent_task_resources(&cancelled)).await?;
        self.agent_cancellation_response(session.id(), &cancelled.task_id)
            .map(Some)
    }

    pub(super) async fn cancel_agent_task_resources(
        &self,
        root: &AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        let answer = self.owned.public_agent_task_answer(root)?;
        self.cancel_agent_task_resources_answering(root, answer)
            .await
    }

    /// `root_answer` is what the root task's waiters read when it ends, such as
    /// the outcome of the failed turn that cancelled it.
    pub(super) async fn cancel_agent_task_resources_answering(
        &self,
        root: &AgentTaskExecution,
        root_answer: Option<serde_json::Value>,
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
                            || child.parent_agent_id() != Some(&task.agent_id)
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
                        let mut resolved_resource = resource.clone();
                        if resource.starts_with("gate_") {
                            if let Some((entry, creator)) = self
                                .owned
                                .durable_state_store
                                .gate_task_entry(session.id(), resource)?
                            {
                                if !entry.disposition.is_terminal() {
                                    if creator.as_deref() != Some(task.agent_id.as_str()) {
                                        first_error.get_or_insert(
                                            crate::durable_state::agent_lifecycle::error(
                                                "gate cancellation creator binding changed",
                                            ),
                                        );
                                        continue;
                                    }
                                    self.owned.durable_state_store.gate(
                                        crate::durable_state::workflow_gate::Operation::Cancel {
                                            owner: session.owner_user_id().into(),
                                            id: resource.clone(),
                                            reason: "creating task cancelled".into(),
                                        },
                                    )?;
                                    continue;
                                }
                                if let Some(run) =
                                    self.owned.agent_workflow_source(&session, resource)?
                                {
                                    resolved_resource = run.id().into();
                                } else if let Some(id) = entry.queued_prompt_id {
                                    if session.workflow_queued_prompts().iter().any(|q| {
                                        q.id() == id
                                            && q.created_by_agent_id()
                                                == Some(task.agent_id.as_str())
                                    }) {
                                        if let Err(error) =
                                            self.owned.workflow_remove_queued_prompt(
                                                crate::local::RemoveQueuedWorkflowPromptRequest {
                                                    session_id: task.room_id.clone(),
                                                    queue_item_ref: id,
                                                },
                                            )
                                        {
                                            first_error.get_or_insert(error);
                                        }
                                    }
                                    continue;
                                } else {
                                    continue;
                                }
                            }
                        }
                        if let Some(run) = self
                            .owned
                            .agent_workflow_source(&session, &resolved_resource)?
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
                                        workflow_run_ref: resolved_resource,
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
                        public_answer: (task.task_id == root.task_id)
                            .then(|| root_answer.clone())
                            .flatten(),
                        room: task.room_id.clone(),
                        source: task.task_id.clone(),
                        occurrence: format!("task-terminal-{}", task.task_id),
                        success: false,
                    })?;
                // A task cancelled from outside (a deadline, the owner, an archive) ends like any other:
                // the parent hears of it.
                if task.task_id == root.task_id {
                    if let Err(error) = self
                        .announce_child_finished(&task, None, root_answer.as_ref())
                        .await
                    {
                        tracing::warn!(%error, agent_id=%task.agent_id,
                            "child_finished retained for the inbox sweep");
                    }
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod growth_tests {
    use super::*;

    // MP-08 / MP-09 / MP-10 / MP-11 KR-06: every CompletePrompt checks this
    // task's resources. Reading all of its agent's finished turns here held
    // the shared reader connection longer on each turn, delaying state reads.
    #[tokio::test]
    async fn task_resource_checks_do_not_decode_the_finished_turn_history() {
        let state = crate::test_support::allocation::runtime_with_rooms(1);
        let room = state.owned.session_store.list_all_sessions().pop().unwrap();
        let agent = state
            .owned
            .agent_store
            .get_session_agents(room.id())
            .pop()
            .unwrap();
        let Outcome::Task(task) = state
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Begin {
                owner: room.owner_user_id().into(),
                room: room.id().into(),
                agent: agent.id().into(),
                prompt: "current".into(),
                run: None,
                now: 1,
            })
            .unwrap()
        else {
            panic!("task should start");
        };
        let mut db = rusqlite::Connection::open(state.owned.durable_state_store.path()).unwrap();
        let mut allocations = Vec::new();
        let mut next = 0;
        for count in [10, 500] {
            let tx = db.transaction().unwrap();
            while next < count {
                let mut finished = task.clone();
                finished.task_id = format!("finished-{next}");
                finished.prompt_id = finished.task_id.clone();
                finished.state = ExecutionState::Done;
                tx.execute("INSERT INTO agent_tasks(task_id,room_id,agent_id,prompt_id,payload) VALUES(?1,?2,?3,?4,?5)", rusqlite::params![finished.task_id, finished.room_id, finished.agent_id, finished.prompt_id, serde_json::to_string(&finished).unwrap()]).unwrap();
                next += 1;
            }
            tx.commit().unwrap();
            let (unsettled, measured) = crate::test_support::allocation::measure(|| {
                state.owned.agent_task_resources_unsettled(&task)
            });
            assert!(!unsettled.unwrap());
            allocations.push(measured.allocated);
        }
        assert!(
            allocations[1] <= allocations[0] + 4096,
            "checking one task allocated {} bytes after 10 turns and {} after 500",
            allocations[0],
            allocations[1]
        );
    }
    #[tokio::test]
    async fn session_state_reads_do_not_clone_finished_provider_runs() {
        let state = crate::test_support::allocation::runtime_with_rooms(1);
        let room = state.owned.session_store.list_all_sessions().pop().unwrap();
        let agent = state
            .owned
            .agent_store
            .get_session_agents(room.id())
            .pop()
            .unwrap();
        let mut allocations = Vec::new();
        let mut next = 0;
        for count in [10, 500] {
            while next < count {
                let mut run =
                    crate::provider::RuntimeProviderRun::from_control_capability_inference(
                        format!("finished-run-{next}"),
                        room.id().into(),
                        Some(agent.id().into()),
                        "dev-stub".into(),
                    );
                run.mark_ended();
                state.owned.provider_store.write().insert_run_for_test(run);
                next += 1;
            }
            let (response, measured) = crate::test_support::allocation::measure(|| {
                state.session_state_response(crate::local::GetSessionStateRequest {
                    session_id: room.id().into(),
                })
            });
            assert!(response.is_ok());
            allocations.push(measured.allocated);
        }
        eprintln!(
            "GetSessionState provider history allocated10={} allocated500={}",
            allocations[0], allocations[1]
        );
        assert!(
            allocations[1] <= allocations[0] + 4096,
            "one session state read allocated {} bytes after10 provider runs and {} after500",
            allocations[0],
            allocations[1]
        );
    }
}
