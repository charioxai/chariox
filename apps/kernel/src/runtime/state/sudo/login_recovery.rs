//! MP-08 / MP-10 / MP-11 V09: retain consent across an official login reload.
use super::*;

impl KernelRuntimeState {
    /// Retention only, never execution authority. The old run is no longer
    /// live; every existing effect check keeps rejecting it during the gap.
    pub(super) fn sudo_login_recovery_pending(&self, turn: &KernelSudoTurn) -> bool {
        let Some(run) = turn.provider_run_id.as_ref() else {
            return false;
        };
        let Some(prompt) = turn.prompt_id.as_deref() else {
            return false;
        };
        self.owned
            .provider_auth_recovery_runs
            .lock()
            .expect("recovery claims poisoned")
            .contains(run)
            && self.owned.sudo_end_reason(turn).is_none()
            && self
                .owned
                .session_store
                .get_session(&turn.session_id)
                .is_ok_and(|session| {
                    session.status() != SessionStatus::Ended
                        && self
                            .owned
                            .prompt_state_owner
                            .sudo_bound_prompt(&session, &turn.agent_id)
                            .is_some_and(|(entry, active)| {
                                entry == turn.entry_id && active == prompt
                            })
                })
    }

    pub(in crate::runtime::state) fn rebind_sudo_after_provider_login(
        &self,
        original: &KernelSudoTurn,
        run: &crate::provider::RuntimeProviderRun,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<bool, DaemonError> {
        if original.session_id != run.session_id()
            || run.agent_instance_id() != Some(original.agent_id.as_str())
            || run.state() != crate::provider::ProviderRunState::Running
            || original.prompt_id.as_deref() != Some(prompt.id())
            || !self.sudo_login_recovery_pending(original)
            || self
                .owned
                .provider_store
                .get_run_for_agent(run.session_id(), &original.agent_id)
                .is_none_or(|current| current.id() != run.id())
        {
            return Ok(false);
        }
        let mut turns = self.owned.sudo_turns.lock().expect("access state poisoned");
        let Some(current) = turns
            .get_mut(&original.entry_id)
            .filter(|current| **current == *original)
        else {
            return Ok(false);
        };
        let mut rebound = current.clone();
        rebound.provider_run_id = Some(run.id().into());
        self.audit_sudo(&rebound, "started")?;
        *current = rebound;
        Ok(true)
    }
}
