//! Provider-run focus and active-run projection mutations.
//!
//! This module owns synchronizing the session active provider pointer with focused agents, active
//! prompts, and parked/running provider runs.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn should_defer_provider_run_sync_for_focus_change(
        &self,
        session_id: &str,
        target_agent_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self.session_snapshot(session_id)?;
        let Some(active_provider_run_id) = session.active_provider_run_id().map(str::to_string)
        else {
            return Ok(false);
        };
        let active_run = self
            .provider_store
            .get_run(&active_provider_run_id)
            .or_else(|_| {
                self.provider_run_projection
                    .get(&active_provider_run_id)
                    .ok_or_else(|| DaemonError::ProviderRunNotFound {
                        provider_run_id: active_provider_run_id.clone(),
                    })
            })?;
        if active_run.agent_instance_id() == Some(target_agent_id)
            || active_run.state() != crate::provider::ProviderRunState::Running
        {
            return Ok(false);
        }

        Ok(self.prompt_state_owner.has_any_active_prompt(&session))
    }

    pub(super) fn sync_active_provider_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let current_active_run_id = self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);

        if let Some(current_active_run_id) = current_active_run_id.as_deref() {
            let active_run = self
                .provider_store
                .get_run(current_active_run_id)
                .or_else(|_| {
                    self.provider_run_projection
                        .get(current_active_run_id)
                        .ok_or_else(|| DaemonError::ProviderRunNotFound {
                            provider_run_id: current_active_run_id.to_string(),
                        })
                })?;
            if active_run.agent_instance_id() != Some(agent_id)
                && active_run.state() == crate::provider::ProviderRunState::Running
                && active_run.client_interface().is_arroba()
                && !self.provider_run_has_active_prompt(session_id, &active_run)?
            {
                let outcome = self
                    .provider_store
                    .park_run_provider_only(session_id, current_active_run_id)?;
                self.clear_active_provider_run_session_pointer(session_id, outcome.run().id())?;
                self.provider_run_projection.update(outcome.into_run());
            }
        }

        if let Some(agent_run) = self.provider_store.get_run_for_agent(session_id, agent_id) {
            match agent_run.state() {
                crate::provider::ProviderRunState::Running
                | crate::provider::ProviderRunState::Starting => {
                    self.session_store
                        .set_active_provider_run(session_id, Some(agent_run.id().to_string()))?;
                }
                crate::provider::ProviderRunState::Parked => {
                    let _ = self.resume_provider_run_for_session(session_id, agent_run.id())?;
                }
                crate::provider::ProviderRunState::Ended => {
                    self.session_store
                        .set_active_provider_run(session_id, None)?;
                }
            }
        } else if let Some(agent_run) = self
            .provider_run_projection
            .get_for_agent(session_id, agent_id)
        {
            match agent_run.state() {
                crate::provider::ProviderRunState::Running
                | crate::provider::ProviderRunState::Starting => {
                    self.session_store
                        .set_active_provider_run(session_id, Some(agent_run.id().to_string()))?;
                }
                crate::provider::ProviderRunState::Parked
                | crate::provider::ProviderRunState::Ended => {
                    self.session_store
                        .set_active_provider_run(session_id, None)?;
                }
            }
        } else {
            self.session_store
                .set_active_provider_run(session_id, None)?;
        }

        Ok(())
    }

    pub(super) fn sync_focused_provider_run_if_idle(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.session_snapshot(session_id)?;
        if session.agents().len() > 1 {
            let focused_agent_id = session.focused_agent_id().map(str::to_string);
            if let Some(focused_agent_id) = focused_agent_id {
                let active_prompt_agent_id =
                    self.prompt_state_owner.active_prompt_agent_id(&session);
                let has_active_prompt = self.prompt_state_owner.has_any_active_prompt(&session);
                if !has_active_prompt {
                    let current_active_run_id =
                        session.active_provider_run_id().map(str::to_string);
                    if let Some(current_active_run_id) = current_active_run_id.as_deref() {
                        let active_run = self
                            .provider_store
                            .get_run(current_active_run_id)
                            .or_else(|_| {
                                self.provider_run_projection
                                    .get(current_active_run_id)
                                    .ok_or_else(|| DaemonError::ProviderRunNotFound {
                                        provider_run_id: current_active_run_id.to_string(),
                                    })
                            })?;
                        if active_run.agent_instance_id() != Some(focused_agent_id.as_str())
                            && active_run.state() == crate::provider::ProviderRunState::Running
                            && active_run.client_interface().is_arroba()
                            && !self.provider_run_has_active_prompt(session_id, &active_run)?
                        {
                            let outcome = self
                                .provider_store
                                .park_run_provider_only(session_id, current_active_run_id)?;
                            self.clear_active_provider_run_session_pointer(
                                session_id,
                                outcome.run().id(),
                            )?;
                            self.provider_run_projection.update(outcome.into_run());
                        }
                    }
                }
                if has_active_prompt {
                    if let Some(projected_agent_id) = active_prompt_agent_id.as_deref() {
                        self.project_active_provider_run_for_agent(session_id, projected_agent_id)?;
                    }
                } else {
                    self.sync_active_provider_run_for_agent(session_id, &focused_agent_id)?;
                }
            } else {
                self.session_store
                    .set_active_provider_run(session_id, None)?;
            }
            return Ok(());
        }

        if self.prompt_state_owner.has_any_active_prompt(&session) {
            return Ok(());
        }

        if let Some(focused_agent_id) = session.focused_agent_id() {
            self.sync_active_provider_run_for_agent(session_id, focused_agent_id)?;
        } else {
            self.session_store
                .set_active_provider_run(session_id, None)?;
        }
        Ok(())
    }

    pub(super) fn project_active_provider_run_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let projected_run_id = self
            .provider_store
            .get_run_for_agent(session_id, agent_id)
            .or_else(|| {
                self.provider_run_projection
                    .get_for_agent(session_id, agent_id)
            })
            .and_then(|run| match run.state() {
                crate::provider::ProviderRunState::Running
                | crate::provider::ProviderRunState::Starting => Some(run.id().to_string()),
                crate::provider::ProviderRunState::Parked
                | crate::provider::ProviderRunState::Ended => None,
            });
        self.session_store
            .set_active_provider_run(session_id, projected_run_id)?;
        Ok(())
    }

    pub(super) fn clear_active_provider_run_session_pointer(
        &self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        if self
            .session_store
            .get_session(session_id)?
            .active_provider_run_id()
            == Some(provider_run_id)
        {
            self.session_store
                .set_active_provider_run(session_id, None)?;
        }
        Ok(())
    }

    pub(super) fn provider_run_has_active_prompt(
        &self,
        session_id: &str,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) -> Result<bool, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(false);
        };
        let session = self.session_store.get_session(session_id)?;
        Ok(self
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, agent_id)
            .is_some())
    }
}
