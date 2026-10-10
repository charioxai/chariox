use super::*;

impl KernelRuntimeState {
    pub(crate) async fn answer_sudo_interaction(
        &self,
        id: &str,
        answer: RespondToInteractionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_sudo_request(
            id,
            &LocalDaemonRequest::RespondToInteraction(answer.clone()),
        )?;
        let turn = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .get(id)
            .cloned()
            .ok_or_else(|| error("sudo turn ended"))?;
        self.owned.resolve_runtime_interaction_authorized(
            &answer.session_id,
            &answer.interaction_id,
            &answer.choice_id,
            answer.custom_reply.as_deref(),
            Some(&turn.owner_user_id),
            true,
            Some(&turn),
            None,
            false,
            false,
        )?;
        Ok(LocalDaemonResponse::InteractionResponded {
            interaction_id: answer.interaction_id,
            session: self.session_snapshot(&answer.session_id).await?,
        })
    }
    pub(super) fn audit_sudo(
        &self,
        turn: &KernelSudoTurn,
        outcome: &str,
    ) -> Result<(), DaemonError> {
        if let Some(run) = turn.provider_run_id.as_deref() {
            self.owned
                .provider_run_projection
                .catalog_changes()
                .invalidate(run);
        }
        self.owned.durable_state_store.append_event(
            "kernel_access.sudo",
            Some(turn.entry_id.clone()),
            serde_json::json!({"outcome": outcome, "turn": turn}),
        )?;
        Ok(())
    }
}

pub(crate) fn sudo_approval_receipt(
    turn: &KernelSudoTurn,
    session: &str,
    interaction: &str,
    choice: &str,
) -> serde_json::Value {
    serde_json::json!({"outcome":"authorized", "turn":turn, "session_id":session, "interaction_id":interaction, "choice_id":choice})
}
