//! MP-08 / MP-10 / MP-11 A07: write-ahead exclusive owner claims.
use super::*;
pub(super) struct HandoffClaimGuard {
    claims: Arc<std::sync::Mutex<BTreeSet<String>>>,
    id: String,
}
impl Drop for HandoffClaimGuard {
    fn drop(&mut self) {
        if let Ok(mut claims) = self.claims.lock() {
            claims.remove(&self.id);
        }
    }
}

impl KernelRuntimeOwnedState {
    /// Atomically claim a pending hand-off for its owner, before any I/O. The
    /// popup closes on every terminal; a later answer is told it was already
    /// answered. `check` refuses an action the hand-off does not offer
    /// without consuming it.
    pub(super) fn claim_handoff(
        &self,
        session_id: &str,
        interaction_id: &str,
        caller_user_id: &str,
        check: impl FnOnce(&RuntimeHandoff) -> Result<(), DaemonError>,
    ) -> Result<(RuntimeHandoff, HandoffClaimGuard), DaemonError> {
        let _mutation = self
            .pending_interactions
            .mutation
            .lock()
            .map_err(|_| handoff_error("Interaction store is unavailable"))?;
        if !self
            .durable_state_store
            .load_subject_events_by_kind(interaction_id, "handoff.claimed", 1)?
            .is_empty()
        {
            return Err(handoff_error("already answered"));
        }
        let pending = self
            .pending_interactions
            .write()
            .get(interaction_id)
            .cloned()
            .filter(|p| p.session_id == session_id && p.belongs_to(&self.session_store))
            .ok_or_else(|| {
                if self
                    .passkey_prompts
                    .was_answered(session_id, interaction_id)
                {
                    super::critical_approval_passkey::passkey_error(
                        super::passkey_prompts::PASSKEY_ALREADY_ANSWERED,
                        "already answered",
                    )
                } else {
                    handoff_error("hand-off is not pending")
                }
            })?;
        // Local TUI and the configured Cloud owner are the same product owner.
        // Preserve that existing user-domain alias without admitting collaborators.
        let config = self.config_projection.snapshot();
        let caller_owner = crate::account_profile::provider_account_authority_owner_user_id(
            &config,
            caller_user_id,
        );
        if !pending
            .kernel_operation_owner
            .as_deref()
            .is_some_and(|owner| {
                crate::account_profile::provider_account_authority_owner_user_id(&config, owner)
                    == caller_owner
            })
        {
            return Err(handoff_error("Only the hand-off owner can answer it"));
        }
        if pending
            .kernel_operation_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            return Err(handoff_error("hand-off expired"));
        }
        let activity_mutation = self.begin_managed_activity_mutation();
        let mut sessions = self.session_store.write();
        let mut session = sessions.get_session(session_id)?.clone();
        let handoff = session
            .active_interactions()
            .iter()
            .find(|i| i.id() == interaction_id)
            .and_then(|i| i.handoff().cloned())
            .ok_or_else(|| handoff_error("hand-off is not pending"))?;
        if crate::session::unix_epoch_ms() >= handoff.expires_at_ms {
            return Err(handoff_error("hand-off expired"));
        }
        check(&handoff)?;
        // A write-ahead claim fences retries even if the kernel dies before
        // projecting removal or recording the physical outcome. No value is kept.
        let mut claims = self
            .handoff_claims
            .lock()
            .map_err(|_| handoff_error("claim store unavailable"))?;
        self.durable_state_store.append_event(
            "handoff.claimed",
            Some(interaction_id.into()),
            json!({"handoff":handoff,"room":session_id,"actor":caller_user_id}),
        )?;
        claims.insert(interaction_id.into());
        let guard = HandoffClaimGuard {
            claims: self.handoff_claims.clone(),
            id: interaction_id.into(),
        };
        drop(claims);
        session.remove_active_interaction(interaction_id);
        sessions.restore_session(session);
        activity_mutation.record();
        drop(sessions);
        self.pending_interactions.write().remove(interaction_id);
        // Dropping the responder tells the waiter the answer was claimed here.
        pending.responder.lock().expect("hand-off responder").take();
        self.passkey_prompts
            .record_answered(session_id, interaction_id);
        let _ = self.session_snapshot(session_id);
        self.terminal_stream
            .notify_terminal_projection_change(session_id);
        Ok((handoff, guard))
    }
}
