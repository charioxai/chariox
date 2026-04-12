use crate::app::DaemonApp;
use crate::config::{DaemonConfig, PersistedMachineRegistration};
use crate::error::DaemonError;
use crate::provider::{
    default_provider_command_catalogs, ensure_codex_catalog_endpoint,
    ensure_opencode_catalog_endpoint, logout_codex, CodexClient, LaunchProviderRequest,
    OpenCodeClient, OpenCodeProviderCatalog, OpenCodeProviderInfo,
};
use arroba_relay::protocol::RelayMachinePresence;
use std::thread;
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::runtime::Runtime;

use super::api::{
    ApproveRemoteMachineRequest, ConfigureRelayRequest, ForgetRemoteMachineRequest,
    GetProviderAuthStatusRequest, GetProviderRunRequest, LaunchProviderRunRequest,
    ListRemoteMachineKernelsRequest, LocalDaemonResponse, LogoutProviderRequest, RelayStatus,
    RemoteMachineRecord, RemoteMachineTrustStatus, RenameRemoteMachineRequest,
    StartProviderLoginRequest,
};

const PROVIDER_CATALOG_CACHE_TTL: Duration = Duration::from_secs(5);

impl DaemonApp {
    pub(super) fn handle_launch_provider_run_request(
        &mut self,
        request: LaunchProviderRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let launch_request = launch_provider_request_from_local(self, request);
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
            .apply_finished_provider_run_selection_sync_jobs();
        self.providers_mut()
            .enqueue_run_selection_sync(&request.provider_run_id)?;
        let provider_run = self.providers().get_run(&request.provider_run_id)?;
        Ok(LocalDaemonResponse::ProviderRun { provider_run })
    }

    pub(crate) fn handle_get_provider_catalog_request(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if let Some(catalog) = self.cached_provider_catalog() {
            return Ok(LocalDaemonResponse::ProviderCatalog { catalog });
        }

        let catalog = load_provider_catalog(self.config().clone())?;
        self.cache_provider_catalog(catalog.clone());
        Ok(LocalDaemonResponse::ProviderCatalog { catalog })
    }

    pub(crate) fn cached_provider_catalog(&self) -> Option<OpenCodeProviderCatalog> {
        let Some((cached_at, catalog)) = &self.provider_catalog_cache else {
            return None;
        };
        if cached_at.elapsed() < PROVIDER_CATALOG_CACHE_TTL {
            Some(catalog.clone())
        } else {
            None
        }
    }

    pub(crate) fn cache_provider_catalog(&mut self, catalog: OpenCodeProviderCatalog) {
        self.provider_catalog_cache = Some((Instant::now(), catalog.clone()));
    }

    pub(crate) fn invalidate_provider_catalog_cache(&mut self) {
        self.provider_catalog_cache = None;
    }

    pub(super) fn handle_get_provider_command_catalogs_request(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        provider_command_catalogs_response()
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
        self.provider_catalog_cache = None;
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
        let machines = remote_machine_records(machines, &config.host_machine_id);
        Ok(LocalDaemonResponse::RemoteMachinesListed { machines })
    }

    pub(super) fn handle_list_remote_machine_kernels_request(
        &mut self,
        request: ListRemoteMachineKernelsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config().clone();
        let machine_ref = resolve_registered_or_raw_machine_ref(&request.machine_ref);
        let kernels = block_on_relay_query(
            crate::transport::relay_discovery::list_live_kernels_for_machine(&config, &machine_ref),
        )?;
        Ok(LocalDaemonResponse::RemoteMachineKernelsListed {
            machine_ref,
            kernels,
        })
    }

    pub(super) fn handle_approve_remote_machine_request(
        &mut self,
        request: ApproveRemoteMachineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config().clone();
        let live = block_on_relay_query(crate::transport::relay_discovery::list_live_machines(
            &config,
        ))
        .unwrap_or_default();
        let machine = resolve_machine_for_registry(&request.machine_ref, &live)?;
        DaemonConfig::approve_remote_machine(
            machine.machine_id.clone(),
            machine.machine_alias.clone(),
        )?;
        self.provider_catalog_cache = None;
        let machine = record_for_machine_id(machine.machine_id, live, &config.host_machine_id)?;
        Ok(LocalDaemonResponse::RemoteMachineApproved { machine })
    }

    pub(super) fn handle_forget_remote_machine_request(
        &mut self,
        request: ForgetRemoteMachineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config().clone();
        let live = block_on_relay_query(crate::transport::relay_discovery::list_live_machines(
            &config,
        ))
        .unwrap_or_default();
        let machine = resolve_machine_id_for_registry(&request.machine_ref, &live)?;
        let saved = DaemonConfig::forget_remote_machine(machine.clone())?;
        self.provider_catalog_cache = None;
        let machine = forgotten_machine_record(machine, saved.alias, live, &config.host_machine_id);
        Ok(LocalDaemonResponse::RemoteMachineForgotten { machine })
    }

    pub(super) fn handle_rename_remote_machine_request(
        &mut self,
        request: RenameRemoteMachineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config().clone();
        let live = block_on_relay_query(crate::transport::relay_discovery::list_live_machines(
            &config,
        ))
        .unwrap_or_default();
        let machine = resolve_machine_id_for_registry(&request.machine_ref, &live)?;
        DaemonConfig::rename_remote_machine(machine.clone(), request.alias)?;
        self.provider_catalog_cache = None;
        let machine = record_for_machine_id(machine, live, &config.host_machine_id)?;
        Ok(LocalDaemonResponse::RemoteMachineRenamed { machine })
    }

    pub(super) fn handle_get_provider_auth_status_request(
        &mut self,
        request: GetProviderAuthStatusRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request.provider.as_str() {
            "codex" => provider_auth_status_response(request),
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
            "codex" => start_provider_login_response(request),
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
                let response = logout_provider_response(request)?;
                self.provider_catalog_cache = None;
                Ok(response)
            }
            provider => Err(DaemonError::LocalTransport {
                operation: "logout_provider",
                message: format!("provider `{provider}` does not expose a logout API"),
            }),
        }
    }
}

pub(crate) fn launch_provider_request_from_local(
    app: &DaemonApp,
    request: LaunchProviderRunRequest,
) -> LaunchProviderRequest {
    let mut launch_request = LaunchProviderRequest::new(
        request.session_id.clone(),
        request.adapter_key,
        request.provider,
        request.account_profile,
        request.model,
    )
    .with_variant(request.variant);
    if let Some(agent_id) = request.agent_id.clone().or_else(|| {
        app.sessions()
            .get_session(&request.session_id)
            .ok()
            .and_then(|session| session.focused_agent_id().map(str::to_string))
    }) {
        launch_request = launch_request.with_agent_id(agent_id);
    }
    launch_request
}

pub(crate) fn provider_command_catalogs_response() -> Result<LocalDaemonResponse, DaemonError> {
    Ok(LocalDaemonResponse::ProviderCommandCatalogs {
        catalogs: default_provider_command_catalogs(),
    })
}

pub(crate) fn load_provider_catalog(
    config: DaemonConfig,
) -> Result<OpenCodeProviderCatalog, DaemonError> {
    if config.provider_catalog_read_delay_ms > 0 {
        thread::sleep(Duration::from_millis(config.provider_catalog_read_delay_ms));
    }

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

    let remote_machines = if config.relay_url.is_some() && config.relay_token.is_some() {
        block_on_relay_query(crate::transport::relay_discovery::list_live_machines(
            &config,
        ))
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let approved_remote_machines =
        approved_live_remote_machines(&remote_machines, &config.host_machine_id);

    let mut catalog = merge_provider_catalogs(catalogs)
        .or_else(|| {
            remote_only_provider_catalog(&approved_remote_machines, &config.host_machine_id)
        })
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "get_provider_catalog",
            message: "no provider catalog sources were reachable".to_string(),
        })?;
    annotate_remote_machine_providers(
        &mut catalog,
        &approved_remote_machines,
        &config.host_machine_id,
    );
    crate::logging::info_with_fields(
        "daemon.local",
        "Retrieved merged provider catalog",
        serde_json::json!({
            "provider_count": catalog.all.len(),
            "model_count": catalog.all.iter().map(|provider| provider.models.len()).sum::<usize>(),
            "remote_provider_count": catalog.all.iter().filter(|provider| !provider.remote_machine_aliases.is_empty()).count(),
            "connected": &catalog.connected,
        }),
    );
    Ok(catalog)
}

pub(crate) fn provider_auth_status_response(
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

pub(crate) fn start_provider_login_response(
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

pub(crate) fn logout_provider_response(
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

fn remote_only_provider_catalog(
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) -> Option<OpenCodeProviderCatalog> {
    let mut provider_ids = live_machines
        .iter()
        .filter(|machine| machine.machine_id != local_machine_id)
        .flat_map(|machine| machine.available_providers.iter().cloned())
        .collect::<Vec<_>>();
    provider_ids.sort();
    provider_ids.dedup();
    if provider_ids.is_empty() {
        return None;
    }

    let all = provider_ids
        .into_iter()
        .map(|provider_id| OpenCodeProviderInfo {
            name: display_name_for_provider(&provider_id),
            id: provider_id,
            remote_machine_aliases: Vec::new(),
            models: Default::default(),
        })
        .collect::<Vec<_>>();

    Some(OpenCodeProviderCatalog {
        connected: all.iter().map(|provider| provider.id.clone()).collect(),
        all,
        default: Default::default(),
    })
}

fn display_name_for_provider(provider_id: &str) -> String {
    match provider_id {
        "codex" => "Codex".to_string(),
        "opencode" => "OpenCode".to_string(),
        "dev-stub" => "Dev Stub".to_string(),
        other => other.to_string(),
    }
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

fn approved_live_remote_machines(
    live_machines: &[RelayMachinePresence],
    local_machine_id: &str,
) -> Vec<RelayMachinePresence> {
    let registry = DaemonConfig::machine_registry_entries();
    live_machines
        .iter()
        .filter(|machine| machine.machine_id != local_machine_id)
        .filter_map(|machine| {
            registry
                .iter()
                .find(|entry| {
                    entry.machine_id == machine.machine_id && entry.approved && !entry.forgotten
                })
                .map(|entry| {
                    let mut machine = machine.clone();
                    if entry.alias.is_some() {
                        machine.machine_alias = entry.alias.clone();
                    }
                    machine
                })
        })
        .collect()
}

pub(crate) fn remote_machine_records(
    live_machines: Vec<RelayMachinePresence>,
    local_machine_id: &str,
) -> Vec<RemoteMachineRecord> {
    let registry = DaemonConfig::machine_registry_entries();
    let mut records: Vec<RemoteMachineRecord> = live_machines
        .into_iter()
        .filter(|machine| machine.machine_id != local_machine_id)
        .filter_map(|machine| {
            let entry = registry
                .iter()
                .find(|entry| entry.machine_id == machine.machine_id);
            if entry.map(|entry| entry.forgotten).unwrap_or(false) {
                return None;
            }
            Some(remote_machine_record(machine, entry, true))
        })
        .collect();

    let offline_entries = registry
        .iter()
        .filter(|entry| entry.approved && !entry.forgotten)
        .filter(|entry| {
            !records
                .iter()
                .any(|record| record.machine_id == entry.machine_id)
        })
        .cloned()
        .collect::<Vec<_>>();

    for entry in offline_entries {
        records.push(RemoteMachineRecord {
            machine_id: entry.machine_id.clone(),
            machine_alias: None,
            registry_alias: entry.alias.clone(),
            display_name: entry
                .alias
                .clone()
                .unwrap_or_else(|| entry.machine_id.clone()),
            trust_status: RemoteMachineTrustStatus::Approved,
            online: false,
            pending: false,
            kernel_count: 0,
            available_providers: Vec::new(),
        });
    }

    records.sort_by_key(|record| {
        (
            !record.online,
            record.pending,
            record.display_name.to_ascii_lowercase(),
            record.machine_id.clone(),
        )
    });
    records
}

fn remote_machine_record(
    machine: RelayMachinePresence,
    entry: Option<&PersistedMachineRegistration>,
    online: bool,
) -> RemoteMachineRecord {
    let approved = entry
        .map(|entry| entry.approved && !entry.forgotten)
        .unwrap_or(false);
    let registry_alias = entry.and_then(|entry| entry.alias.clone());
    let display_name = registry_alias
        .clone()
        .or_else(|| machine.machine_alias.clone())
        .unwrap_or_else(|| machine.machine_id.clone());
    RemoteMachineRecord {
        machine_id: machine.machine_id,
        machine_alias: machine.machine_alias,
        registry_alias,
        display_name,
        trust_status: if approved {
            RemoteMachineTrustStatus::Approved
        } else {
            RemoteMachineTrustStatus::Pending
        },
        online,
        pending: !approved,
        kernel_count: machine.kernel_count,
        available_providers: machine.available_providers,
    }
}

pub(crate) fn resolve_registered_or_raw_machine_ref(machine_ref: &str) -> String {
    DaemonConfig::resolve_registered_machine_ref(machine_ref)
        .unwrap_or_else(|| machine_ref.trim().to_string())
}

fn resolve_machine_for_registry(
    machine_ref: &str,
    live_machines: &[RelayMachinePresence],
) -> Result<RelayMachinePresence, DaemonError> {
    let machine_ref = machine_ref.trim();
    live_machines
        .iter()
        .find(|machine| {
            machine.machine_id == machine_ref
                || machine.machine_alias.as_deref() == Some(machine_ref)
        })
        .cloned()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "resolve remote machine",
            message: format!("no live remote machine found for `{machine_ref}`"),
        })
}

fn resolve_machine_id_for_registry(
    machine_ref: &str,
    live_machines: &[RelayMachinePresence],
) -> Result<String, DaemonError> {
    if let Some(machine_id) = DaemonConfig::resolve_registered_machine_ref(machine_ref) {
        return Ok(machine_id);
    }
    resolve_machine_for_registry(machine_ref, live_machines).map(|machine| machine.machine_id)
}

fn record_for_machine_id(
    machine_id: String,
    live_machines: Vec<RelayMachinePresence>,
    local_machine_id: &str,
) -> Result<RemoteMachineRecord, DaemonError> {
    remote_machine_records(live_machines, local_machine_id)
        .into_iter()
        .find(|machine| machine.machine_id == machine_id)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "load remote machine record",
            message: format!("remote machine `{machine_id}` is not visible"),
        })
}

fn forgotten_machine_record(
    machine_id: String,
    registry_alias: Option<String>,
    live_machines: Vec<RelayMachinePresence>,
    local_machine_id: &str,
) -> RemoteMachineRecord {
    let live = live_machines
        .into_iter()
        .find(|machine| machine.machine_id == machine_id && machine.machine_id != local_machine_id);
    let display_name = registry_alias
        .clone()
        .or_else(|| {
            live.as_ref()
                .and_then(|machine| machine.machine_alias.clone())
        })
        .unwrap_or_else(|| machine_id.clone());
    RemoteMachineRecord {
        machine_id,
        machine_alias: live
            .as_ref()
            .and_then(|machine| machine.machine_alias.clone()),
        registry_alias,
        display_name,
        trust_status: RemoteMachineTrustStatus::Forgotten,
        online: live.is_some(),
        pending: false,
        kernel_count: live
            .as_ref()
            .map(|machine| machine.kernel_count)
            .unwrap_or(0),
        available_providers: live
            .map(|machine| machine.available_providers)
            .unwrap_or_default(),
    }
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

    #[test]
    fn builds_remote_only_catalog_when_local_provider_sources_are_unavailable() {
        let live_machines = vec![
            RelayMachinePresence {
                machine_id: "machine-local".to_string(),
                machine_alias: Some("home".to_string()),
                kernel_count: 1,
                available_providers: vec!["codex".to_string()],
            },
            RelayMachinePresence {
                machine_id: "machine-remote".to_string(),
                machine_alias: Some("builder".to_string()),
                kernel_count: 1,
                available_providers: vec!["codex".to_string(), "opencode".to_string()],
            },
        ];

        let mut catalog = remote_only_provider_catalog(&live_machines, "machine-local")
            .expect("remote providers should create a catalog");
        annotate_remote_machine_providers(&mut catalog, &live_machines, "machine-local");

        assert_eq!(
            catalog.connected,
            vec!["codex".to_string(), "opencode".to_string()]
        );
        assert_eq!(catalog.default.len(), 0);
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
        assert_eq!(codex.name, "Codex");
        assert_eq!(codex.remote_machine_aliases, vec!["builder".to_string()]);
        assert_eq!(opencode.name, "OpenCode");
        assert_eq!(opencode.remote_machine_aliases, vec!["builder".to_string()]);
    }
}
