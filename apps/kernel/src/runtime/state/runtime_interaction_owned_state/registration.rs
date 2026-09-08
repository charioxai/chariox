use super::*;

impl KernelRuntimeOwnedState {
    pub(in crate::runtime::state) fn register_runtime_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        responder: tokio::sync::oneshot::Sender<super::super::PendingInteractionResolution>,
        kernel_operation_owner: Option<&str>,
    ) -> Result<(), DaemonError> {
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
        // Resolve agent identity before taking the session write guard; session
        // updates must not acquire the agent store in the opposite lock order.
        if let Some(agent_id) = interaction.agent_id() {
            let agent = self.agent_store.get_agent(agent_id)?;
            if agent.session_id() != session_id {
                return Err(DaemonError::AgentNotInSession {
                    session_id: session_id.into(),
                    agent_id: agent_id.into(),
                });
            }
        }
        let mut sessions = self.session_store.write();
        let mut session = sessions.get_session(session_id)?.clone();
        match (interaction.agent_id(), kernel_operation_owner) {
            (Some(_), None) => {}
            (None, Some(owner)) if interaction.kernel_operation_id().is_some() => {
                if !session.has_member(owner)
                    || interaction.kind() != crate::session::RuntimeInteractionKind::Permission
                    || interaction.default_on_timeout().is_some()
                    || interaction.custom_choice().is_some()
                    || !interaction
                        .timeout_sec()
                        .is_some_and(|seconds| (1..=3600).contains(&seconds))
                {
                    return Err(interaction_error("Invalid kernel operation decision"));
                }
                let pending = self.pending_interactions.write();
                if pending
                    .values()
                    .filter(|p| p.belongs_to(&self.session_store))
                    .filter(|p| p.kernel_operation_owner.is_some())
                    .count()
                    >= 32
                    || pending
                        .values()
                        .filter(|p| p.belongs_to(&self.session_store))
                        .filter(|p| p.kernel_operation_owner.as_deref() == Some(owner))
                        .count()
                        >= 8
                {
                    return Err(interaction_error("Kernel decision limit reached"));
                }
            }
            _ => {
                return Err(interaction_error(
                    "Interaction subject does not match its registration authority",
                ))
            }
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
            return Err(interaction_error(
                "Interaction subject or identity is already pending",
            ));
        }
        session.add_active_interaction(interaction.clone());
        let pending = super::super::PendingInteraction {
            session_id: session_id.into(),
            session_store_identity: self.session_store.weak_identity(),
            kernel_operation_owner: kernel_operation_owner.map(str::to_owned),
            kernel_operation_deadline: kernel_operation_owner.map(|_| {
                std::time::Instant::now()
                    + std::time::Duration::from_secs(
                        interaction
                            .timeout_sec()
                            .expect("validated decision timeout"),
                    )
            }),
            responder: std::sync::Arc::new(std::sync::Mutex::new(Some(responder))),
        };
        let identity = pending.responder.clone();
        self.pending_interactions
            .write()
            .insert(interaction.id().into(), pending);
        sessions.restore_session(session);
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
            return Err(error);
        }
        self.terminal_stream
            .notify_terminal_projection_change(session_id);
        Ok(())
    }
}
