use tokio::sync::oneshot;

use super::*;

impl KernelRuntimeState {
    pub(crate) fn update_forwarded_provider_login_interaction(
        &self,
        session_id: &str,
        agent_id: &str,
        interaction_id: &str,
        login: Option<crate::session::RuntimeProviderLogin>,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let interaction = session
            .active_interactions()
            .iter()
            .find(|interaction| interaction.id() == interaction_id);
        if !interaction_id.starts_with("provider-auth-recovery:")
            || interaction.is_some_and(|interaction| interaction.agent_id() != Some(agent_id))
        {
            return Err(DaemonError::LocalTransport {
                operation: "update provider login interaction",
                message: "login interaction binding mismatch".into(),
            });
        }
        match login {
            Some(login) => {
                self.owned
                    .update_provider_login_interaction(session_id, interaction_id, login)
            }
            None => self
                .owned
                .timeout_runtime_interaction(session_id, interaction_id),
        }
    }

    pub(super) async fn update_provider_login_interaction(
        &self,
        session_id: &str,
        agent_id: &str,
        interaction_id: &str,
        login: Option<crate::session::RuntimeProviderLogin>,
    ) -> Result<(), DaemonError> {
        if let Some((config, home_kernel_id, context, _)) = self
            .leased_interaction_home_target(session_id, agent_id, None)
            .await?
        {
            let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                &config, ClientTarget { daemon_id: Some(home_kernel_id), daemon_alias: None },
                RelayPeerRequest::UpdateNativeInteraction { context, interaction_id: interaction_id.into(), login },
                Duration::from_secs(20),
            ).await?;
            return match response {
                crate::transport::relay_peer::RelayPeerResponse::NativeInteractionUpdated {} => {
                    Ok(())
                }
                _ => Err(DaemonError::LocalTransport {
                    operation: "update provider login interaction",
                    message: "unexpected interaction update response".into(),
                }),
            };
        }
        self.update_forwarded_provider_login_interaction(
            session_id,
            agent_id,
            interaction_id,
            login,
        )
    }

    /// MP-08 / MP-10 / MP-11: a leased worker agent's kernel interaction
    /// (utility review, Vault input, provider login) is answered in the home
    /// session. Like a provider-native approval, it is bound to the worker turn
    /// that raised it, so the home withdraws it with that turn. Without a live
    /// turn there is nothing to bind and the interaction stays local.
    async fn leased_interaction_home_target(
        &self,
        session_id: &str,
        agent_id: &str,
        origin: Option<&crate::session::NativeInteractionOrigin>,
    ) -> Result<
        Option<(
            crate::config::DaemonConfig,
            String,
            crate::transport::relay_peer::RemoteNativeInteractionContext,
            crate::session::NativeInteractionOrigin,
        )>,
        DaemonError,
    > {
        let origin = match origin {
            Some(origin) => origin.clone(),
            None => {
                let Some(origin) = self
                    .owned
                    .provider_store
                    .get_run_for_agent(session_id, agent_id)
                    .and_then(|run| {
                        self.capture_native_interaction_origin(session_id, agent_id, run.id())
                    })
                else {
                    return Ok(None);
                };
                origin
            }
        };
        Ok(self
            .remote_native_interaction_context(session_id, agent_id, &origin)
            .await?
            .map(|(config, home_kernel_id, context)| (config, home_kernel_id, context, origin)))
    }

    pub(in crate::runtime) async fn create_runtime_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
    ) -> Result<oneshot::Receiver<PendingInteractionResolution>, DaemonError> {
        // MP-08 / MP-10 / MP-11: Leased utility reviews and Vault input belong
        // to the home session, using the same bridge as provider-native approval.
        let agent_id = interaction.agent_id().map(str::to_owned);
        if let Some(agent_id) = agent_id {
            if let Some((config, home_kernel_id, context, origin)) = self
                .leased_interaction_home_target(session_id, &agent_id, interaction.native_origin())
                .await?
            {
                let interaction = interaction.with_native_origin(Some(origin));
                let (tx, rx) = oneshot::channel();
                let timeout = Duration::from_secs(
                    interaction.timeout_sec().unwrap_or(900).saturating_add(15),
                );
                tokio::spawn(async move {
                    let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                        &config, ClientTarget {daemon_id: Some(home_kernel_id), daemon_alias: None},
                        RelayPeerRequest::ForwardNativeTurnInteraction {context, interaction}, timeout,
                    ).await;
                    if let Ok(RelayPeerResponse::NativeInteractionResolved { resolution }) =
                        response
                    {
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
        }
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
            forwarding,
            None,
        )?;
        // Login challenges and PTY output are human-only ephemeral state.
        if event_interaction
            .id()
            .starts_with("provider-auth-recovery:")
        {
            return Ok(rx);
        }
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
            None,
            Some(session.owner_user_id()),
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
            Some(crate::local::KernelConnectionClass::Terminal),
        )
        .await
    }

    /// A human terminal's answer; a critical approval also carries the
    /// passkey, or falls within the owner's remember window.
    /// `connection_class` is the answering connection's (protocol 402). An
    /// answer to a passkey prompt a terminal already answered is refused with
    /// `PASSKEY_ALREADY_ANSWERED` (protocol 403).
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
            .resolve_terminal_runtime_interaction(
                session_id,
                interaction_id,
                choice_id,
                custom_reply,
                caller_user_id,
                authorization.verified,
                connection_class,
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
