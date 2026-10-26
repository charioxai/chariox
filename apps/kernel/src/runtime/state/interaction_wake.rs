//! MP-08 / MP-09 / MP-10 / MP-11: task-owned questions use the ordinary wake ledger.
use super::wake_driver::{Cancel, Liveness, Recovery, WakeDriver};
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, AgentWake, Operation, WakeEnd};
use crate::session::RuntimeInteraction;

pub(super) const KIND: &str = "interaction";

pub(super) struct InteractionWakeDriver {
    owned: KernelRuntimeOwnedState,
}

fn question(wake: &AgentWake) -> Result<RuntimeInteraction, DaemonError> {
    let interaction: RuntimeInteraction = serde_json::from_value(wake.detail.clone())
        .map_err(|_| ledger::error("question wake detail is unreadable"))?;
    if interaction.id() != wake.id
        || interaction.agent_id() != Some(wake.agent_id.as_str())
        || interaction.asking_task_id() != Some(wake.task_id.as_str())
    {
        return Err(ledger::error("question wake binding is invalid"));
    }
    Ok(interaction)
}

impl WakeDriver for InteractionWakeDriver {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn arm(&self, wake: &AgentWake) -> Result<(), DaemonError> {
        let interaction = question(wake)?;
        if !self
            .owned
            .pending_interactions
            .write()
            .get(&wake.id)
            .is_some_and(|p| p.belongs_to(&self.owned.session_store))
        {
            let (tx, _rx) = tokio::sync::oneshot::channel();
            self.owned.register_runtime_interaction(
                &wake.room_id,
                interaction,
                tx,
                None,
                None,
                None,
            )?;
        }
        let registered = self
            .owned
            .pending_interactions
            .write()
            .get(&wake.id)
            .cloned()
            .filter(|p| {
                p.belongs_to(&self.owned.session_store)
                    && p.async_task_id.as_deref() == Some(wake.task_id.as_str())
            })
            .ok_or_else(|| ledger::error("asking task ended before the question was registered"))?;
        if !self.owned.agent_interaction_is_live(&registered) {
            return Err(ledger::error("asking task ended before question recovery"));
        }
        let current = self
            .owned
            .durable_state_store
            .agent_wake(&wake.id)?
            .ok_or_else(|| ledger::error("question wake unavailable"))?;
        if current.state == "starting" {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::WakeVerified {
                    id: wake.id.clone(),
                    now: crate::session::unix_epoch_ms(),
                })?;
        } else if current.state != "running" {
            return Err(ledger::error("question is no longer pending"));
        }
        Ok(())
    }
    fn cancel(&self, wake: &AgentWake) -> Result<Cancel, DaemonError> {
        self.owned.remove_async_question(
            wake,
            "Question withdrawn because its task, agent or wake was cancelled",
        )?;
        Ok(Cancel::Done)
    }
    fn recover(&self, wake: &AgentWake) -> Recovery {
        match self.arm(wake) {
            Ok(()) => Recovery::Reattached,
            Err(error) => Recovery::Lost(error.to_string()),
        }
    }
    fn liveness(&self, wake: &AgentWake) -> Liveness {
        match question(wake) {
            Ok(_) => Liveness::Live,
            Err(error) => Liveness::Lost(error.to_string()),
        }
    }
}

impl KernelRuntimeState {
    pub(crate) fn with_interaction_answerer(
        &self,
        caller: &crate::runtime::command::KernelCaller,
    ) -> Self {
        let mut state = self.clone();
        state.interaction_answerer = Some(caller.clone());
        state
    }

    pub(super) fn schedule_question_answer_delivery(&self, id: &str) -> Result<(), DaemonError> {
        if let Some(wake) = self.owned.durable_state_store.agent_wake(id)? {
            if wake.kind == KIND && wake.state == "exited" {
                self.schedule_wake_delivery(&wake);
            }
        }
        Ok(())
    }

    pub(super) fn register_interaction_wake_driver(&mut self) {
        let driver = Arc::new(InteractionWakeDriver {
            owned: self.owned.clone(),
        });
        let source: Arc<dyn WakeDriver> = driver.clone();
        self.owned
            .agent_wakes
            .drivers
            .register_weak(Arc::downgrade(&source));
        self.interaction_wake_driver = Some(driver);
    }

    pub(super) fn arm_interaction_wake(
        &self,
        room: &str,
        agent: &str,
        interaction: RuntimeInteraction,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        if !self.room_agent_tools_enabled() {
            return Err(ledger::error("async questions require Room event tools"));
        }
        let session = self.owned.session_store.get_session(room)?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent)
            .filter(|p| p.status() == crate::session::PromptStatus::Running)
            .ok_or_else(|| ledger::error("async question requires a running turn"))?;
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .into_iter()
            .find(|t| t.prompt_id == prompt.id())
            .ok_or_else(|| ledger::error("asking task unavailable in this turn"))?;
        if task.wait.is_some() {
            return Err(ledger::error(
                "Wait already accepted. End this provider turn without more tools",
            ));
        }
        let interaction = interaction.with_asking_task_id(task.task_id.clone());
        let mut wake = super::tool_dispatch::agent_wakes::new_wake(
            &task,
            KIND,
            "Question awaiting an answer".into(),
            crate::session::unix_epoch_ms(),
        );
        wake.id = interaction.id().into();
        wake.registration_id = format!("completion-{}", wake.id);
        wake.state = "starting".into();
        wake.next_due_ms = interaction.timeout_sec().map(|seconds| {
            interaction
                .requested_at_ms()
                .saturating_add(seconds.saturating_mul(1000))
        });
        wake.detail = serde_json::to_value(interaction)
            .map_err(|_| ledger::error("question encoding failed"))?;
        self.arm_wake_source(&task, &wake)?;
        self.refresh_wake_projection(room);
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "interaction_id": wake.id, "registration_id": wake.registration_id,
            }),
        })
    }

    /// Runs on the existing scheduler. Both answered and cancelled cards can
    /// survive a crash between the ledger commit and projection cleanup.
    pub(super) fn tick_interaction_wakes(&self, now: u64) -> Result<(), DaemonError> {
        for wake in self.owned.durable_state_store.agent_wakes(None, None)? {
            if wake.kind != KIND {
                continue;
            }
            self.tick_interaction_wake(&wake, now)?;
        }
        Ok(())
    }

    fn tick_interaction_wake(&self, wake: &AgentWake, now: u64) -> Result<(), DaemonError> {
        if matches!(wake.state.as_str(), "exited" | "lost" | "cancelled") {
            let pending = self
                .owned
                .pending_interactions
                .write()
                .get(&wake.id)
                .is_some_and(|p| p.belongs_to(&self.owned.session_store));
            let projected = self
                .owned
                .session_store
                .get_session(&wake.room_id)
                .is_ok_and(|s| s.active_interactions().iter().any(|q| q.id() == wake.id));
            if !pending && !projected {
                return Ok(());
            }
            crate::logging::debug_with_fields(
                "runtime.interaction_wake",
                "reconciling finished question",
                serde_json::json!({"interaction_id": wake.id}),
            );
            self.owned
                .remove_async_question(wake, "Question is no longer pending")?;
            // Ordinary answers dispatch at resolution. Only crash cleanup needs
            // another kick here; terminal receipts belong to history.
            if wake.last_sequence.is_some()
                && wake.last_delivered_at_ms.is_none()
                && !matches!(wake.last_delivery.as_deref(), Some("expired" | "failed"))
            {
                self.schedule_wake_delivery(wake);
            }
        } else if matches!(wake.state.as_str(), "starting" | "running")
            && wake.next_due_ms.is_some_and(|deadline| deadline <= now)
        {
            self.owned
                .timeout_runtime_interaction(&wake.room_id, &wake.id)?;
            self.schedule_wake_delivery(wake);
        }
        Ok(())
    }
}

impl KernelRuntimeOwnedState {
    /// The resolution and inbox receipt commit before the card is removed.
    /// Replays see a settled source and cannot produce a second answer.
    pub(super) fn record_async_question_resolution(
        &self,
        interaction: &RuntimeInteraction,
        resolution: &super::PendingInteractionResolution,
        caller: Option<&str>,
        answerer: Option<&crate::runtime::command::KernelCaller>,
    ) -> Result<(), DaemonError> {
        if interaction.asking_task_id().is_none() {
            return Ok(());
        }
        let wake = self
            .durable_state_store
            .agent_wake(interaction.id())?
            .ok_or_else(|| ledger::error("question wake unavailable"))?;
        if !matches!(wake.state.as_str(), "starting" | "running") {
            return Err(ledger::error("question was already answered or withdrawn"));
        }
        self.durable_state_store
            .agent_lifecycle(Operation::WakeOccurred {
                id: wake.id.clone(),
                occurrence: format!("{}-resolved", wake.id),
                kind: "interaction_resolved".into(),
                summary: Some(format!("Question {}", resolution.status)),
                payload: serde_json::json!({
                    "interaction_id": wake.id, "status": resolution.status,
                    "choice_id": resolution.choice_id, "reply": resolution.reply,
                    "answered_by": answerer.map(|a| a.caller_id.as_str()).or(caller),
                    "user_id": answerer.and_then(|a| a.user_id.as_deref()).or(caller),
                    "agent_id": answerer.and_then(|a| a.metaagent_id.as_deref()),
                    "client_id": answerer.map(|a| a.client_id.as_deref().unwrap_or(&a.caller_id)),
                    "connection_class": answerer.and_then(|a| a.connection_class),
                }),
                end: Some(WakeEnd::Succeeded),
                now: crate::session::unix_epoch_ms(),
            })?;
        Ok(())
    }

    fn remove_async_question(&self, wake: &AgentWake, reason: &str) -> Result<(), DaemonError> {
        let _mutation = self
            .pending_interactions
            .mutation
            .lock()
            .map_err(|_| ledger::error("interaction store unavailable"))?;
        self.pending_interactions.write().remove(&wake.id);
        let mut sessions = self.session_store.write();
        let Ok(mut session) = sessions.get_session(&wake.room_id) else {
            return Ok(());
        };
        if session.remove_active_interaction(&wake.id).is_none() {
            return Ok(());
        }
        sessions.restore_session(session);
        drop(sessions);
        self.record_notice_for_agent(
            &wake.room_id,
            None,
            Some(&wake.agent_id),
            self.attachment_store
                .list_session_attachment_ids(&wake.room_id),
            reason,
        );
        self.session_snapshot(&wake.room_id)?;
        self.terminal_stream
            .notify_terminal_projection_change(&wake.room_id);
        Ok(())
    }
}
