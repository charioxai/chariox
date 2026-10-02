//! Agent decisions belong to the turn that raised them, without a wall-clock cutoff.
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn agent_interaction_is_live(
        &self,
        pending: &super::super::PendingInteraction,
    ) -> bool {
        let Some(lifetime) = &pending.agent_lifetime else {
            return true;
        };
        let Ok(agent) = self.agent_store.get_agent(&lifetime.agent_id) else {
            return false;
        };
        if agent.session_id() != pending.session_id {
            return false;
        }
        let Ok(session) = self.session_store.get_session(&pending.session_id) else {
            return false;
        };
        self.agent_interaction_turn_is_live(pending, &session)
    }

    pub(super) fn agent_interaction_turn_is_live(
        &self,
        pending: &super::super::PendingInteraction,
        session: &crate::session::RuntimeSession,
    ) -> bool {
        let Some(lifetime) = &pending.agent_lifetime else {
            return true;
        };
        if session.status() == crate::session::SessionStatus::Ended {
            return false;
        }
        if let Some(prompt_id) = &lifetime.prompt_id {
            if !self
                .prompt_state_owner
                .active_prompt_for_agent(session, &lifetime.agent_id)
                .is_some_and(|prompt| {
                    prompt.id() == prompt_id
                        && matches!(
                            prompt.status(),
                            crate::session::PromptStatus::Running
                                | crate::session::PromptStatus::Dispatching
                        )
                })
            {
                return false;
            }
        }
        if let Some(turn_id) = &lifetime.native_turn_id {
            if !lifetime
                .provider_run_id
                .as_deref()
                .and_then(|id| self.active_turns.get(id))
                .is_some_and(|turn| turn.prompt_id == *turn_id)
            {
                return false;
            }
        }
        if let Some(run_id) = &lifetime.provider_run_id {
            if !self
                .provider_store
                .get_run(run_id)
                .is_ok_and(|run| run.state() == crate::provider::ProviderRunState::Running)
            {
                return false;
            }
        }
        true
    }

    /// Also used by the pump for exits, abandoned callers, and non-prompt decisions.
    pub(in crate::runtime::state) fn withdraw_stale_agent_interactions(&self) {
        let Ok(_mutation) = self.pending_interactions.mutation.lock() else {
            return;
        };
        let candidates = self
            .pending_interactions
            .write()
            .iter()
            .filter(|(_, pending)| {
                pending.belongs_to(&self.session_store) && pending.agent_lifetime.is_some()
            })
            .map(|(id, pending)| (id.clone(), pending.clone()))
            .collect::<Vec<_>>();
        for (id, pending) in candidates {
            let abandoned = pending.responder.lock().map_or(true, |sender| {
                sender.as_ref().is_none_or(|sender| sender.is_closed())
            });
            if abandoned || !self.agent_interaction_is_live(&pending) {
                let _ = self.withdraw_agent_interaction_locked(&id, &pending);
            }
        }
    }

    pub(in crate::runtime::state) fn withdraw_agent_interactions(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let _mutation = self
            .pending_interactions
            .mutation
            .lock()
            .map_err(|_| interaction_error("Interaction store is unavailable"))?;
        let candidates = self
            .pending_interactions
            .write()
            .iter()
            .filter(|(_, pending)| {
                pending.belongs_to(&self.session_store) && pending.session_id == session_id
            })
            .filter(|(_, pending)| {
                pending
                    .agent_lifetime
                    .as_ref()
                    .is_some_and(|lifetime| agent_id.is_none_or(|id| id == lifetime.agent_id))
            })
            .map(|(id, pending)| (id.clone(), pending.clone()))
            .collect::<Vec<_>>();
        for (id, pending) in candidates {
            self.withdraw_agent_interaction_locked(&id, &pending)?;
        }
        Ok(())
    }

    // Caller holds the shared interaction mutation guard. Never apply timeout defaults.
    pub(super) fn withdraw_agent_interaction_locked(
        &self,
        id: &str,
        pending: &super::super::PendingInteraction,
    ) -> Result<(), DaemonError> {
        self.pending_interactions.write().remove(id);
        let activity_mutation = self.begin_managed_activity_mutation();
        let mut sessions = self.session_store.write();
        if let Ok(mut session) = sessions.get_session(&pending.session_id) {
            session.remove_active_interaction(id);
            sessions.restore_session(session);
        }
        activity_mutation.record();
        drop(sessions);
        if let Some(sender) = pending
            .responder
            .lock()
            .expect("pending interaction responder mutex poisoned")
            .take()
        {
            let _ = sender.send(super::super::PendingInteractionResolution {
                status: "timed_out",
                choice_id: None,
                reply: None,
            });
        }
        self.terminal_stream
            .notify_terminal_projection_change(&pending.session_id);
        if self.session_store.get_session(&pending.session_id).is_ok() {
            self.record_notice_for_agent(
                &pending.session_id,
                None,
                pending.agent_lifetime.as_ref().map(|l| l.agent_id.as_str()),
                self.attachment_store.list_session_attachment_ids(&pending.session_id),
                format!("Approval `{id}` withdrawn because its turn or agent ended, or its caller closed. Late answers are refused."),
            );
            self.session_snapshot(&pending.session_id)?;
        }
        Ok(())
    }
}
