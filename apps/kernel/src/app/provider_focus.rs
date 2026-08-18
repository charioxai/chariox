use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{ProviderRunState, RuntimeProviderRun};
use crate::runtime::projection::projected_active_provider_run_id;
use crate::session::RuntimeSession;

use super::provider_activation::ProviderRunActivationState;
use super::provider_liveness::clear_active_provider_run_session_pointer;

impl DaemonApp {
    pub(crate) fn project_session_runtime_view(&self, session: &mut RuntimeSession) {
        let prompt_session = session.clone();
        let active_prompt_agent_id = self.prompt_state_owner.active_prompt_agent_id(session);
        let projected_run_id = projected_active_provider_run_id(
            session,
            |provider_run_id| {
                self.providers
                    .get_run(provider_run_id)
                    .ok()
                    .or_else(|| self.provider_run_projection.get(provider_run_id))
            },
            |agent_id| {
                self.providers
                    .get_run_for_agent(session.id(), agent_id)
                    .or_else(|| {
                        self.provider_run_projection
                            .get_for_agent(session.id(), agent_id)
                    })
            },
            |agent_id| {
                self.prompt_state_owner
                    .active_prompt_for_agent_snapshot(&prompt_session, agent_id)
                    .is_some()
            },
            active_prompt_agent_id,
        );
        session.set_active_provider_run(projected_run_id);
    }

    pub(crate) fn project_active_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let projected_run_id = self
            .providers
            .get_run_for_agent(session_id, agent_id)
            .or_else(|| {
                self.provider_run_projection
                    .get_for_agent(session_id, agent_id)
            })
            .and_then(|run| match run.state() {
                ProviderRunState::Running | ProviderRunState::Starting => {
                    Some(run.id().to_string())
                }
                ProviderRunState::Parked | ProviderRunState::Ended => None,
            });
        let _ = self
            .sessions
            .set_active_provider_run(session_id, projected_run_id)?;
        Ok(())
    }

    pub(crate) fn sync_active_provider_run_for_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let current_active_run_id = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);

        if let Some(current_active_run_id) = current_active_run_id.as_deref() {
            let active_run = self.providers.get_run(current_active_run_id).or_else(|_| {
                self.provider_run_projection
                    .get(current_active_run_id)
                    .ok_or_else(|| DaemonError::ProviderRunNotFound {
                        provider_run_id: current_active_run_id.to_string(),
                    })
            })?;
            if active_run.agent_instance_id() != Some(agent_id)
                && active_run.state() == ProviderRunState::Running
                && active_run.client_interface().is_chariox()
                && !self.provider_run_has_active_prompt(session_id, &active_run)?
            {
                let outcome = self
                    .providers
                    .park_run_provider_only(session_id, current_active_run_id)?;
                clear_active_provider_run_session_pointer(self, session_id, outcome.run().id())?;
                self.update_provider_run_projection(outcome.into_run());
            }
        }

        if let Some(agent_run) = self
            .providers
            .get_run_for_agent(session_id, agent_id)
            .or_else(|| {
                self.provider_run_projection
                    .get_for_agent(session_id, agent_id)
            })
        {
            match agent_run.state() {
                ProviderRunState::Running => {
                    self.sessions
                        .set_active_provider_run(session_id, Some(agent_run.id().to_string()))?;
                }
                ProviderRunState::Parked => {
                    if self.providers.get_run(agent_run.id()).is_ok() {
                        ProviderRunActivationState::resume_provider_run_for_session(
                            self,
                            session_id,
                            agent_run.id(),
                        )?;
                    } else {
                        self.sessions.set_active_provider_run(session_id, None)?;
                    }
                }
                ProviderRunState::Starting => {
                    self.sessions
                        .set_active_provider_run(session_id, Some(agent_run.id().to_string()))?;
                }
                ProviderRunState::Ended => {
                    self.sessions.set_active_provider_run(session_id, None)?;
                }
            }
        } else {
            self.sessions.set_active_provider_run(session_id, None)?;
        }

        Ok(())
    }

    pub(crate) fn should_defer_provider_run_sync_for_focus_change(
        &self,
        session_id: &str,
        target_agent_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let Some(active_provider_run_id) = session.active_provider_run_id().map(str::to_string)
        else {
            return Ok(false);
        };
        let active_run = self
            .providers
            .get_run(&active_provider_run_id)
            .or_else(|_| {
                self.provider_run_projection
                    .get(&active_provider_run_id)
                    .ok_or_else(|| DaemonError::ProviderRunNotFound {
                        provider_run_id: active_provider_run_id.clone(),
                    })
            })?;
        if active_run.agent_instance_id() == Some(target_agent_id)
            || active_run.state() != ProviderRunState::Running
        {
            return Ok(false);
        }

        Ok(self.prompt_state_owner.has_any_active_prompt(&session))
    }

    pub(crate) fn sync_focused_provider_run_if_idle(
        &mut self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.sessions.get_session(session_id)?;
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
                        match self.providers.get_run(current_active_run_id) {
                            Ok(active_run) => {
                                let active_run_is_remote = active_run
                                    .agent_instance_id()
                                    .and_then(|agent_id| self.agents.get_agent(agent_id).ok())
                                    .is_some_and(|agent| agent.remote_execution().is_some());
                                if active_run_is_remote {
                                    self.sessions.set_active_provider_run(session_id, None)?;
                                } else if active_run.agent_instance_id()
                                    != Some(focused_agent_id.as_str())
                                    && active_run.state() == ProviderRunState::Running
                                    && !self
                                        .provider_run_has_active_prompt(session_id, &active_run)?
                                {
                                    match self
                                        .providers
                                        .park_run_provider_only(session_id, current_active_run_id)
                                    {
                                        Ok(outcome) => {
                                            clear_active_provider_run_session_pointer(
                                                self,
                                                session_id,
                                                outcome.run().id(),
                                            )?;
                                            self.update_provider_run_projection(outcome.into_run());
                                        }
                                        Err(DaemonError::ProviderRunNotFound { .. }) => {
                                            self.sessions
                                                .set_active_provider_run(session_id, None)?;
                                        }
                                        Err(error) => return Err(error),
                                    }
                                }
                            }
                            Err(DaemonError::ProviderRunNotFound { .. }) => {
                                self.sessions.set_active_provider_run(session_id, None)?;
                            }
                            Err(error) => return Err(error),
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
                self.sessions.set_active_provider_run(session_id, None)?;
            }
            return Ok(());
        }
        if self.prompt_state_owner.has_any_active_prompt(&session) {
            return Ok(());
        }

        let focused_agent_id = session.focused_agent_id().map(str::to_string);
        if let Some(focused_agent_id) = focused_agent_id {
            self.sync_active_provider_run_for_agent(session_id, &focused_agent_id)?;
        } else {
            self.sessions.set_active_provider_run(session_id, None)?;
        }

        Ok(())
    }

    pub(crate) fn provider_run_has_active_prompt(
        &self,
        session_id: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<bool, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(false);
        };
        let session = self.sessions.get_session(session_id)?;
        let Some(prompt) = self
            .prompt_state_owner
            .active_prompt_for_agent_snapshot(&session, agent_id)
        else {
            return Ok(false);
        };
        Ok(prompt.durable_delivery_provider_run_id().map_or_else(
            || session.active_provider_run_id() == Some(provider_run.id()),
            |delivery_run_id| delivery_run_id == provider_run.id(),
        ))
    }
}
