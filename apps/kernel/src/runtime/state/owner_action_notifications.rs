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
