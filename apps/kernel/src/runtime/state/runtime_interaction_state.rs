use tokio::sync::oneshot;

use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime) async fn create_runtime_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
    ) -> Result<oneshot::Receiver<PendingInteractionResolution>, DaemonError> {
        // MP-08 / MP-10 / MP-11: Leased utility reviews and Vault input belong
        // to the home session, using the same bridge as provider-native approval.
        if let Some((config, home_kernel_id, context)) = self
            .remote_native_interaction_context(session_id, interaction.agent_id())
            .await?
        {
            let (tx, rx) = oneshot::channel();
            let timeout =
                Duration::from_secs(interaction.timeout_sec().unwrap_or(900).saturating_add(15));
            tokio::spawn(async move {
                let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                    &config, ClientTarget {daemon_id: Some(home_kernel_id), daemon_alias: None},
                    RelayPeerRequest::ForwardNativeInteraction {context, interaction}, timeout,
                ).await;
                if let Ok(RelayPeerResponse::NativeInteractionResolved { resolution }) = response {
                    let status = if resolution.status == "answered" {
                        "answered"
                    } else {
                        "timed_out"
                    };
                    let _ = tx.send(PendingInteractionResolution {
                        status,
                        choice_id: resolution.choice_id,
                        reply: resolution.reply,
                    });
                }
            });
            return Ok(rx);
        }
        let (tx, rx) = oneshot::channel();
        let event_interaction = interaction.clone();
        self.owned
            .register_runtime_interaction(session_id, interaction, tx)?;
        let source_attachment_id =
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(event_interaction.id());
        let dispatches = self.owned.metaagent_owned_agent_event_prompt_dispatches(
            session_id,
            "runtime.interaction",
            event_interaction.agent_id(),
            &source_attachment_id,
            format!(
                "Runtime interaction `{}` is pending",
                event_interaction.id()
            ),
            format!(
                "Agent `{}` needs input for runtime interaction `{}`: {}",
                event_interaction.agent_id(),
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
        self.owned
            .resolve_runtime_interaction(session_id, interaction_id, choice_id, custom_reply)
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
