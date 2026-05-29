use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn spawn_agent(
        &self,
        request: crate::agent::CreateAgentRequest,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let mut sessions = self.session_store.write();
        self.agent_store.create_agent(request, &mut sessions)
    }

    pub(super) fn ensure_agent_owner(
        &self,
        agent_id: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.owner_user_id() == user_id {
            Ok(agent)
        } else {
            Err(DaemonError::OwnershipAccessDenied {
                user_id: user_id.to_string(),
                owner_user_id: agent.owner_user_id().to_string(),
                resource: format!("agent `{agent_id}`"),
                operation,
            })
        }
    }

    pub(super) fn ensure_agent_prompt_access(
        &self,
        agent_id: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        let session = self.session_store.get_session(agent.session_id())?;
        if session.can_prompt_agent_owned_by(user_id, agent.owner_user_id()) {
            Ok(agent)
        } else {
            Err(DaemonError::OwnershipAccessDenied {
                user_id: user_id.to_string(),
                owner_user_id: agent.owner_user_id().to_string(),
                resource: format!("agent `{agent_id}`"),
                operation,
            })
        }
    }

    pub(super) fn ensure_agent_ref_owner(
        &self,
        agent_ref: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.agent_store.get_agent_by_ref(agent_ref))?;
        if agent.owner_user_id() == user_id {
            Ok(agent)
        } else {
            Err(DaemonError::OwnershipAccessDenied {
                user_id: user_id.to_string(),
                owner_user_id: agent.owner_user_id().to_string(),
                resource: format!("agent `{agent_ref}`"),
                operation,
            })
        }
    }

    pub(super) fn ensure_agent_extension_authority(
        &self,
        agent_ref: &str,
        user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .agent_store
            .get_agent(agent_ref)
            .or_else(|_| self.agent_store.get_agent_by_ref(agent_ref))?;
        if agent.remote_execution().is_some() {
            let session = self.session_store.get_session(agent.session_id())?;
            if session.owner_user_id() == user_id {
                return Ok(agent);
            }
            return Err(DaemonError::OwnershipAccessDenied {
                user_id: user_id.to_string(),
                owner_user_id: session.owner_user_id().to_string(),
                resource: format!("home extensions for remote-backed agent `{agent_ref}`"),
                operation,
            });
        }
        if agent.owner_user_id() == user_id {
            Ok(agent)
        } else {
            Err(DaemonError::OwnershipAccessDenied {
                user_id: user_id.to_string(),
                owner_user_id: agent.owner_user_id().to_string(),
                resource: format!("agent `{agent_ref}`"),
                operation,
            })
        }
    }

    pub(super) fn destroy_agent(
        &self,
        agent_id: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.ensure_agent_owner(agent_id, caller_user_id, "destroy agent")?;
        let session_id = agent.session_id().to_string();
        let provider_run_ids = self
            .provider_store
            .list_runs()
            .into_iter()
            .filter(|run| {
                run.session_id() == session_id
                    && run.agent_instance_id() == Some(agent_id)
                    && run.state() != crate::provider::ProviderRunState::Ended
            })
            .map(|run| run.id().to_string())
            .collect::<Vec<_>>();
        for provider_run_id in provider_run_ids {
            let ended = self
                .provider_store
                .terminate_run_provider_only(&session_id, &provider_run_id)?
                .into_run();
            if self
                .session_store
                .get_session(&session_id)?
                .active_provider_run_id()
                == Some(ended.id())
            {
                self.session_store
                    .set_active_provider_run(&session_id, None)?;
            }
            self.provider_run_projection.update(ended.clone());
            self.clear_prompt_activity(ended.id());
            self.remove_provider_process_tracking_for_run(ended.id(), None);
        }
        self.prompt_state_owner.remove_agent(&session_id, agent_id);
        self.session_store.mirror_agent_prompt_state(
            &session_id,
            agent_id,
            None,
            std::collections::VecDeque::new(),
        )?;
        let mut sessions = self.session_store.write();
        self.agent_store.destroy_agent(agent_id, &mut sessions)
    }

    pub(super) fn focus_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.ensure_agent_owner(agent_id, caller_user_id, "focus agent")?;
        let mut sessions = self.session_store.write();
        let agent = self
            .agent_store
            .focus_agent(session_id, agent_id, &mut sessions)?;
        drop(sessions);
        if !self.should_defer_provider_run_sync_for_focus_change(session_id, agent_id)? {
            self.sync_active_provider_run_for_agent(session_id, agent_id)?;
        }
        Ok(agent)
    }

    pub(super) fn cycle_agent_focus(
        &self,
        session_id: &str,
        caller_user_id: &str,
    ) -> Result<Option<crate::agent::AgentInstance>, DaemonError> {
        let own_agents = self
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .filter(|agent| agent.owner_user_id() == caller_user_id)
            .collect::<Vec<_>>();
        if own_agents.is_empty() {
            return Ok(None);
        }
        let current_focused = self
            .agent_store
            .get_focused_agent(session_id)
            .filter(|agent| agent.owner_user_id() == caller_user_id)
            .map(|agent| agent.id().to_string());
        let next_agent_id = if let Some(current_id) = current_focused {
            let current_index = own_agents
                .iter()
                .position(|agent| agent.id() == current_id)
                .unwrap_or(0);
            own_agents[(current_index + 1) % own_agents.len()]
                .id()
                .to_string()
        } else {
            own_agents[0].id().to_string()
        };
        let mut sessions = self.session_store.write();
        let agent = self
            .agent_store
            .focus_agent(session_id, &next_agent_id, &mut sessions)
            .map(Some)?;
        drop(sessions);
        if let Some(focused) = agent.as_ref() {
            if !self.should_defer_provider_run_sync_for_focus_change(session_id, focused.id())? {
                self.sync_active_provider_run_for_agent(session_id, focused.id())?;
            }
        }
        Ok(agent)
    }
}
