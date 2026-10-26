//! Probe source eligibility and native compatibility before either admission fence closes.
use super::*;
use crate::agent_move::error as move_error;
use crate::provider::{NativeCheckpointFailure, NativeMoveCompatibility};

impl KernelRuntimeState {
    pub(super) async fn probe_agent_move_source(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<NativeMoveCompatibility>, DaemonError> {
        let session = self
            .owned
            .agent_moves
            .execution_session(&self.owned.session_store.get_session(session_id)?, agent_id)?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            return Err(move_error(
                "Finish the active source turn before moving this agent",
            ));
        }
        if let Some(process) = self
            .owned
            .durable_state_store
            .leased_process_receipts()?
            .into_iter()
            .find(|receipt| {
                receipt.backing_room == session_id
                    && receipt.backing_agent == agent_id
                    && !matches!(
                        receipt.wake.state.as_str(),
                        "exited" | "lost" | "cancelled" | "fired"
                    )
            })
        {
            return Err(move_error(format!("Move cannot retire live process wake `{}`. Wait for it to exit or cancel it, then retry", process.wake.id)));
        }
        let latest = self
            .owned
            .provider_store
            .get_latest_run_for_agent(session_id, agent_id);
        let project = self.owned.session_store.get_project(session.project_id())?;
        let requested = agent.workspace_id().unwrap_or(session.workspace_id());
        let primary = latest
            .as_ref()
            .and_then(|run| run.working_directory())
            .and_then(|path| path.to_str())
            .or(agent.worktree_id())
            .unwrap_or(session.worktree_id());
        if !project
            .workspace_ids()
            .iter()
            .any(|workspace| workspace == requested || workspace == primary)
        {
            return Err(move_error("The agent's execution Workspace is outside its selected Project. Add that Workspace to the Project or select a worktree in this Project, then retry"));
        }
        if let Some(run) = latest
            .filter(|run| run.provider_session_id().is_some() || !run.resume_state().is_empty())
        {
            return tokio::task::spawn_blocking(move || NativeMoveCompatibility::from_run(&run))
                .await
                .map_err(|_| move_error("Native source preflight stopped; retry before moving"))?
                .map(Some)
                .map_err(NativeCheckpointFailure::into_daemon_error);
        }
        if !agent.provider_resume_state().is_empty() {
            if agent
                .provider_resume_state()
                .required_native_session()
                .is_some()
            {
                if let Some(target) = self
                    .owned
                    .agent_moves
                    .target_for_agent(session_id, agent_id)
                {
                    if let Some(binding) = target.capture.checkpoint_binding {
                        return Ok(Some(binding.move_compatibility()));
                    }
                }
            }
            return Err(move_error("The source provider run is unavailable after restart. Resume it on the source and finish one turn before moving"));
        }
        Ok(None)
    }

    pub(super) async fn probe_agent_move_target(
        &self,
        native: Option<NativeMoveCompatibility>,
        home_agent: Option<&crate::agent::AgentInstance>,
    ) -> Result<(), DaemonError> {
        let Some(native) = native else {
            return Ok(());
        };
        let account = home_agent
            .map(|agent| {
                let owner = crate::account_profile::provider_account_authority_owner_user_id(
                    &self.owned.config_projection.snapshot(),
                    agent.owner_user_id(),
                );
                let profile = agent.provider_account_profile().to_string();
                let environment = self
                    .owned
                    .provider_account_profiles
                    .resolve_environment(&owner, "codex", &profile)?;
                Ok::<_, DaemonError>((owner, profile, environment))
            })
            .transpose()?;
        tokio::task::spawn_blocking(move || {
            native.validate_target()?;
            if let Some((owner, profile, environment)) = account {
                native.verify_account(&owner, &profile, environment)?;
            }
            Ok::<_, NativeCheckpointFailure>(())
        })
        .await
        .map_err(|_| move_error("Native target preflight stopped; retry before moving"))?
        .map_err(NativeCheckpointFailure::into_daemon_error)
    }
}
