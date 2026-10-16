//! MP-08 / MP-10 / MP-11: authenticated immutable packages for every worker placement.
use super::RemoteLeaseRuntime;
use crate::{
    error::DaemonError,
    skill::v2::SkillTransferManifest,
    transport::relay_peer::{RemoteSkillMaterialization, RemoteSkillSyncContext},
};
impl<'a> RemoteLeaseRuntime<'a> {
    pub(crate) fn ensure_remote_skill_packages(
        &mut self,
        context: RemoteSkillSyncContext,
        execution_lease_id: String,
        manifest: SkillTransferManifest,
        packages: Vec<crate::skill::CharioxSkillPackage>,
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
        let config_path = crate::config::DaemonConfig::default_user_config_path();
        let home = config_path
            .parent()
            .ok_or_else(|| DaemonError::InvalidConfig {
                field: "CHARIOX_HOME",
                message: "Skills require a resolved Chariox home",
            })?;
        crate::skill::transfer::materialize_received_packages(
            home,
            &context,
            &execution_lease_id,
            &manifest,
            &packages,
        )
    }
}
