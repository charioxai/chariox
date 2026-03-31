use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{opencode_catalog_endpoint, LaunchProviderRequest, OpenCodeClient};

use super::api::{GetProviderRunRequest, LaunchProviderRunRequest, LocalDaemonResponse};

impl DaemonApp {
    pub(super) fn handle_launch_provider_run_request(
        &mut self,
        request: LaunchProviderRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut launch_request = LaunchProviderRequest::new(
            request.session_id.clone(),
            request.adapter_key,
            request.provider,
            request.account_profile,
            request.model,
        )
        .with_variant(request.variant);
        if let Some(agent_id) = request.agent_id.clone().or_else(|| {
            self.sessions()
                .get_session(&request.session_id)
                .ok()
                .and_then(|session| session.focused_agent_id().map(str::to_string))
        }) {
            launch_request = launch_request.with_agent_id(agent_id);
        }
        let provider_run = self.launch_provider(launch_request)?;
        crate::logging::debug_with_fields(
            "daemon.local_api",
            "returning launched provider run to client",
            serde_json::json!({
                "provider_run_id": provider_run.id(),
                "session_id": provider_run.session_id(),
                "provider": provider_run.provider(),
                "model": provider_run.model(),
                "variant": provider_run.variant(),
                "state": provider_run.state().to_string(),
            }),
        );
        Ok(LocalDaemonResponse::ProviderRunLaunched { provider_run })
    }

    pub(super) fn handle_get_provider_run_request(
        &mut self,
        request: GetProviderRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.providers_mut()
            .sync_run_selection(&request.provider_run_id)?;
        let provider_run = self.providers().get_run(&request.provider_run_id)?;
        Ok(LocalDaemonResponse::ProviderRun { provider_run })
    }

    pub(super) fn handle_get_provider_catalog_request(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let endpoint = opencode_catalog_endpoint()?;
        let client = OpenCodeClient::new("catalog", endpoint)?;
        let catalog = client.provider_catalog()?;
        crate::logging::info_with_fields(
            "daemon.local",
            "Retrieved provider catalog from OpenCode",
            serde_json::json!({
                "provider_count": catalog.all.len(),
                "providers": catalog.all.iter().map(|p| serde_json::json!({
                    "id": &p.id,
                    "name": &p.name,
                    "model_count": p.models.len(),
                    "models": p.models.keys().collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "connected": &catalog.connected,
            }),
        );
        Ok(LocalDaemonResponse::ProviderCatalog { catalog })
    }
}
