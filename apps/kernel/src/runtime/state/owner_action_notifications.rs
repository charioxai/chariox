//! Blocked-task decisions are sources on the ordinary notification router.
use super::*;
use crate::durable_state::agent_lifecycle::AgentTaskExecution;
use crate::durable_state::workflow_notifications::{
    NotificationOperation, NotificationOutcome, SourceAdmission,
};

impl KernelRuntimeOwnedState {
    pub(super) fn ensure_owner_action_notification_source(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> Result<crate::local::WorkflowNotificationSource, DaemonError> {
        let source = owner_action_source(session);
        match self
            .durable_state_store
            .notify(NotificationOperation::Register(SourceAdmission {
                source,
                workflow_json: String::new(),
            }))? {
            NotificationOutcome::Source(source) => Ok(source),
            _ => Err(crate::durable_state::workflow_notifications::error(
                "unexpected notification registration result",
            )),
        }
    }
}

impl KernelRuntimeOwnedState {
    /// Every live Room of `owner` on this kernel has its "Agent needs your action" source, so an
    /// agent of the owner on another kernel can follow it before the Room's first alert.
    pub(super) fn ensure_owner_action_notification_sources(
        &self,
        owner: &str,
    ) -> Result<(), DaemonError> {
        let rooms = self.session_store.read().list_sessions();
        let (known, _, _) = self.durable_state_store.notification_inventory(owner)?;
        for room in rooms.iter().filter(|room| {
            room.owner_user_id() == owner
                && room.status() != crate::session::SessionStatus::Ended
                && !known
                    .iter()
                    .any(|s| s.source_id == owner_action_source(room).source_id)
        }) {
            self.ensure_owner_action_notification_source(room)?;
        }
        Ok(())
    }
}

/// The source of a Room's blocked-task alerts for its current owner.
pub(super) fn owner_action_source(
    session: &crate::session::RuntimeSession,
) -> crate::local::WorkflowNotificationSource {
    use sha2::{Digest, Sha256};
    // Each owner of the Room has their own source: a former owner's row never takes
    // the current owner's alerts, and Register retires it.
    crate::local::WorkflowNotificationSource {
        kind: crate::local::NotificationSourceKind::OwnerAction,
        source_id: format!(
            "owner_action_{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    session.host_daemon_id(),
                    session.id(),
                    session.owner_user_id()
                ))
                .expect("identities serialize")
            )
        ),
        owner_user_id: session.owner_user_id().into(),
        kernel_id: session.host_daemon_id().into(),
        session_id: session.id().into(),
        workflow_id: "owner_action".into(),
        exit_node_id: "blocked".into(),
        enabled: true,
        available: session.status() != crate::session::SessionStatus::Ended,
        name: "Agent needs your action".into(),
        display_name: "Agent needs your action".into(),
        exit_ordinal: 0,
        options: Vec::new(),
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn emit_task_owner_notification(
        &self,
        task: &AgentTaskExecution,
        interaction: &str,
    ) -> Result<(), DaemonError> {
        let session = self.session_store.get_session(&task.room_id)?;
        if session.status() == crate::session::SessionStatus::Ended
            || session.owner_user_id() != task.owner_user_id
        {
            return Ok(());
        }
        let source = self.ensure_owner_action_notification_source(&session)?;
        // The alert locates the kernel-owned decision. Task reasons, prompts,
        // outputs, and approval values never enter an outbound alert.
        let output = crate::session::WorkflowOutputPayload::new(
            serde_json::json!({"title":"Agent needs your action","room_id":task.room_id,"agent_id":task.agent_id,"task_id":task.task_id,"interaction_id":interaction}).to_string(), Vec::new());
        self.durable_state_store
            .notify(NotificationOperation::EmitOwnerAction {
                source_id: source.source_id,
                occurrence_id: interaction.into(),
                output,
                at: crate::session::unix_epoch_ms(),
            })?;
        Ok(())
    }
}

/// What followers read of an ask, cut at these sizes after secret-shaped text is removed.
const ASK_TITLE_MAX_CHARS: usize = 200;
const ASK_MESSAGE_MAX_CHARS: usize = 2000;

fn bounded_ask_text(text: &str, max_chars: usize) -> String {
    let clean = crate::secret_redaction::redact_secrets(text);
    if clean.chars().count() <= max_chars {
        return clean.into_owned();
    }
    let mut cut: String = clean.chars().take(max_chars).collect();
    cut.push('\u{2026}');
    cut
}

impl KernelRuntimeOwnedState {
    /// An agent's open question the person holds wakes the followers of "Agent needs your action"
    /// with its title and message, so they can answer it without a second read. A question an
    /// agent holds alerts them only when it is handed up to the person.
    pub(super) fn alert_owner_of_ask(
        &self,
        session_id: &str,
        interaction: &crate::session::RuntimeInteraction,
    ) {
        // A blocked task's decision has its own alert above.
        let blocked_task = interaction
            .id()
            .starts_with(super::agent_task_owner_interactions::BLOCKED_TASK_DECISION_PREFIX);
        let theirs = self
            .pending_interactions
            .write()
            .get(interaction.id())
            .is_some_and(|pending| pending.holder == super::DecisionHolder::User);
        if blocked_task || !theirs {
            return;
        }
        if let Err(error) = self.emit_ask_notification(session_id, interaction) {
            tracing::warn!(%error, interaction_id = interaction.id(), "ask alert failed");
        }
    }

    fn emit_ask_notification(
        &self,
        session_id: &str,
        interaction: &crate::session::RuntimeInteraction,
    ) -> Result<(), DaemonError> {
        let (Some(agent_id), Some(task_id)) = (interaction.agent_id(), interaction.asking_task_id())
        else {
            return Ok(());
        };
        let session = self.session_store.get_session(session_id)?;
        if session.status() == crate::session::SessionStatus::Ended {
            return Ok(());
        }
        let source = self.ensure_owner_action_notification_source(&session)?;
        let output = crate::session::WorkflowOutputPayload::new(
            serde_json::json!({
                "title": "Agent needs your action",
                "room_id": session_id,
                "agent_id": agent_id,
                "task_id": task_id,
                "interaction_id": interaction.id(),
                "question": {
                    "title": interaction.title().map(|title| bounded_ask_text(title, ASK_TITLE_MAX_CHARS)),
                    "message": bounded_ask_text(interaction.message(), ASK_MESSAGE_MAX_CHARS),
                },
            })
            .to_string(),
            Vec::new(),
        );
        self.durable_state_store
            .notify(NotificationOperation::EmitOwnerAction {
                source_id: source.source_id,
                occurrence_id: interaction.id().into(),
                output,
                at: crate::session::unix_epoch_ms(),
            })?;
        Ok(())
    }
}
