use crate::error::DaemonError;
use crate::transport::relay_peer::{RemoteSkillMaterialization, RemoteSkillSyncContext};

use super::RemoteLeaseRuntime;

impl<'a> RemoteLeaseRuntime<'a> {
    pub(crate) fn ensure_remote_skill_packages(
        &mut self,
        context: RemoteSkillSyncContext,
        packages: Vec<crate::skill::ArrobaSkillPackage>,
    ) -> Result<Vec<RemoteSkillMaterialization>, DaemonError> {
        let leased_agent = self
            .app
            .leased_agents
            .get(&context.leased_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LeasedAgentNotFound {
                leased_agent_id: context.leased_agent_id.clone(),
            })?;
        let lease = self
            .app
            .execution_leases
            .get(&leased_agent.lease_id)
            .cloned()
            .ok_or_else(|| DaemonError::ExecutionLeaseNotFound {
                lease_id: leased_agent.lease_id.clone(),
            })?;
        if lease.home_kernel_id != context.home_kernel_id
            || lease.home_session_id != context.home_session_id
            || lease.home_agent_id != context.home_agent_id
        {
            return Err(DaemonError::LocalTransport {
                operation: "ensure remote skill packages",
                message: "remote skill sync context does not match leased agent".to_string(),
            });
        }
        let session = self
            .app
            .sessions
            .get_session(&leased_agent.backing_session_id)?;
        let base_dir = crate::skill::remote_skill_materialization_base(session.worktree_id())
            .join(&context.home_kernel_id);
        packages
            .iter()
            .map(|package| {
                let materialized_root =
                    crate::skill::materialize_skill_package(&base_dir, package)?;
                Ok(RemoteSkillMaterialization {
                    name: package.metadata.name.clone(),
                    version_hash: package.version_hash.clone(),
                    materialized_root: materialized_root.to_string_lossy().to_string(),
                })
            })
            .collect()
    }
}
