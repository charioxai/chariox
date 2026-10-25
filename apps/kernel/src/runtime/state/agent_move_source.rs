//! Quiesce the source before capturing its native session and current Project.
use super::*;
use crate::agent_move::{
    error as move_error, AgentMoveCapture, AgentMoveIdentity, HomeMovePhase, MoveSource,
};
use crate::managed_context::development::{
    DevelopmentRepositoryRole, DevelopmentRepositorySelection,
};
use sha2::Digest;
impl KernelRuntimeState {
    pub(super) async fn prepare_agent_move_source(
        &self,
        identity: AgentMoveIdentity,
        session_id: &str,
        agent_id: &str,
    ) -> Result<AgentMoveCapture, DaemonError> {
        let _source_operation = self
            .owned
            .agent_moves
            .operation(format!("source:{}", identity.operation_id))
            .await;

        if let Some(source) = self.owned.agent_moves.source(&identity.operation_id) {
            if source.identity != identity
                || source.session_id != session_id
                || source.agent_id != agent_id
                || source.released
            {
                return Err(move_error(
                    "Move source no longer belongs to this operation",
                ));
            }
            if let Some(capture) = source.capture {
                return Ok(capture);
            }
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
        let session = self
            .owned
            .agent_moves
            .execution_session(&self.owned.session_store.get_session(session_id)?, agent_id);
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let _claim = self
            .owned
            .prompt_state_owner
            .claim_idle_agent_move_transition(&session, agent_id)?;
        let latest = self
            .owned
            .provider_store
            .get_latest_run_for_agent(session_id, agent_id);
        let pending_checkpoint = if latest.is_none()
            && agent
                .provider_resume_state()
                .required_native_session()
                .is_some()
        {
            self.owned
                .agent_moves
                .target_for_agent(session_id, agent_id)
                .map(|target| self.owned.agent_moves.native_checkpoint(&target.capture))
                .transpose()?
                .flatten()
        } else {
            None
        };
        if latest.is_none()
            && pending_checkpoint.is_none()
            && !agent.provider_resume_state().is_empty()
        {
            return Err(move_error("The source provider run is unavailable after restart. Resume it on the source and finish one turn before moving"));
        }
        let mut source = self
            .owned
            .agent_moves
            .source(&identity.operation_id)
            .unwrap_or(MoveSource {
                identity,
                session_id: session_id.into(),
                agent_id: agent_id.into(),
                provider_run_id: latest.as_ref().map(|run| run.id().to_string()),
                stopped: false,
                released: false,
                capture: None,
                retired: false,
            });
        self.owned.agent_moves.save_source(source.clone())?;
        let mut terminated = self
            .owned
            .terminate_idle_provider_runs_for_agent_before_remote_move(session_id, &agent)?;
        if let Some(run) = &latest {
            if !terminated.iter().any(|id| id == run.id()) {
                terminated.push(run.id().to_string());
            }
        }
        for run_id in terminated {
            let (_, key) = self
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app).remove_run(&run_id)
                })
                .await?;
            self.owned
                .remove_provider_process_tracking_for_run(&run_id, key);
            self.owned.provider_store.clear_runtime(&run_id);
            self.owned
                .connector_adapter_processes
                .shutdown_run(&run_id)
                .await;
        }
        if let Some(run) = &latest {
            self.owned.provider_store.clear_runtime(run.id());
        }
        source.stopped = true;
        self.owned.agent_moves.save_source(source.clone())?;
        if let Some(mut home) = self.owned.agent_moves.home(&source.identity.operation_id) {
            if matches!(home.phase, HomeMovePhase::Fenced) {
                home.phase = HomeMovePhase::SourceStopped;
                self.owned.agent_moves.save_home(home)?;
            }
        }
        let provider_worktree = latest
            .as_ref()
            .and_then(|run| run.working_directory())
            .map(|path| path.display().to_string());
        let checkpoint = if let Some(run) = latest
            .filter(|run| run.provider_session_id().is_some() || !run.resume_state().is_empty())
        {
            let providers = self.owned.provider_store.clone();
            let run_id = run.id().to_string();
            Some(
                tokio::task::spawn_blocking(move || providers.export_native_checkpoint(&run_id))
                    .await
                    .map_err(|_| {
                        move_error("Native checkpoint task stopped; retry the durable operation")
                    })??,
            )
        } else {
            pending_checkpoint
        };
        let requested_workspace = agent.workspace_id().unwrap_or(session.workspace_id());
        let primary_path = provider_worktree
            .as_deref()
            .or(agent.worktree_id())
            .unwrap_or(session.worktree_id());
        let project = self.owned.session_store.get_project(session.project_id())?;
        let primary_workspace = project.workspace_ids().iter()
            .find(|workspace| workspace.as_str() == requested_workspace)
            .or_else(|| project.workspace_ids().iter().find(|workspace| workspace.as_str() == primary_path))
            .ok_or_else(|| move_error("The agent's execution Workspace is outside its selected Project. Add that Workspace to the Project or select a worktree in this Project, then retry"))?;
        let project_id = project.id().to_string();
        let repositories: Vec<DevelopmentRepositorySelection> = project
            .workspace_ids()
            .iter()
            .map(|workspace| DevelopmentRepositorySelection {
                workspace_id: workspace.clone(),
                worktree_id: (workspace == primary_workspace).then(|| primary_path.to_string()),
                worktree_path: PathBuf::from(if workspace == primary_workspace {
                    primary_path
                } else {
                    workspace
                }),
                role: if workspace == primary_workspace {
                    DevelopmentRepositoryRole::Primary
                } else {
                    DevelopmentRepositoryRole::Supporting
                },
            })
            .collect();
        let copied_project_id = format!(
            "move-{:x}",
            sha2::Sha256::digest(source.identity.operation_id.as_bytes())
        );
        let environment = self
            .prepare_agent_move_environment_layer(
                &project_id,
                &repositories,
                &source.identity.operation_id,
                &source.identity.target_kernel_id,
                &source.identity.target_public_key,
                &copied_project_id,
            )
            .await?;
        let store = self.owned.agent_moves.clone();
        let config = self.owned.config_projection.snapshot();
        tokio::task::spawn_blocking(move || {
            store.capture_source(
                &source,
                project_id,
                repositories,
                checkpoint.as_ref(),
                &config,
                environment,
            )
        })
        .await
        .map_err(|_| move_error("Project snapshot task stopped; retry the durable operation"))?
    }
}
