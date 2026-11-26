use tokio::sync::oneshot;

use super::*;

/// The agent that answered, with the relationship used for the audit record.
pub(super) enum AgentDecisionAnswer<'a> {
    Parent {
        holder: &'a str,
        child: &'a str,
    },
    RoomAgent {
        agent: &'a str,
        room: &'a str,
    },
    /// An external agent answering for the person, by name.
    ForPerson {
        agent: &'a str,
    },
}

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
            // Worker to home: the route the interaction itself was created on.
            let response = self
                .send_worker_home_runtime_request(
                    &config,
                    &home_kernel_id,
                    RelayPeerRequest::UpdateNativeInteraction {
                        context,
                        interaction_id: interaction_id.into(),
                        login,
                    },
                    Duration::from_secs(20),
                )
                .await?;
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
    /// session. Bind to the current turn when present, or to the live provider
    /// run for an idle review. The home withdraws it when that binding ends.
    pub(super) async fn leased_interaction_home_target(
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
                        if run.state() != crate::provider::ProviderRunState::Running {
                            return None;
                        }
                        Some(
                            self.capture_native_interaction_origin(session_id, agent_id, run.id())
                                .unwrap_or_else(|| {
                                    crate::session::NativeInteractionOrigin::ProviderStartup {
                                        provider_run_id: run.id().into(),
                                    }
                                }),
                        )
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
        kind: super::CardKind,
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
                let (mut tx, rx) = oneshot::channel();
                let timeout = Duration::from_secs(
                    interaction.timeout_sec().unwrap_or(900).saturating_add(15),
                );
                let runtime = self.clone();
                tokio::spawn(async move {
                    let response = tokio::select! {
                        biased;
                        _ = tx.closed() => return,
                        response = runtime.send_worker_home_runtime_request(
                            &config, &home_kernel_id,
                            RelayPeerRequest::ForwardNativeTurnInteraction {context, interaction}, timeout,
                        ) => response,
                    };
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
        self.create_runtime_interaction_with_forwarding(kind, session_id, interaction, None)
            .await
    }

    pub(in crate::runtime) async fn create_runtime_interaction_with_forwarding(
        &self,
        kind: super::CardKind,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        forwarding: Option<&crate::transport::relay_peer::RemoteNativeInteractionContext>,
    ) -> Result<oneshot::Receiver<PendingInteractionResolution>, DaemonError> {
        self.create_agent_interaction(kind, session_id, interaction, forwarding, None)
            .await
    }

    pub(in crate::runtime) async fn create_codex_workspace_approval(
        &self,
        kind: super::CardKind,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        request: &crate::runtime::native_interaction_bridge::CodexApprovalRequest,
    ) -> Result<oneshot::Receiver<PendingInteractionResolution>, DaemonError> {
        self.create_agent_interaction(kind, session_id, interaction, None, Some(request))
            .await
    }

    async fn create_agent_interaction(
        &self,
        kind: super::CardKind,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        forwarding: Option<&crate::transport::relay_peer::RemoteNativeInteractionContext>,
        codex_approval: Option<&crate::runtime::native_interaction_bridge::CodexApprovalRequest>,
    ) -> Result<oneshot::Receiver<PendingInteractionResolution>, DaemonError> {
        let agent_id = interaction
            .agent_id()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "create runtime interaction",
                message: "Agent interaction requires an agent subject".into(),
            })?
            .to_owned();
        if let Some(refused) = self
            .refuse_run_step_interaction(session_id, &agent_id, &interaction)
            .await?
        {
            return Ok(refused);
        }
        let (tx, rx) = oneshot::channel();
        let event_interaction = interaction.clone();
        if let Some(request) = codex_approval {
            self.owned.register_codex_workspace_approval(
                crate::runtime::state::runtime_interaction_owned_state::CardRequest(
                    kind,
                    interaction,
                ),
                session_id,
                tx,
                request,
            )?;
        } else {
            self.owned.register_runtime_interaction(
                crate::runtime::state::runtime_interaction_owned_state::CardRequest(
                    kind,
                    interaction,
                ),
                session_id,
                tx,
                None,
                forwarding,
                None,
            )?;
        }
        self.announce_child_decision(session_id, event_interaction.id())
            .await?;
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

    #[cfg(test)]
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
        )?;
        self.schedule_question_answer_delivery(interaction_id)
    }

    pub(super) fn create_terminal_credential_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
    ) -> Result<oneshot::Receiver<PendingInteractionResolution>, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let (tx, rx) = oneshot::channel();
        self.owned.register_runtime_interaction(
            crate::runtime::state::runtime_interaction_owned_state::CardRequest(
                super::CardKind::VaultCredentialPayment,
                interaction,
            ),
            session_id,
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
        kind: super::CardKind,
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
        let id = interaction.id().to_owned();
        self.owned.register_runtime_interaction(
            crate::runtime::state::runtime_interaction_owned_state::CardRequest(kind, interaction),
            session_id,
            tx,
            Some(owner_user_id),
            None,
            None,
        )?;
        // A person-only decision is projected to terminals, never dispatched to an agent's prompt
        // or Meta delegation tools. One an agent may hold, such as a machine acquisition, tells its holder.
        self.announce_child_decision(session_id, &id).await?;
        Ok(rx)
    }

    #[cfg(test)]
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
        let agent = self.answering_agent();
        let card = agent
            .as_ref()
            .map(|_| self.delegated_card(caller_user_id, interaction_id))
            .transpose()?;
        if agent.is_none()
            && connection_class
                .is_some_and(|class| class != crate::local::KernelConnectionClass::Terminal)
            && (interaction_id.starts_with("capability-request-")
                || interaction_id.starts_with("machine-acquisition-")
                || interaction_id.starts_with("kernel-enrollment-"))
            && self
                .owned
                .pending_interactions
                .write()
                .get(interaction_id)
                .is_some_and(|pending| pending.kernel_operation_owner.is_some())
        {
            return Err(crate::runtime::room_tool_admission::denied(
                "Only the owner's Chariox terminal can authorize resource acquisition",
            ));
        }
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
        if agent.is_none()
            && connection_class
                .is_some_and(|class| class != crate::local::KernelConnectionClass::Terminal)
            && self
                .owned
                .pending_interactions
                .write()
                .get(interaction_id)
                .is_some_and(|pending| pending.user_domain_interaction.is_some())
        {
            return Err(DaemonError::LocalTransport {
                operation: "runtime interaction",
                message: "Only a Chariox terminal can answer a user-domain decision".into(),
            });
        }
        // MP-11 A07: a hand-off is answered only by the owner's terminal, and
        // only with safe choices here; values use protected entry.
        if super::is_handoff_interaction_id(interaction_id) {
            if connection_class != Some(crate::local::KernelConnectionClass::Terminal)
                || custom_reply.is_some()
                || passkey.is_some()
            {
                return Err(DaemonError::LocalTransport {
                    operation: "owner hand-off",
                    message: "Only the owner's Chariox terminal can answer a hand-off".into(),
                });
            }
            return self.answer_handoff_choice(
                session_id,
                interaction_id,
                choice_id,
                caller_user_id,
            );
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
        // Row 539: a secret card's answer goes into the Vault entry it names before the card closes;
        // a refused store leaves the card open, and the agent only ever reads that it was stored.
        let (_reservation, _vault_guard, undo) = if choice_id == "secret" {
            match self
                .store_card_secret(session_id, interaction_id, custom_reply, agent.is_some())
                .await?
            {
                Some((reservation, guard, undo)) => (Some(reservation), Some(guard), Some(undo)),
                None => (None, None, None),
            }
        } else if self
            .owned
            .pending_interactions
            .secret_store_in_progress(interaction_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "credential secret card",
                message: "The secret is being stored; answer again in a moment".into(),
            });
        } else {
            (None, None, None)
        };
        let resolved = self
            .owned
            .resolve_attributed_terminal_runtime_interaction(
                session_id,
                interaction_id,
                choice_id,
                custom_reply,
                caller_user_id,
                authorization.verified,
                connection_class,
                self.interaction_answerer.as_ref(),
            )
            .map_err(|error| {
                self.owned
                    .closed_interaction_error(session_id, interaction_id, error)
            });
        // A rejected answer leaves the Vault as it was, whichever path closed the card first.
        if let (Err(_), Some(undo)) = (&resolved, undo) {
            undo.restore();
        }
        resolved?;
        self.audit_interaction_answered(
            session_id,
            interaction_id,
            choice_id,
            connection_class,
            agent
                .as_deref()
                .map(|agent| AgentDecisionAnswer::ForPerson { agent }),
        );
        // A card outside any Room has no Room to tell; its record names the agent.
        if let (Some(agent), Some(card), false) = (&agent, &card, session_id.is_empty()) {
            self.record_delegated_answer(session_id, agent, card, choice_id);
        }
        self.schedule_question_answer_delivery(interaction_id)
    }

    /// Who closed a decision and through which connection; never the reply text. An agent
    /// answer names its source room, or the child when it answered as the child's parent.
    /// The answer already took effect, so a failed record is logged, not returned.
    pub(super) fn audit_interaction_answered(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        connection_class: Option<crate::local::KernelConnectionClass>,
        agent_answer: Option<AgentDecisionAnswer<'_>>,
    ) {
        let mut payload = serde_json::json!({
            "session_id": session_id,
            "choice_id": choice_id,
            "connection_class": connection_class,
            "at_ms": crate::session::unix_epoch_ms(),
        });
        match agent_answer {
            Some(AgentDecisionAnswer::Parent { holder, child }) => {
                payload["answered_by_agent"] = holder.into();
                payload["child_agent_id"] = child.into();
            }
            Some(AgentDecisionAnswer::RoomAgent { agent, room }) => {
                payload["answered_by_agent"] = agent.into();
                payload["answering_room_id"] = room.into();
            }
            Some(AgentDecisionAnswer::ForPerson { agent }) => {
                payload["answered_for_owner_by"] = agent.into();
            }
            None => {}
        }
        let recorded = self.owned.durable_state_store.append_event(
            "interaction.answered",
            Some(interaction_id.into()),
            payload,
        );
        if let Err(error) = recorded {
            crate::logging::error_with_fields(
                "runtime.interaction",
                "answered interaction could not be recorded",
                serde_json::json!({"error": error.to_string()}),
            );
        }
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

impl KernelRuntimeState {
    async fn store_card_secret(
        &self,
        session_id: &str,
        interaction_id: &str,
        custom_reply: Option<&str>,
        answered_by_agent: bool,
    ) -> Result<
        Option<(
            super::runtime_interaction_owned_state::SecretStoreReservation,
            tokio::sync::OwnedRwLockWriteGuard<()>,
            SecretUndo,
        )>,
        DaemonError,
    > {
        if session_id.is_empty() {
            return Ok(None);
        }
        let Some(card) = self
            .owned
            .session_store
            .get_session(session_id)?
            .active_interactions()
            .iter()
            .find(|card| card.id() == interaction_id)
            .cloned()
        else {
            return Ok(None);
        };
        let (Some(request), Some(custom_choice)) = (card.secret_request(), card.custom_choice())
        else {
            return Ok(None);
        };
        if answered_by_agent {
            return Err(DaemonError::LocalTransport {
                operation: "credential secret card",
                message: "Only the person can enter this secret".into(),
            });
        }
        let secret = custom_reply.ok_or_else(|| DaemonError::LocalTransport {
            operation: "credential secret card",
            message: "the secret is required".into(),
        })?;
        super::runtime_interaction_owned_state::validate_runtime_interaction_custom_reply(
            custom_choice,
            secret,
        )?;
        let reservation = self
            .owned
            .reserve_secret_store(session_id, interaction_id)?;
        let user_config = self.owned.config_projection.snapshot().user_config;
        let service = crate::secret::RuntimeSecretService::with_vault_config(
            crate::credential::load_user_credentials()?,
            &user_config.credential_vault,
        )?;
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        // The Vault lifecycle lock is held from the capture of what the entry holds, through the save, the
        // resolution and any rollback: nothing else can change the entry in between, so taking a refused
        // answer back never undoes another operation's update.
        let vault_guard = self.vault_observation_mutation_guard().await;
        let undo = SecretUndo::capture(&service, &registry, &request.credential)?;
        self.upsert_vault_credential_locked(
            &service,
            &registry,
            request.credential.clone(),
            secret,
            request.overwrite,
        )
        .await?;
        Ok(Some((reservation, vault_guard, undo)))
    }
}

/// What the Vault held for a secret card's entry before its answer was stored, so an answer the card
/// then refuses (its asking agent was destroyed, its Room ended) leaves the Vault as it was.
struct SecretUndo {
    service: crate::secret::RuntimeSecretService,
    registry: crate::credential::CharioxCredentialRegistry,
    credential_id: String,
    vault_key: Option<String>,
    previous_entry: Option<crate::config::UserCredentialConfig>,
    previous_value: Option<zeroize::Zeroizing<String>>,
}

impl SecretUndo {
    fn capture(
        service: &crate::secret::RuntimeSecretService,
        registry: &crate::credential::CharioxCredentialRegistry,
        credential: &crate::config::UserCredentialConfig,
    ) -> Result<Self, DaemonError> {
        let vault_key = match &credential.source {
            crate::config::UserCredentialSourceConfig::Vault { key } => Some(key.clone()),
            _ => None,
        };
        let previous_value = match &vault_key {
            Some(key) => service.vault_secret_for_dictation(key)?,
            None => None,
        };
        Ok(Self {
            service: service.clone(),
            registry: registry.clone(),
            credential_id: credential.id.clone(),
            vault_key,
            previous_entry: registry.get(&credential.id)?,
            previous_value,
        })
    }

    fn restore(self) {
        let outcome = (|| -> Result<(), DaemonError> {
            match (&self.vault_key, &self.previous_value) {
                (Some(key), Some(value)) => self.service.set_vault_secret(key, value.as_str())?,
                (Some(key), None) => self.service.delete_vault_secret(key)?,
                (None, _) => {}
            }
            match self.previous_entry {
                Some(entry) => {
                    self.registry.upsert(entry)?;
                }
                None => {
                    self.registry.remove(&self.credential_id)?;
                }
            }
            Ok(())
        })();
        if let Err(error) = outcome {
            tracing::error!(%error, "a refused secret answer could not be taken back from the Vault");
        }
    }
}
