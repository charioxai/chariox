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
        self.sweep_stale_task_owner_interactions()?;
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
        // must never prompt the human for it. A card payment the kernel raises
        // in the asking agent's Room is the kernel's own decision.
        let payment = super::super::vault_use::is_vault_payment_decision(interaction.id())
            && interaction.payment().is_some();
        if interaction.agent_id().is_some()
            && !payment
            && interaction
                .choices()
                .iter()
                .any(crate::session::RuntimeInteractionChoice::requires_passkey)
        {
            return Err(interaction_error(
                "Only kernel decisions can require the Chariox passkey",
            ));
        }
        if session_id.is_empty() {
            if forwarding.is_some() || terminal_credential_owner.is_some() {
                return Err(interaction_error("Invalid unattached decision authority"));
            }
            return self.register_user_domain_interaction(
                interaction,
                responder,
                kernel_operation_owner,
            );
        }
        if session_id == crate::runtime::kernel_access::ACCESS_INTERACTION_SCOPE {
            if interaction
                .kernel_operation_id()
                .is_some_and(|id| id.starts_with(crate::runtime::state::VAULT_SECRET_OPERATION))
            {
                return self.register_kernel_wide_secret_interaction(
                    interaction,
                    responder,
                    kernel_operation_owner,
                    forwarding,
                    terminal_credential_owner,
                );
            }
            return self.register_kernel_wide_access_interaction(
                interaction,
                responder,
                kernel_operation_owner,
                forwarding,
                terminal_credential_owner,
            );
        }
        if let Some(task_id) = interaction.asking_task_id() {
            let wake = self
                .durable_state_store
                .agent_wake(interaction.id())?
                .ok_or_else(|| interaction_error("Async question has no admitted wake"))?;
            if wake.kind != super::super::interaction_wake::KIND
                || wake.task_id != task_id
                || wake.room_id != session_id
                || interaction.agent_id() != Some(wake.agent_id.as_str())
                || !matches!(wake.state.as_str(), "starting" | "running")
                || serde_json::to_value(&interaction).ok().as_ref() != Some(&wake.detail)
            {
                return Err(interaction_error(
                    "Async question does not match its admitted wake",
                ));
            }
        }
        // Kernel action cards keep their human authority and also belong to
        // the requesting agent's exact prompt/run.
        let lifetime_agent = interaction.agent_id().or_else(|| {
            interaction
                .native_origin()
                .and(interaction.related_agent_id())
        });
        // The agent a card brings back is whichever one it concerns, with or without prompt-lifetime authority.
        let concerned_agent = interaction
            .agent_id()
            .or(interaction.related_agent_id())
            .map(str::to_owned);
        // Resolve agent identity before taking the session write guard; session
        // updates must not acquire the agent store in the opposite lock order.
        let mut remote_binding = None;
        if let Some(agent_id) = lifetime_agent {
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
                if !self.is_room_member(&session, owner)
                    || interaction.kind() != crate::session::RuntimeInteractionKind::Permission
                    || interaction.default_on_timeout().is_some()
                    || interaction.custom_choice().is_some()
                    || !match interaction.timeout_sec() {
                        Some(seconds) => {
                            seconds >= 1
                                && (seconds <= 3600
                                    || interaction
                                        .kernel_operation_id()
                                        .is_some_and(|id| id.starts_with("access-")))
                        }
                        // An App install waits for its owner for as long as it takes; so does a blocked task's decision.
                        None => interaction.app_install().is_some()
                            || super::super::agent_task_owner_interactions::is_blocked_task_decision(
                                interaction.id(),
                            )
                                && interaction.kernel_operation_id() == Some(interaction.id()),
                    }
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
        // MP-08 / MP-10 / MP-11: a restart or cancelled launch can leave a
        // projected Vault card without a responder. Retire only an abandoned
        // Vault card for this subject; never replace a live decision.
        if let Some(owner) =
            terminal_credential_owner.filter(|_| interaction.id().starts_with("vault-unlock-"))
        {
            let mut pending = self.pending_interactions.write();
            let abandoned = session
                .active_interactions()
                .iter()
                .filter(|existing| {
                    existing.subject() == interaction.subject()
                        && existing.id().starts_with("vault-unlock-")
                        && pending.get(existing.id()).is_none_or(|entry| {
                            !entry.belongs_to(&self.session_store)
                                || (entry.terminal_credential_owner.as_deref() == Some(owner)
                                    && entry.nobody_waits())
                        })
                })
                .map(|existing| existing.id().to_owned())
                .collect::<Vec<_>>();
            for id in abandoned {
                if let Some(previous) = session.remove_active_interaction(&id) {
                    self.publish_outside_interaction(
                        session_id,
                        &previous,
                        InteractionPhase::cancelled(),
                    )?;
                }
                if pending
                    .get(&id)
                    .is_some_and(|entry| entry.belongs_to(&self.session_store))
                {
                    pending.remove(&id);
                }
            }
        }
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
            for id in &superseded {
                if let Some(previous) = session.remove_active_interaction(id) {
                    self.publish_outside_interaction(
                        session_id,
                        &previous,
                        InteractionPhase::cancelled(),
                    )?;
                }
                pending.remove(id);
            }
            if !superseded.is_empty() {
                // This cleanup is definitive even when the new admission is refused.
                sessions.restore_session(session.clone());
            }
            drop(pending);
            if let Err(error) = self.ensure_kernel_decision_capacity(owner, &interaction) {
                drop(sessions);
                activity_mutation.record();
                self.session_snapshot(session_id)?;
                self.terminal_stream
                    .notify_terminal_projection_change(session_id);
                return Err(error);
            }
        }
        // Where the user sees a card is the kernel's: a restored question is the same question
        // whichever panel lists it.
        if session.active_interactions().iter().any(|existing| {
            (existing.id() == interaction.id()
                && existing.clone().shown_in(None) != interaction.clone().shown_in(None))
                || (existing.subject() == interaction.subject()
                    && existing.asking_task_id().is_none()
                    && interaction.asking_task_id().is_none())
        }) || self
            .pending_interactions
            .write()
            .contains_key(interaction.id())
        {
            return Err(interaction_error(super::INTERACTION_ALREADY_PENDING));
        }
        let agent_lifetime = lifetime_agent.map(|agent_id| {
            if interaction.asking_task_id().is_some() {
                return super::super::PendingAgentInteractionLifetime {
                    agent_id: agent_id.into(),
                    prompt_id: None,
                    native_turn_id: None,
                    provider_run_id: None,
                    worker: None,
                };
            }
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
                        startup_permission: interaction.kind()
                            == crate::session::RuntimeInteractionKind::Permission
                            && matches!(origin, NativeInteractionOrigin::ProviderStartup { .. }),
                        binding_observed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                            // MP-08 / MP-10 / MP-11: Kernel choices raised by
                            // an idle worker need an already live run. Active
                            // dispatches and native startup permissions retain
                            // the pre-ACK allowance.
                            interaction.kind()
                                != crate::session::RuntimeInteractionKind::Permission
                                && matches!(
                                    origin,
                                    NativeInteractionOrigin::ProviderStartup { .. }
                                )
                                && self
                                    .prompt_state_owner
                                    .active_prompt_for_agent(&session, agent_id)
                                    .is_none(),
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
        let mut pending = super::super::PendingInteraction {
            checking_answer: Default::default(),
            goal_approval: None,
            async_task_id: interaction.asking_task_id().map(str::to_owned),
            kernel_wide_interaction: None,
            agent_lifetime,
            session_id: session_id.into(),
            session_store_identity: self.session_store.weak_identity(),
            user_domain_interaction: None,
            kernel_operation_owner: kernel_operation_owner.map(str::to_owned),
            terminal_credential_owner: terminal_credential_owner.map(str::to_owned),
            // A card payment ends at its own deadline like a kernel operation; its popup lives as
            // long as the card.
            kernel_operation_deadline: (kernel_operation_owner.is_some() || payment)
                .then(|| interaction.timeout_sec())
                .flatten()
                .map(|seconds| std::time::Instant::now() + std::time::Duration::from_secs(seconds)),
            passkey_prompt: passkey_prompt.clone(),
            holder: super::super::DecisionHolder::User,
            holder_tenure: 0,
            responder: std::sync::Arc::new(std::sync::Mutex::new(Some(responder))),
        };
        pending.holder = self.decision_holder_at_raise(&session, &pending, &interaction);
        // Registered again after a restart, a question keeps the holder it was handed to.
        if let Some((holder, tenure)) =
            self.restored_decision_holder(&session, &pending, &interaction)?
        {
            pending.holder = holder;
            pending.holder_tenure = tenure;
        }
        // The user sees a sub-agent's decision in its top parent's panel from the start, also while
        // a parent agent holds it: the first answer, the holder's or the user's, wins.
        let shown_in = self.decision_shown_in(session_id, &interaction);
        let interaction = interaction.shown_in(shown_in);
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
        if pending.terminal_credential_owner.is_none() && pending.kernel_operation_owner.is_none() {
            pending.goal_approval = self.begin_goal_native_approval(
                session_id,
                &interaction,
                pending.agent_lifetime.as_ref(),
            )?;
        }
        if let Err(error) = self.publish_outside_interaction(
            session_id,
            &interaction,
            InteractionPhase::Pending(&pending),
        ) {
            self.answer_goal_native_approval(pending.goal_approval.as_ref())?;
            return Err(error);
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
        drop(sessions);
        activity_mutation.record();
        // Nobody looks at a hidden agent's card, so the agent returns with it.
        if let Some(agent_id) = concerned_agent.as_deref().filter(|id| {
            self.agent_store
                .get_agent(id)
                .is_ok_and(|a| a.session_id() == session_id)
        }) {
            if let Err(error) = self.unhide_agent(agent_id) {
                crate::logging::error_with_fields(
                    "kernel.agents",
                    "a hidden agent's decision could not bring it back",
                    serde_json::json!({ "agent_id": agent_id, "error": error.to_string() }),
                );
            }
        }
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
                    if self
                        .publish_outside_interaction(
                            session_id,
                            &interaction,
                            InteractionPhase::cancelled(),
                        )
                        .is_err()
                    {
                        crate::logging::error_with_fields(
                            "kernel.events",
                            "failed interaction registration could not publish its withdrawal",
                            serde_json::json!({}),
                        );
                    }
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
