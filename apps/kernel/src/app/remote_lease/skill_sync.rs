//! MP-08 / MP-10 / MP-11: authenticated immutable packages for every worker placement.
use super::RemoteLeaseRuntime;
use crate::{
    error::DaemonError,
    skill::{leased_names::LeasedSkillNames, v2::SkillTransferManifest},
    transport::relay_peer::{RemoteSkillMaterialization, RemoteSkillSyncContext},
};
impl<'a> RemoteLeaseRuntime<'a> {
    pub(crate) fn ensure_remote_skill_packages(
        &mut self,
        context: RemoteSkillSyncContext,
        execution_lease_id: String,
        manifest: SkillTransferManifest,
        packages: Vec<crate::skill::CharioxSkillPackage>,
        skill_names: Option<LeasedSkillNames>,
    ) -> Result<Vec<RemoteSkillMaterialization>, DaemonError> {
        let agent = self
            .app
            .leased_agents
            .get(&context.leased_agent_id)
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: context.leased_agent_id.clone(),
            })?;
        let lease = self
            .app
            .execution_leases
            .get(&agent.lease_id)
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: agent.lease_id.clone(),
            })?;
        let expired = self.app.config.kernel_runtime_role
            == crate::config::KernelRuntimeRole::RemoteLeaseWorker
            && crate::session::unix_epoch_ms().saturating_sub(lease.created_at_ms)
                >= super::REMOTE_EXECUTION_LEASE_MAX_LIFETIME_MS;
        if agent.lease_id != execution_lease_id
            || lease.worker_kernel_id != self.app.config.daemon_id
            || lease.home_kernel_id != context.home_kernel_id
            || lease.home_session_id != context.home_session_id
            || lease.home_agent_id != context.home_agent_id
            || agent.home_agent_id != context.home_agent_id
            || expired
        {
            return Err(DaemonError::LocalTransport {
                operation: "ensure remote skill packages",
                message:
                    "skill transfer is bound to an expired, replaced or foreign execution lease"
                        .into(),
            });
        }
        if let Some(names) = &skill_names {
            names.check_transferred(&manifest)?;
        }
        let home = skill_home()?;
        let materialized = crate::skill::transfer::materialize_received_packages(
            &home,
            &context,
            &execution_lease_id,
            &manifest,
            &packages,
        )?;
        // The newest catalog's transfer is the home's current view, including "nothing
        // typable". Refreshes run concurrently, so an older one can arrive last: its
        // packages are cached but it never replaces a newer view.
        if let Some(agent) = self.app.leased_agents.get_mut(&context.leased_agent_id) {
            if manifest.catalog_revision >= agent.skill_view_revision {
                agent.skill_view_revision = manifest.catalog_revision;
                agent.skill_names = skill_names;
            }
        }
        Ok(materialized)
    }
}

fn skill_home() -> Result<std::path::PathBuf, DaemonError> {
    crate::config::DaemonConfig::default_user_config_path()
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or(DaemonError::InvalidConfig {
            field: "CHARIOX_HOME",
            message: "Skills require a resolved Chariox home",
        })
}

impl<'a> RemoteLeaseRuntime<'a> {
    /// The only skill cache a managed provider of this leased agent may read.
    pub(crate) fn leased_skill_cache_root(
        &self,
        leased_agent_id: &str,
    ) -> Result<std::path::PathBuf, DaemonError> {
        let agent = self.app.leased_agents.get(leased_agent_id).ok_or_else(|| {
            DaemonError::LeasedAgentNotFound {
                leased_agent_id: leased_agent_id.to_string(),
            }
        })?;
        let lease = self
            .app
            .execution_leases
            .get(&agent.lease_id)
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: agent.lease_id.clone(),
            })?;
        let context = RemoteSkillSyncContext {
            home_kernel_id: lease.home_kernel_id.clone(),
            home_session_id: lease.home_session_id.clone(),
            home_agent_id: lease.home_agent_id.clone(),
            leased_agent_id: leased_agent_id.to_string(),
        };
        crate::skill::transfer::execution_cache_root(&skill_home()?, &context, &agent.lease_id)
    }
}

impl<'a> RemoteLeaseRuntime<'a> {
    /// The leased agent whose provider runs in this backing session, if any.
    pub(crate) fn leased_agent_id_for_backing(
        &self,
        backing_session_id: &str,
        backing_agent_id: &str,
    ) -> Option<String> {
        self.app
            .leased_agents
            .values()
            .find(|agent| {
                agent.backing_session_id == backing_session_id
                    && agent.backing_agent_id == backing_agent_id
            })
            .map(|agent| agent.id.clone())
    }
}
