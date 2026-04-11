use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{
    default_provider_command_catalogs, ensure_codex_catalog_endpoint,
    ensure_opencode_catalog_endpoint, logout_codex, CodexClient, LaunchProviderRequest,
    OpenCodeClient, OpenCodeProviderCatalog,
};
use arroba_relay::protocol::RelayMachinePresence;
use tokio::runtime::Handle;
use tokio::runtime::Runtime;

use super::api::{
    ConfigureRelayRequest, GetProviderAuthStatusRequest, GetProviderRunRequest,
    LaunchProviderRunRequest, ListRemoteMachineKernelsRequest, LocalDaemonResponse,
    LogoutProviderRequest, RelayStatus, StartProviderLoginRequest,
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

    pub(crate) fn handle_get_provider_catalog_request(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut catalogs = Vec::new();

        if let Ok(endpoint) = ensure_opencode_catalog_endpoint() {
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

        let mut catalog =
            merge_provider_catalogs(catalogs).ok_or_else(|| DaemonError::LocalTransport {
                operation: "get_provider_catalog",
                message: "no provider catalog sources were reachable".to_string(),
            })?;
        let remote_machines =
            if self.config().relay_url.is_some() && self.config().relay_token.is_some() {
                block_on_relay_query(crate::transport::relay_discovery::list_live_machines(
                    self.config(),
                ))
                .unwrap_or_default()
            } else {
                Vec::new()
            };
        annotate_remote_machine_providers(
            &mut catalog,
            &remote_machines,
            &self.config().host_machine_id,
        );
        crate::logging::info_with_fields(
            "daemon.local",
            "Retrieved merged provider catalog",
            serde_json::json!({
                "provider_count": catalog.all.len(),
                "providers": catalog.all.iter().map(|p| serde_json::json!({
                    "id": &p.id,
                    "name": &p.name,
                    "remote_machine_aliases": &p.remote_machine_aliases,
                    "model_count": p.models.len(),
                    "models": p.models.keys().collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "connected": &catalog.connected,
            }),
        );
        Ok(LocalDaemonResponse::ProviderCatalog { catalog })
    }

    pub(super) fn handle_get_provider_command_catalogs_request(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::ProviderCommandCatalogs {
            catalogs: default_provider_command_catalogs(),
        })
    }

    pub(super) fn handle_relay_status_request(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::RelayStatus {
            status: self.relay_status_snapshot()?,
        })
    }

    pub(super) fn handle_configure_relay_request(
        &mut self,
        request: ConfigureRelayRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.configure_relay(request.relay_url, request.relay_token)?;
        Ok(LocalDaemonResponse::RelayConfigured {
            status: self.relay_status_snapshot()?,
        })
    }

    fn relay_status_snapshot(&self) -> Result<RelayStatus, DaemonError> {
        let relay_state = self.relay_client_state();
        let connected = block_on_relay_query(async move {
            Ok::<bool, DaemonError>(relay_state.read().await.connected())
        })?;
        Ok(RelayStatus {
            configured: self.config().relay_url.is_some() && self.config().relay_token.is_some(),
            connected,
            relay_url: self.config().relay_url.clone(),
            relay_token_configured: self.config().relay_token.is_some(),
            daemon_id: self.config().daemon_id.clone(),
            machine_id: self.config().host_machine_id.clone(),
            machine_alias: self.config().host_machine_alias.clone(),
        })
    }

    pub(super) fn handle_list_remote_machines_request(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config().clone();
        let machines = block_on_relay_query(
            crate::transport::relay_discovery::list_live_machines(&config),
        )?;
        Ok(LocalDaemonResponse::RemoteMachinesListed { machines })
    }

    pub(super) fn handle_list_remote_machine_kernels_request(
        &mut self,
        request: ListRemoteMachineKernelsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config().clone();
        let machine_ref = request.machine_ref;
        let kernels = block_on_relay_query(
            crate::transport::relay_discovery::list_live_kernels_for_machine(&config, &machine_ref),
        )?;
        Ok(LocalDaemonResponse::RemoteMachineKernelsListed {
            machine_ref,
            kernels,
        })
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

    pub(super) fn handle_logout_provider_request(
        &mut self,
        request: LogoutProviderRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request.provider.as_str() {
            "codex" => {
                logout_codex()?;
                Ok(LocalDaemonResponse::ProviderLoggedOut {
                    provider: "codex".to_string(),
                })
            }
            provider => Err(DaemonError::LocalTransport {
                operation: "logout_provider",
                message: format!("provider `{provider}` does not expose a logout API"),
            }),
        }
    }
}

fn block_on_relay_query<F, T>(future: F) -> Result<T, DaemonError>
where
    F: std::future::Future<Output = Result<T, DaemonError>>,
{
    if let Ok(handle) = Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(future))
    } else {
        Runtime::new()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "create relay discovery runtime",
                message: error.to_string(),
            })?
            .block_on(future)
    }
}

fn merge_provider_catalogs(
    catalogs: Vec<OpenCodeProviderCatalog>,
) -> Option<OpenCodeProviderCatalog> {
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

fn annotate_remote_machine_providers(
    catalog: &mut OpenCodeProviderCatalog,
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) {
    for provider in &mut catalog.all {
        provider.remote_machine_aliases =
            remote_machine_aliases_for_provider(&provider.id, live_machines, local_machine_id);
    }
}

fn remote_machine_aliases_for_provider(
    provider_id: &str,
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) -> Vec<String> {
    let mut aliases = live_machines
        .iter()
        .filter(|machine| machine.machine_id != local_machine_id)
        .filter(|machine| {
            machine
                .available_providers
                .iter()
                .any(|provider| provider == provider_id)
        })
        .map(|machine| {
            machine
                .machine_alias
                .as_deref()
                .map(str::trim)
                .filter(|alias| !alias.is_empty())
                .unwrap_or(machine.machine_id.as_str())
                .to_string()
        })
        .collect::<Vec<_>>();
    aliases.sort();
    aliases.dedup();
    aliases
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::OpenCodeProviderInfo;

    #[test]
    fn annotates_remote_machine_provider_aliases_without_including_local_machine() {
        let mut catalog = OpenCodeProviderCatalog {
            all: vec![
                OpenCodeProviderInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    remote_machine_aliases: Vec::new(),
                    models: Default::default(),
                },
                OpenCodeProviderInfo {
                    id: "opencode".to_string(),
                    name: "OpenCode".to_string(),
                    remote_machine_aliases: Vec::new(),
                    models: Default::default(),
                },
            ],
            default: Default::default(),
            connected: vec!["codex".to_string(), "opencode".to_string()],
        };
        let live_machines = vec![
            RelayMachinePresence {
                machine_id: "machine-local".to_string(),
                machine_alias: Some("home".to_string()),
                kernel_count: 1,
                available_providers: vec!["codex".to_string()],
            },
            RelayMachinePresence {
                machine_id: "machine-remote-a".to_string(),
                machine_alias: Some("builder-west".to_string()),
                kernel_count: 1,
                available_providers: vec!["codex".to_string(), "opencode".to_string()],
            },
            RelayMachinePresence {
                machine_id: "machine-remote-b".to_string(),
                machine_alias: None,
                kernel_count: 1,
                available_providers: vec!["codex".to_string()],
            },
        ];

        annotate_remote_machine_providers(&mut catalog, &live_machines, "machine-local");

        let codex = catalog
            .all
            .iter()
            .find(|provider| provider.id == "codex")
            .unwrap();
        let opencode = catalog
            .all
            .iter()
            .find(|provider| provider.id == "opencode")
            .unwrap();

        assert_eq!(
            codex.remote_machine_aliases,
            vec!["builder-west".to_string(), "machine-remote-b".to_string()]
        );
        assert_eq!(
            opencode.remote_machine_aliases,
            vec!["builder-west".to_string()]
        );
    }
}
