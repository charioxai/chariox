use super::RemoteLeaseRuntime;
use crate::error::DaemonError;

impl RemoteLeaseRuntime<'_> {
    pub(super) fn ensure_leased_provider_capacity(
        &self,
        provider: &str,
        replacing: Option<&str>,
    ) -> Result<(), DaemonError> {
        if crate::provider::adapter_key_for_provider(provider) != "codex" {
            return Ok(());
        }
        let Some(capacity) = crate::provider::configured_codex_session_capacity()? else {
            return Ok(());
        };
        let allocated = self
            .app
            .leased_agents
            .values()
            .filter(|agent| {
                Some(agent.id.as_str()) != replacing
                    && crate::provider::adapter_key_for_provider(&agent.provider) == "codex"
            })
            .count();
        if allocated >= capacity {
            return Err(DaemonError::MachineRequest {
                code: "provider_capacity_reached".into(),
                message: format!(
                    "Worker Codex endpoint capacity reached ({allocated}/{capacity} agents). Delete an unused Codex agent or choose another worker. Container memory and process limits may support fewer running providers."
                ),
            });
        }
        Ok(())
    }
}
