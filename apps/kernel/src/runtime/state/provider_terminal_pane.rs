//! An agent pane a provider's own terminal runs (docs/PROTOCOL.md section 3.3.2.1): the
//! settings that terminal changed, and the person sitting at it.
use super::*;

impl KernelRuntimeState {
    /// Whether a provider's own terminal runs this agent now. A card shown in its pane then
    /// waits for the person without a deadline.
    pub(crate) fn provider_terminal_runs_agent(&self, session_id: &str, agent_id: &str) -> bool {
        self.owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
            .is_some_and(|run| !run.client_interface().is_chariox())
    }

    /// Whether the registered card waits for the person at a provider terminal: the pane it is
    /// shown in now (a sub-agent's card shows in its top parent's) is run by one.
    pub(crate) fn provider_terminal_holds_card(
        &self,
        session_id: &str,
        interaction_id: &str,
    ) -> bool {
        let Ok(session) = self.owned.session_store.get_session(session_id) else {
            return false;
        };
        session
            .active_interactions()
            .iter()
            .find(|interaction| interaction.id() == interaction_id)
            .and_then(|interaction| {
                interaction
                    .shown_in_agent_id()
                    .or_else(|| interaction.agent_id())
            })
            .is_some_and(|pane| self.provider_terminal_runs_agent(session_id, pane))
    }

    /// Records settings the provider already runs with; nothing is applied to the run.
    pub(crate) async fn report_pane_settings(
        &self,
        request: crate::local::ReportPaneSettingsRequest,
        caller_user_id: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let crate::local::ReportPaneSettingsRequest {
            session_id,
            agent_id,
            model,
            effort,
            execution_mode,
            permission_level,
        } = request;
        let agent = owned.agent_store.get_agent(&agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id,
                agent_id,
            });
        }
        owned.ensure_agent_owner(&agent_id, caller_user_id, "report pane settings")?;
        let run = owned
            .provider_store
            .get_run_for_agent(&session_id, &agent_id)
            .filter(|run| !run.client_interface().is_chariox())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "report pane settings",
                message: format!(
                    "agent `{agent_id}` runs in no provider terminal; change its settings with UpdateAgentConfig"
                ),
            })?;
        if model.is_some() || effort.is_some() {
            owned.agent_store.set_agent_runtime_profile_durably(
                &owned.durable_state_store,
                &agent_id,
                agent.provider(),
                model.or_else(|| agent.model().map(str::to_owned)),
                effort.unwrap_or_else(|| agent.effort().map(str::to_owned)),
                agent.account_profile().map(str::to_owned),
                agent.provider_resume_state().clone(),
                Some(run.id()),
                Some("changed in the provider's terminal"),
            )?;
        }
        let scope_changed = execution_mode.is_some() || permission_level.is_some();
        if scope_changed {
            owned.agent_store.update_agent_config(
                &agent_id,
                execution_mode.map(Some),
                permission_level.map(Some),
                None,
                None,
            )?;
        }
        let _ = owned.session_snapshot(&session_id)?;
        if scope_changed {
            // A lowered agent and its subtree may no longer hold the decisions they held.
            let (moved, announce) = owned.recheck_subtree_decisions(&session_id, &agent_id)?;
            self.finish_moved_decisions(&session_id, moved, announce)
                .await?;
        }
        // The agent as it now is, every reported field applied, survives a restart.
        let agent = owned.agent_store.get_agent(&agent_id)?;
        self.append_agent_durable_event("agent.updated", &agent, None)
            .await?;
        Ok(())
    }
}
