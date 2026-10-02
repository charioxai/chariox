use tokio::sync::oneshot;

use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime) async fn create_runtime_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
    ) -> Result<oneshot::Receiver<PendingInteractionResolution>, DaemonError> {
        self.create_runtime_interaction_with_forwarding(session_id, interaction, None)
            .await
    }

    pub(in crate::runtime) async fn create_runtime_interaction_with_forwarding(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        forwarding: Option<&crate::transport::relay_peer::RemoteNativeInteractionContext>,
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
        self.owned.register_runtime_interaction(
            session_id,
            interaction,
            tx,
            None,
            None,
            forwarding,
        )?;
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
            false,
        )
    }

    pub(super) fn create_terminal_credential_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
    ) -> Result<oneshot::Receiver<PendingInteractionResolution>, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let (tx, rx) = oneshot::channel();
        self.owned.register_runtime_interaction(
            session_id,
            interaction,
            tx,
            None,
            Some(session.owner_user_id()),
            None,
        )?;
        // Secret-entry and vault-management prompts go only to terminals.
        Ok(rx)
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
            None,
            None,
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
        self.answer_terminal_runtime_interaction(
            session_id,
            interaction_id,
            choice_id,
            custom_reply,
            caller_user_id,
            None,
            None,
            None,
        )
        .await
    }

    /// A human terminal's answer; a critical approval also carries the
    /// passkey, or falls within the owner's remember window.
    /// `connection_class` is the answering connection's (protocol 393). An
    /// answer to a passkey prompt a terminal already answered is refused with
    /// `PASSKEY_ALREADY_ANSWERED` (protocol 394).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::runtime) async fn answer_terminal_runtime_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        custom_reply: Option<&str>,
        caller_user_id: Option<&str>,
        passkey: Option<&crate::local::ApprovalPasskey>,
        passkey_remember_minutes: Option<u32>,
        connection_class: Option<crate::local::KernelConnectionClass>,
    ) -> Result<(), DaemonError> {
        if connection_class
            .is_some_and(|class| class != crate::local::KernelConnectionClass::Terminal)
            && self
                .owned
                .pending_interactions
                .write()
                .get(interaction_id)
                .is_some_and(|pending| pending.terminal_credential_owner.is_some())
        {
            return Err(DaemonError::LocalTransport {
                operation: "credential interaction",
                message: "Only a Chariox terminal can answer a credential prompt".into(),
            });
        }
        let authorization = self
            .authorize_critical_approval(
                session_id,
                interaction_id,
                choice_id,
                caller_user_id,
                passkey,
                passkey_remember_minutes,
                connection_class,
            )
            .await?;
        self.owned
            .resolve_runtime_interaction(
                session_id,
                interaction_id,
                choice_id,
                custom_reply,
                caller_user_id,
                authorization.verified,
            )
            .map_err(|error| {
                self.owned
                    .closed_interaction_error(session_id, interaction_id, error)
            })
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
