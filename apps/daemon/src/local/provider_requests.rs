use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{
    ensure_codex_catalog_endpoint, opencode_catalog_endpoint, CodexClient, LaunchProviderRequest,
    OpenCodeClient, OpenCodeProviderCatalog,
};

use super::api::{
    GetProviderAuthStatusRequest, GetProviderRunRequest, LaunchProviderRunRequest,
    LocalDaemonResponse, StartProviderLoginRequest,
};

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
        let mut catalogs = Vec::new();

        if let Ok(endpoint) = opencode_catalog_endpoint() {
            if let Ok(client) = OpenCodeClient::new("catalog", endpoint) {
                if let Ok(catalog) = client.provider_catalog() {
                    catalogs.push(catalog);
                }
            }
        }
        if let Ok(endpoint) = ensure_codex_catalog_endpoint() {
            if let Ok(client) = CodexClient::new("catalog", endpoint) {
                if let Ok(catalog) = client.provider_catalog() {
                    catalogs.push(catalog);
                }
            }
        }

        let catalog = merge_provider_catalogs(catalogs).ok_or_else(|| DaemonError::LocalTransport {
            operation: "get_provider_catalog",
            message: "no provider catalog sources were reachable".to_string(),
        })?;
        crate::logging::info_with_fields(
            "daemon.local",
            "Retrieved merged provider catalog",
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

    pub(super) fn handle_get_provider_auth_status_request(
        &mut self,
        request: GetProviderAuthStatusRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request.provider.as_str() {
            "codex" => {
                let endpoint = ensure_codex_catalog_endpoint()?;
                let client = CodexClient::new("provider-auth", endpoint)?;
                Ok(LocalDaemonResponse::ProviderAuthStatus {
                    status: client.auth_status()?,
                })
            }
            provider => Err(DaemonError::LocalTransport {
                operation: "get_provider_auth_status",
                message: format!("provider `{provider}` does not expose an auth status API"),
            }),
        }
    }

    pub(super) fn handle_start_provider_login_request(
        &mut self,
        request: StartProviderLoginRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request.provider.as_str() {
            "codex" => {
                let endpoint = ensure_codex_catalog_endpoint()?;
                let client = CodexClient::new("provider-login", endpoint)?;
                Ok(LocalDaemonResponse::ProviderLoginStarted {
                    login: client.start_login()?,
                })
            }
            provider => Err(DaemonError::LocalTransport {
                operation: "start_provider_login",
                message: format!("provider `{provider}` does not expose a login API"),
            }),
        }
    }
}

fn merge_provider_catalogs(catalogs: Vec<OpenCodeProviderCatalog>) -> Option<OpenCodeProviderCatalog> {
    let mut iter = catalogs.into_iter();
    let mut merged = iter.next()?;
    for catalog in iter {
        merged.connected.extend(catalog.connected);
        merged.connected.sort();
        merged.connected.dedup();
        for (provider_id, model_id) in catalog.default {
            merged.default.insert(provider_id, model_id);
        }
        for provider in catalog.all {
            if let Some(existing) = merged.all.iter_mut().find(|item| item.id == provider.id) {
                for (model_id, model) in provider.models {
                    existing.models.insert(model_id, model);
                }
            } else {
                merged.all.push(provider);
            }
        }
    }
    merged
        .all
        .sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    Some(merged)
}
