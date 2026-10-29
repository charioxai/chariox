//! Move uses the ordinary worker capability checks before it admits execution.
use super::RemoteLeaseRuntime;
use crate::agent_move::AgentMoveRequirements;
use crate::error::DaemonError;

impl RemoteLeaseRuntime<'_> {
    pub(crate) fn prepare_agent_move_capabilities(
        &mut self,
        identity: &crate::agent_move::AgentMoveIdentity,
        leased_agent_id: &str,
        requirements: &AgentMoveRequirements,
    ) -> Result<(), DaemonError> {
        let leased = self
            .app
            .leased_agents
            .get(leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.into(),
            })?;
        let target = self
            .app
            .agent_moves
            .target(&identity.operation_id)
            .ok_or_else(|| crate::agent_move::error("Move target is missing"))?;
        let lease = self
            .app
            .execution_leases
            .get(&leased.lease_id)
            .ok_or_else(|| crate::agent_move::error("Move backing lease is missing"))?;
        if target.active
            || target.retired
            || target.capture.identity != *identity
            || target.execution_lease_id.as_deref() != Some(leased.lease_id.as_str())
            || lease.home_kernel_id != identity.home_kernel_id
            || lease.home_session_id != identity.home_session_id
            || lease.owner_user_id != identity.owner_user_id
            || leased.home_agent_id != identity.home_agent_id
        {
            return Err(crate::agent_move::error(
                "Move capabilities require this generation's exact backing lease",
            ));
        }
        let home_path = std::path::Path::new(&identity.home_kernel_id);
        if !matches!(
            home_path.components().collect::<Vec<_>>().as_slice(),
            [std::path::Component::Normal(_)]
        ) {
            return Err(crate::agent_move::error(
                "Move home kernel identity is not a valid package location",
            ));
        }
        let session = self.app.sessions.get_session(&leased.backing_session_id)?;
        let staged =
            crate::skill::remote_skill_materialization_base(session.worktree_id()).join(home_path);
        let registry = crate::skill::CharioxSkillRegistry::new(
            crate::skill::CharioxSkillRegistry::user_root()
                .into_iter()
                .collect(),
        );
        for required in &requirements.skills {
            if required.version_hash.len() != 64
                || !required
                    .version_hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(crate::agent_move::error(
                    "Move skill requirement has an invalid package digest",
                ));
            }
            if registry.package(&required.name)?.is_some() {
                continue;
            }
            let staged_registry =
                crate::skill::CharioxSkillRegistry::new(vec![staged.join(&required.name)]);
            let package = staged_registry.package(&required.version_hash)?.ok_or_else(|| crate::agent_move::error(format!("Move skill `{}` has not been staged; install the selected package and retry", required.name)))?;
            if package.metadata.name != required.name
                || package.version_hash != required.version_hash
            {
                return Err(crate::agent_move::error(format!(
                    "Move skill `{}` staged package failed hash verification; restage it and retry",
                    required.name
                )));
            }
            registry
                .install_from_path(&staged.join(&required.name).join(&required.version_hash))?;
        }
        self.apply_required_remote_skills(&leased, &requirements.skills)?;
        self.ensure_home_proxy_manifest_has_no_worker_collisions(
            &leased,
            &[],
            &requirements.extensions,
            "prepare agent Move",
        )
    }
}
