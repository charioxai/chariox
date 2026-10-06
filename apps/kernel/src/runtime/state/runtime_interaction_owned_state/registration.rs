use super::*;

impl KernelRuntimeOwnedState {
    pub(in crate::runtime::state) fn register_runtime_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        responder: tokio::sync::oneshot::Sender<super::super::PendingInteractionResolution>,
        kernel_operation_owner: Option<&str>,
        forwarding: Option<&crate::transport::relay_peer::RemoteNativeInteractionContext>,
        terminal_credential_owner: Option<&str>,
    ) -> Result<(), DaemonError> {
        let _admission = self.begin_managed_activity_admission()?;
        let _mutation = self
            .pending_interactions
            .mutation
            .lock()
            .map_err(|_| interaction_error("Interaction store is unavailable"))?;
        self.pending_interactions.prune_abandoned_kernel_owners();
        if !interaction.valid_subject()
            || interaction.id().trim().is_empty()
            || interaction.id().len() > 128
            || interaction.id().chars().any(char::is_control)
        {
            return Err(interaction_error("Invalid interaction identity"));
        }
        // Only a kernel decision may ask for the passkey; an agent's question
        // must never prompt the human for it.
        if interaction.agent_id().is_some()
            && interaction
                .choices()
                .iter()
                .any(crate::session::RuntimeInteractionChoice::requires_passkey)
        {
            return Err(interaction_error(
                "Only kernel decisions can require the Chariox passkey",
            ));
        }
        if session_id == crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE {
            return self.register_kernel_wide_access_interaction(
                interaction,
                responder,
                kernel_operation_owner,
                forwarding,
                terminal_credential_owner,
            );
        }
        // Resolve agent identity before taking the session write guard; session
        // updates must not acquire the agent store in the opposite lock order.
        let mut remote_binding = None;
        if let Some(agent_id) = interaction.agent_id() {
            let agent = self.agent_store.get_agent(agent_id)?;
            remote_binding = agent.remote_execution().cloned();
            if agent.session_id() != session_id {
                return Err(DaemonError::AgentNotInSession {
                    session_id: session_id.into(),
                    agent_id: agent_id.into(),
                });
            }
        }
        let activity_mutation = self.begin_managed_activity_mutation();
        let mut sessions = self.session_store.write();
        let mut session = sessions.get_session(session_id)?.clone();
        if terminal_credential_owner.is_some_and(|owner| {
            owner != session.owner_user_id()
                || kernel_operation_owner.is_some()
                || interaction.agent_id().is_none()
                || !interaction
                    .timeout_sec()
                    .is_some_and(|seconds| (1..=3600).contains(&seconds))
        }) {
            return Err(interaction_error("Invalid terminal credential prompt"));
        }
        match (interaction.agent_id(), kernel_operation_owner) {
            (Some(_), None) => {}
            (None, Some(owner)) if interaction.kernel_operation_id().is_some() => {
                if !session.has_member(owner)
                    || interaction.kind() != crate::session::RuntimeInteractionKind::Permission
                    || interaction.default_on_timeout().is_some()
                    || interaction.custom_choice().is_some()
                    || !interaction.timeout_sec().is_some_and(|seconds| {
                        seconds >= 1
                            && (seconds <= 3600
                                || interaction
                                    .kernel_operation_id()
                                    .is_some_and(|id| id.starts_with("access-")))
                    })
                {
                    return Err(interaction_error("Invalid kernel operation decision"));
                }
            }
            _ => {
                return Err(interaction_error(
                    "Interaction subject does not match its registration authority",
                ))
            }
        }
        // MP-11: bind both review forms to the enrolled human, while leaving
        // every other credential/operation owner and session membership intact.
        let review_owner =
            if super::super::owner_context_review::is_owner_context_review(&interaction) {
                if kernel_operation_owner.or(terminal_credential_owner)
                    != Some(session.owner_user_id())
                {
                    return Err(interaction_error("Copy review requires the session owner"));
                }
                Some(self.owner_context_review_owner(session.owner_user_id()))
            } else {
                None
            };
        let kernel_operation_owner =
            kernel_operation_owner.map(|owner| review_owner.as_deref().unwrap_or(owner));
        let terminal_credential_owner =
            terminal_credential_owner.map(|owner| review_owner.as_deref().unwrap_or(owner));
        // A kernel decision nobody waits for any more does not block its
        // subject: the new request supersedes it. The pump's sweep times such
        // decisions out too, but a registration must not depend on it having
        // run first, and a decision restored after a restart has no pending
        // entry, so no sweep ever sees it and no one can ever resolve it.
        // Superseding comes before the limits, which count what remains.
        if let Some(owner) = kernel_operation_owner {
            let mut pending = self.pending_interactions.write();
            let superseded = session
                .active_interactions()
                .iter()
                .filter(|existing| {
                    existing.kernel_operation_id().is_some()
                        && existing.subject() == interaction.subject()
                        && pending.get(existing.id()).is_none_or(|entry| {
                            entry.kernel_operation_owner.as_deref() == Some(owner)
                                && entry.nobody_waits()
                        })
                })
                .map(|existing| existing.id().to_owned())
                .collect::<Vec<_>>();
            for id in superseded {
                session.remove_active_interaction(&id);
                pending.remove(&id);
            }
            drop(pending);
            self.ensure_kernel_decision_capacity(owner)?;
        }
        if session
            .active_interactions()
            .iter()
            .any(|existing| existing.subject() == interaction.subject())
            || self
                .pending_interactions
                .write()
                .contains_key(interaction.id())
        {
            return Err(interaction_error(super::INTERACTION_ALREADY_PENDING));
        }
        let agent_lifetime = interaction.agent_id().map(|agent_id| {
            if let Some(origin) = interaction.native_origin() {
                use crate::session::NativeInteractionOrigin;
                let (mut prompt_id, native_turn_id) = match origin {
                    NativeInteractionOrigin::Prompt { prompt_id, .. } => {
                        (Some(prompt_id.clone()), None)
                    }
                    NativeInteractionOrigin::NativeTurn { native_turn_id, .. } => {
                        (None, Some(native_turn_id.clone()))
                    }
                    NativeInteractionOrigin::ProviderStartup { .. } => (None, None),
                };
                let worker = forwarding.map(|context| {
                    super::super::pending_runtime_state::PendingWorkerInteractionLifetime {
                        leased_agent_id: context.leased_agent_id.clone(),
                        execution_lease_id: remote_binding
                            .as_ref()
                            .map(|remote| remote.execution_lease_id.clone())
                            .unwrap_or_default(),
                        provider_run_id: context.worker_provider_run_id.clone(),
                        binding_observed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                            false,
                        )),
                    }
                });
                // Startup dialogs can precede the dispatch ACK and even the worker
                // prompt registration. Fence that launch to its pending home dispatch.
                if worker.is_some()
                    && matches!(origin, NativeInteractionOrigin::ProviderStartup { .. })
                {
                    prompt_id = forwarding
                        .and_then(|context| context.home_prompt_id.clone())
                        .or_else(|| {
                            self.prompt_state_owner
                                .active_prompt_for_agent(&session, agent_id)
                                .filter(|prompt| prompt.delivery_pending())
                                .map(|prompt| prompt.id().to_owned())
                        });
                }
                return super::super::PendingAgentInteractionLifetime {
                    agent_id: agent_id.into(),
                    prompt_id,
                    native_turn_id,
                    provider_run_id: worker.is_none().then(|| origin.provider_run_id().into()),
                    worker,
                };
            }
            let prompt_id = self
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
                .map(|prompt| prompt.id().to_owned());
            let provider_run_id = self
                .provider_store
                .get_run_for_agent(session_id, agent_id)
                .filter(|run| run.state() == crate::provider::ProviderRunState::Running)
                .map(|run| run.id().to_owned());
            let native_turn_id = if prompt_id.is_none() {
                provider_run_id
                    .as_deref()
                    .and_then(|id| self.active_turns.get(id))
                    .map(|turn| turn.prompt_id)
            } else {
                None
            };
            super::super::PendingAgentInteractionLifetime {
                agent_id: agent_id.into(),
                prompt_id,
                native_turn_id,
                provider_run_id,
                worker: None,
            }
        });
        // Protocol 403: a decision that needs the passkey is also a popup on
        // every terminal of its owner.
        let passkey_prompt = super::super::passkey_prompts::passkey_prompt(
            &session,
            &interaction,
            crate::session::unix_epoch_ms(),
        )?;
        let pending = super::super::PendingInteraction {
            kernel_wide_interaction: None,
            agent_lifetime,
            session_id: session_id.into(),
            session_store_identity: self.session_store.weak_identity(),
            kernel_operation_owner: kernel_operation_owner.map(str::to_owned),
            terminal_credential_owner: terminal_credential_owner.map(str::to_owned),
            kernel_operation_deadline: kernel_operation_owner.map(|_| {
                std::time::Instant::now()
                    + std::time::Duration::from_secs(
                        interaction
                            .timeout_sec()
                            .expect("validated decision timeout"),
                    )
            }),
            passkey_prompt: passkey_prompt.clone(),
            responder: std::sync::Arc::new(std::sync::Mutex::new(Some(responder))),
        };
        let worker_live = pending.agent_lifetime.as_ref().is_none_or(|lifetime| {
            self.worker_interaction_binding_is_live(lifetime, remote_binding.as_ref(), &session)
        });
        if !worker_live || !self.agent_interaction_turn_is_live(&pending, &session) {
            if let Some(sender) = pending
                .responder
                .lock()
                .expect("interaction responder")
                .take()
            {
                let _ = sender.send(super::super::PendingInteractionResolution {
                    status: "timed_out",
                    choice_id: None,
                    reply: None,
                });
            }
            return Ok(());
        }
        session.add_active_interaction(interaction.clone());
        let identity = pending.responder.clone();
        if passkey_prompt.is_some() {
            self.passkey_prompts
                .forget_answered(session_id, interaction.id());
        }
        self.pending_interactions
            .write()
            .insert(interaction.id().into(), pending);
        if passkey_prompt.is_some() {
            self.passkey_prompts.record_change();
        }
        sessions.restore_session(session);
        activity_mutation.record();
        drop(sessions);
        if let Err(error) = self.session_snapshot(session_id) {
            let mut pending = self.pending_interactions.write();
            if pending
                .get(interaction.id())
                .is_some_and(|current| std::sync::Arc::ptr_eq(&current.responder, &identity))
            {
                pending.remove(interaction.id());
            }
            drop(pending);
            // Projection can fail after publication (for example when the
            // durable writer is fenced). Remove only this failed registration,
            // under the same session authority; preserve intervening changes.
            let mut sessions = self.session_store.write();
            if let Ok(mut session) = sessions.get_session(session_id) {
                if session
                    .active_interactions()
                    .iter()
                    .any(|current| current == &interaction)
                {
                    session.remove_active_interaction(interaction.id());
                    sessions.restore_session(session);
                }
            }
            if passkey_prompt.is_some() {
                self.passkey_prompts.record_change();
            }
            return Err(error);
        }
        self.terminal_stream
            .notify_terminal_projection_change(session_id);
        Ok(())
    }
}
