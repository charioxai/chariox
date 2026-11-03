//! Compose ordinary account, extension and Project setup admission with Move.
use super::*;
use crate::agent_move::{
    agent_configuration_sha256, error as move_error, AgentMoveCommitReceipt, AgentMoveIdentity,
    AgentMoveReadiness, AgentMoveRequirements, HomeMoveOperation,
};

impl KernelRuntimeState {
    pub(super) async fn agent_move_requirements(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<AgentMoveRequirements, DaemonError> {
        self.with_app_side_effect(|app| {
            let extensions = app.extension_manifest_for_agent_move(agent)?;
            extensions.validate_unique_tool_names("prepare agent Move")?;
            Ok(AgentMoveRequirements {
                agent_configuration_sha256: agent_configuration_sha256(agent)?,
                extensions,
                skills: app.required_remote_skills_for_native_provider_launch(agent)?,
            })
        })
        .await
    }

    pub(super) async fn verify_agent_move_requirements(
        &self,
        operation: &HomeMoveOperation,
    ) -> Result<(), DaemonError> {
        let expected = operation.composition.requirements.as_ref().ok_or_else(|| move_error(
            "This pending Move predates resource readiness receipts. Cancel preparation and start a new Move after upgrading both kernels",
        ))?;
        let agent = self
            .owned
            .agent_store
            .get_agent(&operation.identity.home_agent_id)?;
        if agent_configuration_sha256(&agent)? != expected.agent_configuration_sha256
            || &self.agent_move_requirements(&agent).await? != expected
        {
            return Err(move_error("The selected account, grants, packages or Project setup changed during Move. Restore the selected resources or cancel preparation and retry"));
        }
        Ok(())
    }

    pub(super) async fn prepare_agent_move_capabilities(
        &self,
        identity: &AgentMoveIdentity,
        leased_agent_id: &str,
        requirements: &AgentMoveRequirements,
    ) -> Result<(), DaemonError> {
        let mut target = self.ensure_agent_move_target_identity(identity)?;
        if target
            .composition
            .requirements
            .as_ref()
            .is_some_and(|old| old != requirements)
        {
            return Err(move_error(
                "Move resource requirements changed; cancel preparation and retry",
            ));
        }
        self.with_app_side_effect(|app| {
            crate::app::RemoteLeaseRuntime::new(app).prepare_agent_move_capabilities(
                identity,
                leased_agent_id,
                requirements,
            )
        })
        .await?;
        target.composition.requirements = Some(requirements.clone());
        self.owned.agent_moves.save_target(target)
    }

    pub(super) async fn finish_agent_move_readiness(
        &self,
        identity: &AgentMoveIdentity,
        requirements: &AgentMoveRequirements,
    ) -> Result<AgentMoveReadiness, DaemonError> {
        let mut target = self.ensure_agent_move_target_identity(identity)?;
        if target
            .composition
            .requirements
            .as_ref()
            .is_some_and(|old| old != requirements)
        {
            return Err(move_error(
                "Move resource requirements changed; cancel preparation and retry",
            ));
        }
        let session_id = target
            .backing_session_id
            .clone()
            .ok_or_else(|| move_error("Move target Room is missing"))?;
        let agent_id = target
            .backing_agent_id
            .clone()
            .ok_or_else(|| move_error("Move target agent is missing"))?;
        let agent = self.owned.agent_store.get_agent(&agent_id)?;
        if let Some(binding) = &target.capture.checkpoint_binding {
            self.probe_agent_move_target(Some(binding.move_compatibility()), Some(&agent))
                .await?;
        }
        let receipt = target
            .receipt
            .as_ref()
            .ok_or_else(|| move_error("Move Project receipt is missing"))?;
        let readiness = AgentMoveReadiness {
            identity: identity.clone(),
            requirements_sha256: requirements.sha256()?,
            workspace_sha256: target.capture.workspace.sha256.clone(),
            native_sha256: target
                .capture
                .native
                .as_ref()
                .map(|artifact| artifact.sha256.clone()),
            target_project_id: receipt.target_project_id.clone(),
            target_account_profile: agent.provider_account_profile().to_string(),
            backing_session_id: session_id,
            backing_agent_id: agent_id,
        };
        if target
            .composition
            .readiness
            .as_ref()
            .is_some_and(|old| old != &readiness)
        {
            return Err(move_error(
                "Move target readiness changed; reconcile this generation before retrying",
            ));
        }
        target = self.ensure_agent_move_target_identity(identity)?;
        if target
            .composition
            .readiness
            .as_ref()
            .is_some_and(|old| old != &readiness)
        {
            return Err(move_error(
                "Move target readiness changed while setup was running",
            ));
        }
        target.composition.requirements = Some(requirements.clone());
        target.composition.readiness = Some(readiness.clone());
        self.owned.agent_moves.save_target(target)?;
        Ok(readiness)
    }

    pub(super) async fn activate_agent_move_target(
        &self,
        identity: &AgentMoveIdentity,
        readiness: &AgentMoveReadiness,
    ) -> Result<AgentMoveCommitReceipt, DaemonError> {
        let target = self.ensure_agent_move_target_identity(identity)?;
        if target.composition.readiness.as_ref() != Some(readiness)
            || &readiness.identity != identity
        {
            return Err(move_error(
                "Move activation requires its exact verified readiness receipt",
            ));
        }
        if let Some(receipt) = target.composition.commit {
            return Ok(receipt);
        }
        if !target.ready
            || target.retired
            || (identity.home_kernel_id != identity.target_kernel_id && target.backing.is_none())
        {
            return Err(move_error(
                "Move target is not attached; finish resource preparation before committing",
            ));
        }
        let requirements = target
            .composition
            .requirements
            .as_ref()
            .ok_or_else(|| move_error("Move resource requirements are missing"))?
            .clone();
        if let Some(backing) = &target.backing {
            self.prepare_agent_move_capabilities(identity, &backing.leased_agent.id, &requirements)
                .await?;
        }
        let checked = self
            .finish_agent_move_readiness(identity, &requirements)
            .await?;
        if &checked != readiness {
            return Err(move_error(
                "Move resources changed before activation; prepare again",
            ));
        }
        let mut target = self.ensure_agent_move_target_identity(identity)?;
        let commit = AgentMoveCommitReceipt {
            readiness: readiness.clone(),
            committed_at_ms: crate::session::unix_epoch_ms(),
        };
        target.active = true;
        target.composition.commit = Some(commit.clone());
        self.owned.agent_moves.save_target(target)?;
        Ok(commit)
    }
}

impl KernelRuntimeState {}
