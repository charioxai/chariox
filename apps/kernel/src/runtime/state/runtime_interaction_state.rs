use tokio::sync::oneshot;

use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime) async fn create_runtime_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
    ) -> Result<oneshot::Receiver<PendingInteractionResolution>, DaemonError> {
        let agent_id = interaction
            .agent_id()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "create runtime interaction",
                message: "Agent interaction requires an agent subject".into(),
            })?
            .to_owned();
        let (tx, rx) = oneshot::channel();
        let event_interaction = interaction.clone();
        self.owned
            .register_runtime_interaction(session_id, interaction, tx, None)?;
        let source_attachment_id =
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(event_interaction.id());
        let dispatches = self.owned.metaagent_owned_agent_event_prompt_dispatches(
            session_id,
            "runtime.interaction",
            &agent_id,
            &source_attachment_id,
            format!(
                "Runtime interaction `{}` is pending",
                event_interaction.id()
            ),
            format!(
                "Agent `{}` needs input for runtime interaction `{}`: {}",
                agent_id,
                event_interaction.id(),
                event_interaction.message()
            ),
            serde_json::json!({
                "interaction": event_interaction,
            }),
        );
        self.spawn_workflow_prompt_dispatches(dispatches);
        Ok(rx)
    }

    pub(crate) async fn resolve_runtime_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        custom_reply: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.owned.resolve_runtime_interaction(
            session_id,
            interaction_id,
            choice_id,
            custom_reply,
            None,
        )
    }

    pub(in crate::runtime) async fn create_kernel_operation_interaction(
        &self,
        session_id: &str,
        owner_user_id: &str,
        interaction: crate::session::RuntimeInteraction,
    ) -> Result<oneshot::Receiver<PendingInteractionResolution>, DaemonError> {
        if interaction.kernel_operation_id().is_none() {
            return Err(DaemonError::LocalTransport {
                operation: "create kernel operation interaction",
                message: "Kernel decision requires an operation subject".into(),
            });
        }
        let (tx, rx) = oneshot::channel();
        self.owned.register_runtime_interaction(
            session_id,
            interaction,
            tx,
            Some(owner_user_id),
        )?;
        // Human-only decisions are projected to terminals, never dispatched to
        // an agent's prompt or Meta delegation tools.
        Ok(rx)
    }

    pub(in crate::runtime) async fn resolve_terminal_runtime_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        custom_reply: Option<&str>,
        caller_user_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.owned.resolve_runtime_interaction(
            session_id,
            interaction_id,
            choice_id,
            custom_reply,
            caller_user_id,
        )
    }

    pub(crate) async fn timeout_runtime_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
    ) -> Result<(), DaemonError> {
        self.owned
            .timeout_runtime_interaction(session_id, interaction_id)
    }
}
