//! MP-08 / MP-09 / MP-10 / MP-11 A02: provider/interaction wiring for the ledger.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, ExecutionState, Operation, Outcome,
};

impl KernelRuntimeOwnedState {
    /// Returns true when this admission created the task.
    pub(super) fn admit_agent_task(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> Result<bool, DaemonError> {
        if !self.config_projection.snapshot().room_agent_tools {
            return Ok(false);
        }
        let prompt = prepared.prompt.id();
        let existed = self
            .durable_state_store
            .agent_tasks(
                Some(&prepared.session_id),
                Some(prepared.prompt.target_agent_id()),
            )?
            .iter()
            .any(|t| t.prompt_id == prompt || t.pending_prompt_id.as_deref() == Some(prompt));
        let Outcome::Task(task) = self.durable_state_store.agent_lifecycle(Operation::Begin {
            owner: self
                .session_store
                .get_session(&prepared.session_id)?
                .owner_user_id()
                .into(),
            room: prepared.session_id.clone(),
            agent: prepared.prompt.target_agent_id().into(),
            prompt: prepared.prompt.id().into(),
            run: None,
            now: crate::session::unix_epoch_ms(),
        })?
        else {
            unreachable!()
        };
        self.bind_agent_workflow_task(&task, &prepared.prompt)?;
        Ok(!existed)
    }
    pub(super) fn withdraw_agent_task(&self, prompt: &str) -> Result<(), DaemonError> {
        self.durable_state_store
            .agent_lifecycle(Operation::Withdraw {
                task: prompt.into(),
            })?;
        Ok(())
    }
    pub(super) fn settle_agent_task(
        &self,
        room: &str,
        agent: &str,
        prompt: &crate::session::PromptQueueItem,
        run: &str,
        cancelled: bool,
    ) -> Result<Option<(AgentTaskExecution, bool)>, DaemonError> {
        if !self.config_projection.snapshot().room_agent_tools {
            return Ok(None);
        }
        if !self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .iter()
            .any(|t| t.prompt_id == prompt.id())
        {
            self.durable_state_store.agent_lifecycle(Operation::Begin {
                owner: self.session_store.get_session(room)?.owner_user_id().into(),
                room: room.into(),
                agent: agent.into(),
                prompt: prompt.id().into(),
                run: Some(run.into()),
                now: crate::session::unix_epoch_ms(),
            })?;
        }
        self.reconcile_workflow_delegation(room, agent, prompt)?;
        let entries = self
            .operational_history_store
            .load_session_history_entries(room, Some(agent))?;
        let has_answer =
            super::agent_task_projection::task_public_outputs(&entries, prompt.id(), Some(run))
                .iter()
                .any(|text| !text.trim().is_empty());
        match self
            .durable_state_store
            .agent_lifecycle(Operation::Settle {
                room: room.into(),
                agent: agent.into(),
                prompt: prompt.id().into(),
                run: run.into(),
                has_answer,
                cancelled,
                now: crate::session::unix_epoch_ms(),
            })? {
            Outcome::Settled { task, correction } => Ok(Some((task, correction))),
            _ => unreachable!(),
        }
    }
}
impl KernelRuntimeState {
    pub(in crate::runtime::state) async fn send_durable_agent_message(
        &self,
        session: &crate::session::RuntimeSession,
        sender: &crate::agent::AgentInstance,
        args: crate::transport::runtime_tools::SendAgentMessageArgs,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if args.message.trim().is_empty() && args.attachments.is_empty() {
            return Err(ledger::error("message is empty"));
        }
        let (actor, origin_run) = self.room_provider_origin.as_ref().ok_or_else(|| {
            ledger::error("durable messages require authenticated provider origin")
        })?;
        if actor != sender.id()
            || self
                .owned
                .provider_store
                .get_run_for_agent(session.id(), sender.id())
                .is_none_or(|run| run.id() != origin_run)
        {
            return Err(ledger::error("stale provider message authority"));
        }
        let current = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(session, sender.id())
            .ok_or_else(|| ledger::error("sender has no running turn"))?;
        if current.id() != args.origin_prompt_id
            || current.status() != crate::session::PromptStatus::Running
        {
            return Err(ledger::error("stale sender turn"));
        }
        let agents = self.session_agents(session.id());
        let target = crate::runtime::room_tool_admission::resolve_agent(
            &agents,
            session.id(),
            args.agent.trim_start_matches('@'),
        )?;
        if sender.id() == target.id() {
            return Err(ledger::error("self-trigger message denied"));
        }
        // Room admission precedes resource lookup; no cross-room discovery.
        let id = args
            .idempotency_key
            .unwrap_or_else(|| format!("{:032x}", rand::random::<u128>()));
        if id.trim().is_empty() || id.len() > 128 {
            return Err(ledger::error("invalid message occurrence"));
        }
        let mut event = ledger::occurrence(
            session.id(),
            target.id(),
            sender.id(),
            &format!("{}:{id}", current.id()),
            "message",
            serde_json::json!({"message":args.message,"sender_agent_id":sender.id(),"sender_prompt_id":current.id(),"no_reply":!args.reply_requested,"attachments":args.attachments}),
        );
        event.urgent = args.urgent;
        event.reply_requested = args.reply_requested;
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::Begin {
                owner: sender.owner_user_id().into(),
                room: session.id().into(),
                agent: sender.id().into(),
                prompt: current.id().into(),
                run: self
                    .owned
                    .provider_store
                    .get_run_for_agent(session.id(), sender.id())
                    .map(|r| r.id().into()),
                now: crate::session::unix_epoch_ms(),
            })?;
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(session.id()), Some(sender.id()))?
            .into_iter()
            .find(|t| t.prompt_id == current.id())
            .ok_or_else(|| ledger::error("sender task unavailable"))?;
        let Outcome::Event(event) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Send {
                    task: task.task_id,
                    prompt: current.id().into(),
                    event,
                })?
        else {
            unreachable!()
        };
        self.owned.record_notice_for_agent(
            session.id(),
            None,
            Some(target.id()),
            self.owned
                .attachment_store
                .list_session_attachment_ids(session.id()),
            format!(
                "Agent inbox: event {} from {} ({})",
                event.sequence,
                sender.agent_ref(),
                if args.urgent { "urgent" } else { "next turn" }
            ),
        );
        if let Err(error) = Box::pin(self.deliver_agent_inbox(session.id(), target.id())).await {
            self.owned.record_notice_for_agent(
                session.id(),
                None,
                Some(target.id()),
                self.owned
                    .attachment_store
                    .list_session_attachment_ids(session.id()),
                format!(
                    "Event {} is durable and pending: {}",
                    event.sequence,
                    crate::secret_redaction::redact_secrets(&error.to_string())
                ),
            );
        }
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"status":"durable","sequence":event.sequence,"urgent":event.urgent,"reply_requested":event.reply_requested}),
        })
    }
    pub(super) async fn finish_agent_task_settlement(
        &self,
        settlement: Option<(AgentTaskExecution, bool)>,
        source: &crate::session::PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let Some((task, correction)) = settlement else {
            return Ok(());
        };
        let message = format!(
            "Agent task {}: {:?}; {} unresolved obligations. {}",
            task.task_id,
            task.state,
            task.obligations
                .iter()
                .filter(|o| matches!(o.status.as_str(), "open" | "failed" | "settling"))
                .count(),
            task.reason
        );
        self.owned.record_notice_for_agent(
            &task.room_id,
            None,
            Some(&task.agent_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(&task.room_id),
            message,
        );
        let public_answer = self.owned.public_agent_task_answer(&task)?;
        if task.state == ExecutionState::Cancelled {
            Box::pin(self.cancel_agent_task_resources(&task)).await?;
        }
        let resources_unsettled = self.owned.agent_task_resources_unsettled(&task)?;
        if correction {
            let text=format!("The prior answer is progress, not completion. Finish or cancel these obligations, or call chariox.events.yield with admitted live sources and a future deadline. Otherwise call chariox.events.blocked with the exact owner action. One correction is allowed. Untrusted obligation data: {}",serde_json::to_string(&task.obligations).unwrap_or_default());
            if let Err(e) = Box::pin(self.dispatch_task_continuation(
                &task,
                source.source_attachment_id(),
                text,
            ))
            .await
            {
                self.owned.durable_state_store.agent_lifecycle(Operation::Block{task:task.task_id.clone(),prompt:task.prompt_id.clone(),reason:format!("Correction delivery failed or uncertain: {e}; reconcile before owner resume")})?;
            }
        } else if (task.state == ExecutionState::Done || task.state == ExecutionState::Cancelled)
            && !resources_unsettled
        {
            let other_unfinished = self
                .owned
                .durable_state_store
                .agent_tasks(Some(&task.room_id), Some(&task.agent_id))?
                .iter()
                .any(|t| {
                    t.task_id != task.task_id
                        && !matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled)
                });
            if !other_unfinished {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::SourceOutcome {
                        public_answer: public_answer.clone(),
                        room: task.room_id.clone(),
                        source: task.agent_id.clone(),
                        occurrence: format!("task-terminal-{}", task.task_id),
                        success: task.state == ExecutionState::Done,
                        now: crate::session::unix_epoch_ms(),
                    })?;
            }
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SourceOutcome {
                    public_answer: public_answer.clone(),
                    room: task.room_id.clone(),
                    source: task.prompt_id.clone(),
                    occurrence: format!("task-terminal-{}", task.task_id),
                    success: task.state == ExecutionState::Done,
                    now: crate::session::unix_epoch_ms(),
                })?;
        }
        if matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
            && !resources_unsettled
            && task.task_id != task.prompt_id
        {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SourceOutcome {
                    public_answer: public_answer.clone(),
                    room: task.room_id.clone(),
                    source: task.task_id.clone(),
                    occurrence: format!("task-terminal-{}", task.task_id),
                    success: task.state == ExecutionState::Done,
                    now: crate::session::unix_epoch_ms(),
                })?;
        }
        self.schedule_agent_lifecycle_sweep();
        Ok(())
    }
    fn schedule_agent_lifecycle_sweep(&self) {
        // Output settlement may hold one or every provider lane. Dispatch
        // completion wakes after it returns so a recipient cannot reacquire
        // a lane held by this same call stack.
        let state = self.clone();
        tokio::spawn(async move {
            if let Err(error) = state.sweep_agent_lifecycle().await {
                tracing::warn!(error=%crate::secret_redaction::redact_secrets(&error.to_string()),
                    "MP-08/MP-09/MP-10/MP-11 A02: completion sweep retained for periodic reconciliation");
            }
        });
    }
    pub(super) async fn dispatch_task_continuation(
        &self,
        task: &AgentTaskExecution,
        attachment: &str,
        text: String,
    ) -> Result<(), DaemonError> {
        let id = task
            .pending_prompt_id
            .clone()
            .ok_or_else(|| ledger::error("missing durable continuation intent"))?;
        let workflow = self.owned.agent_workflow_task_context(task)?;
        let workflow_attachment = workflow
            .as_ref()
            .map(|(run, _)| crate::scheduler::runtime::workflow_prompt_source_attachment_id(run));
        let prompt = crate::session::PromptQueueItem::new(
            id.clone(),
            workflow_attachment.as_deref().unwrap_or(attachment),
            &task.agent_id,
            text,
            crate::session::PromptStatus::Queued,
        )
        .with_durable_operation(&id, &format!("task:{}:{}", task.task_id, task.revision));
        let prompt = match workflow {
            Some((run, node)) => prompt.with_workflow_context(run, node),
            None => prompt,
        };
        let mut submission = self
            .submit_prepared_prompt_with_queue_policy(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: task.room_id.clone(),
                    prompt,
                    // Start an idle provider immediately; the shared owner
                    // still queues behind an actually running turn.
                    force_queue: false,
                    refresh_projection: true,
                },
                true,
            )
            .await?;
        if let Some(dispatch) = submission.dispatch.take() {
            self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
        }
        if let Some(dispatch) = submission.remote_dispatch.take() {
            self.spawn_remote_prompt_dispatch(dispatch);
        }
        Ok(())
    }
    pub(super) async fn sweep_agent_lifecycle(&self) -> Result<(), DaemonError> {
        if !self.owned.config_projection.snapshot().room_agent_tools {
            return Ok(());
        }
        let now = crate::session::unix_epoch_ms();
        if let Err(error) = self.sweep_agent_wakes(now).await {
            tracing::warn!(%error, "MP-08/MP-09/MP-10/MP-11 A03: wake dead-man check retained");
        }
        let mut blocked = Vec::new();
        for task in self.owned.durable_state_store.agent_tasks(None, None)? {
            let Ok(session) = self.owned.session_store.get_session(&task.room_id) else {
                continue;
            };
            if task.state == ExecutionState::Cancelled {
                if let Err(error) = Box::pin(self.cancel_agent_task_resources(&task)).await {
                    tracing::warn!(%error,"MP-08/MP-09/MP-10/MP-11 A02: cancellation remains supervised");
                }
            }
            for obligation in task.obligations.iter().filter(|o| o.status == "open") {
                let Some(source) = obligation.resource_id.as_ref() else {
                    continue;
                };
                let outcome = match obligation.kind.as_str() {
                    "delegate" => {
                        let child_task = match &obligation.completion_task_id {
                            Some(id) => self
                                .owned
                                .durable_state_store
                                .agent_tasks(Some(&task.room_id), Some(source))?
                                .into_iter()
                                .find(|t| &t.task_id == id),
                            None => None,
                        };
                        // Provider failure is not settlement of resources still
                        // owned by the exact cancelled child task.
                        match child_task {
                            Some(ref child)
                                if child.state == ExecutionState::Cancelled
                                    && self.owned.agent_task_resources_unsettled(child)? =>
                            {
                                None
                            }
                            _ if !self.owned.agent_store.get_agent(source).is_ok_and(|agent| {
                                agent.state() != crate::agent::AgentState::Error
                            }) =>
                            {
                                Some(false)
                            }
                            Some(child) => match child.state {
                                ExecutionState::Done => Some(true),
                                ExecutionState::Cancelled => Some(false),
                                _ => None,
                            },
                            None => None,
                        }
                    }
                    _ if obligation.tracks_workflow_run()
                        && self
                            .owned
                            .workflow_agent_tasks_unsettled(&task.room_id, source)? =>
                    {
                        None
                    }
                    _ if obligation.tracks_workflow_run() => session
                        .workflow_runs()
                        .iter()
                        .find(|r| r.id() == source)
                        .map(|r| match r.status() {
                            crate::session::WorkflowRunStatus::Completed => Some(true),
                            crate::session::WorkflowRunStatus::Failed
                            | crate::session::WorkflowRunStatus::Stopped => Some(false),
                            _ => None,
                        })
                        .unwrap_or(Some(false)),
                    _ => None,
                };
                if let Some(success) = outcome {
                    let terminal_task = if obligation.kind == "delegate" {
                        self.owned
                            .durable_state_store
                            .agent_tasks(Some(&task.room_id), Some(source))?
                            .into_iter()
                            .find(|t| {
                                obligation.completion_task_id.as_deref() == Some(t.task_id.as_str())
                                    && matches!(
                                        t.state,
                                        ExecutionState::Done | ExecutionState::Cancelled
                                    )
                            })
                    } else {
                        None
                    };
                    let public_answer = terminal_task
                        .as_ref()
                        .map(|t| self.owned.public_agent_task_answer(t))
                        .transpose()?
                        .flatten();
                    let occurrence = terminal_task
                        .as_ref()
                        .map(|t| format!("task-terminal-{}", t.task_id))
                        .unwrap_or_else(|| {
                            if let Some(run) =
                                session.workflow_runs().iter().find(|r| r.id() == source)
                            {
                                format!(
                                    "source-terminal-{source}-{}",
                                    run.completed_at_ms().unwrap_or(0)
                                )
                            } else {
                                format!("source-lost-{source}")
                            }
                        });
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::SourceOutcome {
                            public_answer,
                            room: task.room_id.clone(),
                            source: obligation.completion_source().unwrap_or(source).into(),
                            occurrence,
                            success,
                            now,
                        })?;
                }
            }
            // A queue item is not a live executor. Recover the exact eligible
            // head through normal provider/project admission; do not advance
            // a different (possibly blocked) task on this task's authority.
            if task.state == ExecutionState::Working
                && self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &task.agent_id)
                    .is_none()
                && self
                    .owned
                    .prompt_state_owner
                    .peek_next_queued_prompt(&session, &task.agent_id)
                    .is_some_and(|p| p.id() == task.prompt_id)
                && Box::pin(
                    self.advance_project_queued_prompt_after_settlement(
                        &task.room_id,
                        &task.agent_id,
                    ),
                )
                .await
                .is_some()
            {
                continue;
            }
            let active = self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &task.agent_id)
                .is_some();
            let queued = session
                .queued_prompts_for_agent(&task.agent_id)
                .is_some_and(|q| {
                    q.iter().any(|p| {
                        p.id() == task.prompt_id
                            || task.pending_prompt_id.as_deref() == Some(p.id())
                    })
                });
            if ledger::lacks_live_executor(&task, now, active, queued)
                && !self
                    .owned
                    .durable_state_store
                    .agent_has_live_wake_admission(&task, crate::session::unix_epoch_ms)?
            {
                if let Outcome::Task(task) = self.owned.durable_state_store.agent_lifecycle(Operation::Block{task:task.task_id,prompt:task.prompt_id,reason:"No live provider turn or confirmed wake delivery; owner must reconcile and resume".into()})? {
                    blocked.push(task);
                }
            }
        }
        // Include taskless recipients: their active user/provider turn must
        // not consume the idle-refusal budget or suppress later urgent steering.
        let mut busy_recipients = Vec::new();
        for (room, agent) in self
            .owned
            .durable_state_store
            .agent_pending_inbox_recipients()?
        {
            if let Ok(session) = self.owned.session_store.get_session(&room) {
                if self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &agent)
                    .is_some()
                {
                    busy_recipients.push((room, agent));
                }
            }
        }
        let Outcome::Swept(changed) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Sweep {
                    now,
                    busy_recipients,
                })?
        else {
            unreachable!()
        };
        // Every transition is a notice so attached clients refresh the projection.
        for task in blocked.into_iter().chain(changed) {
            self.owned.record_notice_for_agent(
                &task.room_id,
                None,
                Some(&task.agent_id),
                self.owned
                    .attachment_store
                    .list_session_attachment_ids(&task.room_id),
                if task.state == ExecutionState::Blocked {
                    format!("Blocked: {}", task.reason)
                } else {
                    format!(
                        "Long wait: task {} has made no progress for 15 minutes",
                        task.task_id
                    )
                },
            );
        }
        let tasks = self.owned.durable_state_store.agent_tasks(None, None)?;
        let mut rooms = BTreeSet::new();
        for task in tasks.clone() {
            // Recovery retains unavailable Rooms; their tasks must not halt live supervision.
            if self.owned.session_store.get_session(&task.room_id).is_err() {
                continue;
            }
            if task.state == ExecutionState::Blocked {
                if self
                    .ensure_task_owner_interaction(task.clone())
                    .await
                    .is_err()
                {
                    tracing::warn!(room_id=%task.room_id, agent_id=%task.agent_id, "MP-08/MP-09/MP-10/MP-11 A02: owner projection unavailable; other recipients continue");
                }
            }
            if rooms.insert(task.room_id.clone()) {
                self.retract_stale_task_owner_interactions(&task.room_id, &tasks)
                    .await;
            }
        }
        // A rejected cold submission withdraws its fresh task. The inbox still
        // owns the work and must retry even when this recipient has no tasks.
        for (room, agent) in self
            .owned
            .durable_state_store
            .agent_pending_inbox_recipients()?
        {
            if self.owned.session_store.get_session(&room).is_err() {
                continue;
            }
            if let Err(error) = Box::pin(self.deliver_agent_inbox(&room, &agent)).await {
                tracing::warn!(%error, room_id=%room, agent_id=%agent, "MP-08/MP-10/MP-11 A02: recipient delivery retained; other recipients continue");
            }
            let _ = self.owned.session_snapshot(&room);
        }
        Ok(())
    }
    /// A task that left Blocked (for example cancelled with its parent) or
    /// re-blocked at a later revision withdraws its older owner decision.
    async fn retract_stale_task_owner_interactions(
        &self,
        room: &str,
        tasks: &[AgentTaskExecution],
    ) {
        let Ok(session) = self.owned.session_store.get_session(room) else {
            return;
        };
        let stale = session
            .active_interactions()
            .iter()
            .map(|i| i.id())
            .filter(|id| {
                let Some(rest) = id.strip_prefix("task-blocked-") else {
                    return false;
                };
                tasks.iter().any(|t| {
                    rest.strip_prefix(t.task_id.as_str())
                        .and_then(|r| r.strip_prefix('-'))
                        .and_then(|r| r.parse::<u64>().ok())
                        .is_some_and(|revision| {
                            t.state != ExecutionState::Blocked || revision != t.blocked_revision
                        })
                })
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for id in stale {
            let _ = self.timeout_runtime_interaction(room, &id).await;
        }
    }
    pub(super) async fn ensure_task_owner_interaction(
        &self,
        task: AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        let id = format!("task-blocked-{}-{}", task.task_id, task.blocked_revision);
        let session = self.owned.session_store.get_session(&task.room_id)?;
        if let Some(existing) = session.active_interactions().iter().find(|i| i.id() == id) {
            let message=format!("Task {}: {}. {} obligations remain supervised. Progress does not resume this task; resolve the cause, then resume or cancel.",task.task_id,task.reason,task.obligations.iter().filter(|o|matches!(o.status.as_str(),"open"|"settling"|"failed")).count());
            if existing.message() != message {
                let mutation = self.owned.begin_managed_activity_mutation();
                let mut sessions = self.owned.session_store.write();
                let mut latest = sessions.get_session(&task.room_id)?;
                if let Some(current) = latest.remove_active_interaction(&id) {
                    latest.add_active_interaction(current.with_message(message));
                    sessions.restore_session(latest);
                    mutation.record();
                }
                drop(sessions);
                self.owned.session_snapshot(&task.room_id)?;
                self.owned
                    .terminal_stream
                    .notify_terminal_projection_change(&task.room_id);
            }
            return Ok(());
        }
        let interaction=crate::session::RuntimeInteraction::for_kernel_operation(&id,&id,"Agent needs your action",format!("Task {}: {}. {} obligations remain supervised. Resume after resolving the cause or cancel explicitly.",task.task_id,task.reason,task.obligations.iter().filter(|o|o.status!="satisfied"&&o.status!="cancelled").count()),vec![crate::session::RuntimeInteractionChoice::new("cancel","Cancel","cancel",None),crate::session::RuntimeInteractionChoice::new("resume","Resume","resume",None)]).with_timeout_sec(900);
        let rx = self
            .create_kernel_operation_interaction(
                &task.room_id,
                session.owner_user_id(),
                interaction,
            )
            .await?;
        let state = self.clone();
        tokio::spawn(async move {
            if let Ok(answer) = rx.await {
                if answer.status == "timed_out" {
                    return;
                }
                let resume = answer.choice_id.as_deref() == Some("resume");
                state.resolve_agent_task_owner_action(&task, resume).await;
            }
        });
        Ok(())
    }
}
