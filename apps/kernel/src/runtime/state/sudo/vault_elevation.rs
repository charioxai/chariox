//! MP-08 / MP-09 / MP-10 / MP-11: one elevation seam for agent factor capture/use.
use super::KernelRuntimeState;
use crate::config::{UserCredentialConfig, UserCredentialInjectionConfig};
use crate::error::DaemonError;

impl KernelRuntimeState {
    /// S1 replaces this predicate with project-mandate-or-sudo authorization.
    /// Keep the exact current task check: an open window for another agent or
    /// an earlier task is not permission to capture or use this factor.
    pub(crate) fn vault_credential_elevation_allows(
        &self,
        session_id: &str,
        agent_id: &str,
        credential: &UserCredentialConfig,
    ) -> Result<(), DaemonError> {
        if !matches!(
            credential.injection,
            UserCredentialInjectionConfig::Totp(_) | UserCredentialInjectionConfig::BackupCodes
        ) {
            return Ok(());
        }
        let run = self
            .owned
            .agent_store
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| {
                if agent.session_id() != session_id {
                    return None;
                }
                if let Some(remote) = agent.remote_execution() {
                    remote.active_worker_provider_run_id.as_deref().map(|run| {
                        crate::provider::projected_leased_provider_run_id(
                            &remote.leased_agent_id,
                            run,
                        )
                    })
                } else {
                    self.owned
                        .provider_store
                        .get_run_for_agent(session_id, agent_id)
                        .map(|run| run.id().to_string())
                }
            });
        if run.is_some_and(|run| self.sudo_for_run(&run).is_ok()) {
            return Ok(());
        }
        Err(DaemonError::LocalTransport {
            operation: "vault_elevation",
            message: "TOTP and backup-code capture/use requires a live sudo window for this task"
                .into(),
        })
    }
}
