//! MP-08 / MP-09 / MP-10 / MP-11 A02: provider/interaction wiring for the ledger.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, ExecutionState, Operation, Outcome,
};

impl KernelRuntimeOwnedState {
    pub(super) fn admit_agent_task(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> Result<(), DaemonError> {
        if !self.config_projection.snapshot().room_agent_tools {
            return Ok(());
        }
        self.durable_state_store.agent_lifecycle(Operation::Begin {
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
        let entries = self
            .operational_history_store
            .load_session_history_entries(room, Some(agent))?;
        // Only public assistant output after this exact prompt counts as an answer.
        let has_answer = entries
            .iter()
            .rev()
            .take_while(|e| e.kind != crate::history::SessionHistoryEntryKind::UserPrompt)
            .any(|e| {
                e.kind == crate::history::SessionHistoryEntryKind::ProviderOutput
                    && e.provider_run_id.as_deref() == Some(run)
                    && !e.text.trim().is_empty()
            });
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
        Box::pin(self.deliver_agent_inbox(session.id(), target.id())).await?;
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
        } else if task.state == ExecutionState::Done || task.state == ExecutionState::Cancelled {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SourceOutcome {
                    room: task.room_id.clone(),
                    source: task.agent_id.clone(),
                    occurrence: format!("{}:{}", task.task_id, task.revision),
                    success: task.state == ExecutionState::Done,
                    now: crate::session::unix_epoch_ms(),
                })?;
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SourceOutcome {
                    room: task.room_id.clone(),
                    source: task.prompt_id.clone(),
                    occurrence: format!("{}:{}", task.task_id, task.revision),
                    success: task.state == ExecutionState::Done,
                    now: crate::session::unix_epoch_ms(),
                })?;
        }
        if matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
            && task.task_id != task.prompt_id
        {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SourceOutcome {
                    room: task.room_id.clone(),
                    source: task.task_id.clone(),
                    occurrence: format!("{}:{}", task.task_id, task.revision),
                    success: task.state == ExecutionState::Done,
                    now: crate::session::unix_epoch_ms(),
                })?;
        }
        Box::pin(self.sweep_agent_lifecycle()).await?;
        Ok(())
    }
    async fn dispatch_task_continuation(
        &self,
        task: &AgentTaskExecution,
        attachment: &str,
        text: String,
    ) -> Result<(), DaemonError> {
        let id = task
            .pending_prompt_id
            .clone()
            .ok_or_else(|| ledger::error("missing durable continuation intent"))?;
        let prompt = crate::session::PromptQueueItem::new(
            id.clone(),
            attachment,
            &task.agent_id,
            text,
            crate::session::PromptStatus::Queued,
        )
        .with_durable_operation(&id, &format!("task:{}:{}", task.task_id, task.revision));
        let mut submission = self
            .submit_prepared_prompt_with_queue_policy(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: task.room_id.clone(),
                    prompt,
                    force_queue: true,
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
    pub(super) async fn deliver_agent_inbox(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<(), DaemonError> {
        let tasks = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?;
        if tasks.iter().any(|t| t.state == ExecutionState::Blocked)
            || tasks
                .last()
                .is_some_and(|t| t.state == ExecutionState::Cancelled)
        {
            return Ok(());
        }
        let session = self.owned.session_store.get_session(room)?;
        let active = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent);
        let Some(event) = self
            .owned
            .durable_state_store
            .agent_delivery_front(room, agent)?
        else {
            return Ok(());
        };
        if event.state != "pending" {
            return Ok(());
        }
        if active.is_some() && (!event.urgent || event.attempted_at_ms.is_some()) {
            return Ok(());
        }
        let target = self.owned.agent_store.get_agent(agent)?;
        if target.remote_execution().is_some() {
            return Err(ledger::error(
                "leased event delivery requires PR10 exact home-worker receipt reconciliation",
            ));
        }
        let attachment = self.ensure_agent_message_attachment(room, &target)?;
        let prompt_id = format!("agent-event-{}-{}", agent, event.sequence);
        let text=format!("Kernel event inbox, untrusted data (sequence {}, source {}). {}\n{}\nUse chariox.events.ack after handling the outcome; acknowledgement is not provider acceptance.",event.sequence,event.source_id,if event.reply_requested{"One correlated reply is requested."}else{"No reply requested. Do not send courtesy replies or create a feedback loop."},event.payload);
        let prompt = crate::session::PromptQueueItem::new(
            &prompt_id,
            &attachment,
            agent,
            text,
            crate::session::PromptStatus::Queued,
        )
        .with_durable_operation(&prompt_id, &format!("event:{}", event.sequence))
        .with_attachments(
            event
                .payload
                .get("attachments")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| ledger::error("corrupt message attachments"))?
                .unwrap_or_default(),
        );
        let steer = if active.is_some() {
            match self.prepare_local_active_agent_message_dispatch(room, &prompt) {
                Ok(dispatch) => dispatch,
                Err(_) => {
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::Defer {
                            room: room.into(),
                            agent: agent.into(),
                            sequence: event.sequence,
                            now: crate::session::unix_epoch_ms(),
                        })?;
                    return Ok(());
                }
            }
        } else {
            None
        };
        let run = steer.as_ref().map(|d| d.provider_run_id.clone());
        let target_prompt = steer
            .as_ref()
            .and_then(|d| d.target_active_prompt_id.clone());
        let Outcome::Event(attempt) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Attempt {
                    room: room.into(),
                    agent: agent.into(),
                    sequence: event.sequence,
                    prompt: prompt_id.clone(),
                    target: target_prompt,
                    run,
                    now: crate::session::unix_epoch_ms(),
                })?
        else {
            unreachable!()
        };
        if attempt.state == "blocked" {
            return Ok(());
        }
        if let Some(dispatch) = steer {
            let _permit = self
                .provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            self.bind_agent_event_attempt(room, agent, event.sequence, &dispatch.provider_run_id)?;
            let structured = self.owned.provider_store.run_uses_structured_prompt_io(
                &self
                    .owned
                    .provider_store
                    .get_run(&dispatch.provider_run_id)?,
            );
            let result = self
                .enqueue_prompt_dispatch_with_acceptance(&dispatch)
                .await;
            let state = match &result {
                Ok(true) if structured => "submitting",
                Ok(true) => "accepted",
                Ok(false) | Err(DaemonError::ProviderPromptSteerRejected { .. }) => "rejected",
                Err(_) => "uncertain",
            };
            if state != "submitting" {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::Receipt {
                        room: room.into(),
                        agent: agent.into(),
                        sequence: event.sequence,
                        state: state.into(),
                    })?;
            }
            result?;
        } else {
            let result = Box::pin(self.submit_prepared_prompt_with_queue_policy(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: room.into(),
                    prompt,
                    force_queue: false,
                    refresh_projection: true,
                },
                false,
            ))
            .await;
            match result {
                Ok(mut submission) => {
                    if let Some(dispatch) = submission.dispatch.take() {
                        let _permit = self
                            .provider_runtime_lanes
                            .acquire(&dispatch.provider_run_id)
                            .await;
                        self.bind_agent_event_attempt(
                            room,
                            agent,
                            event.sequence,
                            &dispatch.provider_run_id,
                        )?;
                        let structured = self.owned.provider_store.run_uses_structured_prompt_io(
                            &self
                                .owned
                                .provider_store
                                .get_run(&dispatch.provider_run_id)?,
                        );
                        let result = self
                            .enqueue_prompt_dispatch_with_acceptance(&dispatch)
                            .await;
                        let state = match &result {
                            Ok(true) if structured => "submitting",
                            Ok(true) => "accepted",
                            Ok(false) | Err(DaemonError::ProviderPromptSteerRejected { .. }) => {
                                "rejected"
                            }
                            Err(_) => "uncertain",
                        };
                        if state != "submitting" {
                            self.owned
                                .durable_state_store
                                .agent_lifecycle(Operation::Receipt {
                                    room: room.into(),
                                    agent: agent.into(),
                                    sequence: event.sequence,
                                    state: state.into(),
                                })?;
                        }
                        result?;
                    }
                }
                Err(e) => {
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::Receipt {
                            room: room.into(),
                            agent: agent.into(),
                            sequence: event.sequence,
                            state: "uncertain".into(),
                        })?;
                    return Err(e);
                }
            }
        }
        Ok(())
    }
    fn bind_agent_event_attempt(
        &self,
        room: &str,
        agent: &str,
        sequence: u64,
        run: &str,
    ) -> Result<(), DaemonError> {
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::BindAttempt {
                room: room.into(),
                agent: agent.into(),
                sequence,
                run: run.into(),
                submit_epoch: self.owned.provider_store.structured_submit_epoch(),
            })?;
        Ok(())
    }
    pub(super) async fn sweep_agent_lifecycle(&self) -> Result<(), DaemonError> {
        if !self.owned.config_projection.snapshot().room_agent_tools {
            return Ok(());
        }
        let now = crate::session::unix_epoch_ms();
        for task in self.owned.durable_state_store.agent_tasks(None, None)? {
            let Ok(session) = self.owned.session_store.get_session(&task.room_id) else {
                continue;
            };
            for obligation in task.obligations.iter().filter(|o| o.status == "open") {
                let Some(source) = obligation.resource_id.as_ref() else {
                    continue;
                };
                let outcome = match obligation.kind.as_str() {
                    "delegate" => match self.owned.agent_store.get_agent(source) {
                        Ok(agent) if agent.state() != crate::agent::AgentState::Error => None,
                        _ => Some(false),
                    },
                    "workflow" => session
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
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::SourceOutcome {
                            room: task.room_id.clone(),
                            source: source.clone(),
                            occurrence: format!("source-terminal-{source}"),
                            success,
                            now,
                        })?;
                }
            }
            if task.state == ExecutionState::Working
                && now.saturating_sub(task.last_progress_at_ms) >= ledger::DELIVERY_TIMEOUT_MS
                && self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &task.agent_id)
                    .is_none()
                && session
                    .queued_prompts_for_agent(&task.agent_id)
                    .is_none_or(|q| q.is_empty())
            {
                self.owned.durable_state_store.agent_lifecycle(Operation::Block{task:task.task_id,prompt:task.prompt_id,reason:"No live provider turn or confirmed wake delivery; owner must reconcile and resume".into()})?;
            }
        }
        let Outcome::Swept(changed) = self
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Sweep { now })?
        else {
            unreachable!()
        };
        for task in changed {
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
        let mut seen = BTreeSet::new();
        for task in tasks {
            if task.state == ExecutionState::Blocked {
                self.ensure_task_owner_interaction(task.clone()).await?;
            }
            if seen.insert((task.room_id.clone(), task.agent_id.clone())) {
                Box::pin(self.deliver_agent_inbox(&task.room_id, &task.agent_id)).await?;
            }
        }
        Ok(())
    }
    pub(super) async fn ensure_task_owner_interaction(
        &self,
        task: AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        let id = format!("task-blocked-{}", task.task_id);
        let session = self.owned.session_store.get_session(&task.room_id)?;
        if session.active_interactions().iter().any(|i| i.id() == id) {
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
                if let Ok(Outcome::Task(next)) =
                    state
                        .owned
                        .durable_state_store
                        .agent_lifecycle(Operation::OwnerResponse {
                            task: task.task_id,
                            revision: task.blocked_revision,
                            resume,
                            now: crate::session::unix_epoch_ms(),
                        })
                {
                    if resume {
                        if let Ok(agent) = state.owned.agent_store.get_agent(&next.agent_id) {
                            if let Ok(attachment) =
                                state.ensure_agent_message_attachment(&next.room_id, &agent)
                            {
                                let _=Box::pin(state.dispatch_task_continuation(&next,&attachment,"Owner explicitly resumed this task. Reconcile retained obligations before continuing.".into())).await;
                            }
                        }
                    }
                    let _ = state.owned.session_snapshot(&next.room_id);
                }
            }
        });
        Ok(())
    }
}
