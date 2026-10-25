//! Attach a verified publication, then probe the required native session.
use super::*;
use crate::agent_move::{error as move_error, AgentMoveIdentity};
use sha2::Digest;
impl KernelRuntimeState {
    pub(super) async fn attach_agent_move_target(
        &self,
        identity: &AgentMoveIdentity,
        session_id: &str,
        agent_id: &str,
        leased: bool,
    ) -> Result<(), DaemonError> {
        let mut target = self.ensure_agent_move_target_identity(identity)?;
        let receipt = target
            .receipt
            .clone()
            .ok_or_else(|| move_error("Publish the move Project before attaching an agent"))?;
        if target.ready
            && target.backing_session_id.as_deref() == Some(session_id)
            && target.backing_agent_id.as_deref() == Some(agent_id)
        {
            return Ok(());
        }
        if target.active {
            return Err(move_error("This move already committed; recover its current execution binding instead of reattaching its old checkpoint"));
        }
        let primary = receipt.primary_path()?.display().to_string();
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(move_error(
                "Move target agent does not belong to its execution Room",
            ));
        }
        if leased {
            let session = self.owned.session_store.get_session(session_id)?;
            if agent.worktree_id().unwrap_or(session.worktree_id()) != primary {
                return Err(move_error(
                    "Target agent was not created from the materialization's primary workspace",
                ));
            }
            let workspaces = receipt
                .repositories
                .iter()
                .map(|repository| repository.destination_path.display().to_string())
                .collect();
            let project = self.owned.session_store.read().prepare_leased_project(
                session_id,
                &receipt.target_project_id,
                workspaces,
            )?;
            let bound = self
                .owned
                .session_store
                .write()
                .bind_leased_project(session_id, project.clone())?;
            self.owned.durable_state_store.append_event(
                "project.updated",
                Some(project.id().to_string()),
                serde_json::json!({"project":project}),
            )?;
            self.owned.session_store.write().restore_ephemeral_session(
                bound.for_execution_project(&receipt.target_project_id, &primary, &primary),
            )?;
        }
        if !leased {
            let mut project = crate::session::RuntimeProject::new(
                &receipt.target_project_id,
                agent.owner_user_id(),
                &primary,
                self.owned
                    .session_store
                    .get_project(
                        self.owned
                            .session_store
                            .get_session(session_id)?
                            .project_id(),
                    )?
                    .name(),
                crate::session::RuntimeProjectKind::Named,
            );
            project.replace_workspace_ids(
                receipt
                    .repositories
                    .iter()
                    .map(|repository| repository.destination_path.display().to_string())
                    .collect(),
            );
            self.owned
                .session_store
                .restore_projects(vec![project.clone()]);
            self.owned.durable_state_store.append_event(
                "project.updated",
                Some(project.id().to_string()),
                serde_json::json!({"project":project}),
            )?;
        }
        let workspace = primary.clone();
        let mut agent = self.owned.agent_store.update_agent_config(
            agent_id,
            None,
            None,
            Some(Some(workspace)),
            Some(Some(primary)),
        )?;
        if let Some(checkpoint) = self.owned.agent_moves.native_checkpoint(&target.capture)? {
            if !leased {
                let account_owner =
                    self.provider_account_authority_owner_user_id(agent.owner_user_id());
                let profile = format!(
                    "move-{:x}",
                    sha2::Sha256::digest(identity.operation_id.as_bytes())
                );
                let profiles = &self.owned.provider_account_profiles;
                if profiles.find(&account_owner, "codex", &profile)?.is_none() {
                    let mut materialization = profiles.export_materialization(
                        &account_owner,
                        "codex",
                        agent.provider_account_profile(),
                    )?;
                    materialization.profile.profile_id = profile.clone();
                    materialization.profile.is_default = false;
                    profiles.materialize_replica(&account_owner, &materialization)?;
                }
                target.target_account_profile = Some(profile.clone());
                self.owned.agent_moves.save_target(target.clone())?;
                agent.set_account_profile(Some(profile));
            }
            let resume = crate::provider::ProviderResumeState::from_codex_thread_id(
                checkpoint.provider_session_id(),
            )
            .requiring_native_session(
                "codex",
                checkpoint.provider_session_id(),
                Some(checkpoint.binding().clone()),
            );
            agent.set_provider_resume_state(resume);
        }
        self.owned.agent_store.restore_agent(agent.clone());
        if !leased {
            self.owned.durable_state_store.append_event(
                "agent.updated",
                Some(agent.id().into()),
                serde_json::json!({"agent":agent}),
            )?;
        }
        target.project = Some(
            self.owned
                .session_store
                .get_project(&receipt.target_project_id)?,
        );
        target.backing_session_id = Some(session_id.into());
        target.backing_agent_id = Some(agent_id.into());
        self.owned.agent_moves.save_target(target.clone())?;
        self.probe_agent_move_native_target(&target, &agent).await?;
        target.ready = true;
        self.owned.agent_moves.save_target(target)?;
        Ok(())
    }

    async fn probe_agent_move_native_target(
        &self,
        target: &crate::agent_move::MoveTarget,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        let Some(checkpoint) = self.owned.agent_moves.native_checkpoint(&target.capture)? else {
            return Ok(());
        };
        let mut request = crate::provider::LaunchProviderRequest::new(
            agent.session_id(),
            "codex",
            agent.provider(),
            agent.provider_account_profile(),
            agent.model().unwrap_or("default"),
        )
        .with_agent_id(agent.id())
        .with_owner_user_id(agent.owner_user_id())
        .with_variant(agent.effort().map(str::to_string))
        .with_working_directory(
            target
                .receipt
                .as_ref()
                .ok_or_else(|| move_error("Move receipt is missing"))?
                .primary_path()?
                .to_path_buf(),
        )
        .with_resume_state(agent.provider_resume_state().clone());
        request.agent_move_preparation = Some(target.capture.identity.operation_id.clone());
        let request = self
            .prepare_provider_launch_request_with_vault(request, "prepare moved native session")
            .await?;
        tokio::task::spawn_blocking(move || checkpoint.import_for_move(&request))
            .await
            .map_err(|_| move_error("Native target probe stopped; retry the durable move"))?
            .map(|_| ())
            .map_err(crate::provider::NativeCheckpointFailure::into_daemon_error)
    }
}
