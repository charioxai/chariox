use crate::app::DaemonApp;
use crate::config::{DaemonConfig, PersistedMachineRegistration};
use crate::error::DaemonError;
use crate::provider::{
    default_provider_command_catalogs, ensure_codex_catalog_endpoint,
    ensure_opencode_catalog_endpoint, logout_codex, CodexClient, LaunchProviderRequest,
    OpenCodeClient, OpenCodeProviderCatalog, OpenCodeProviderInfo, ProviderClientInterface,
};
use arroba_relay::protocol::RelayMachinePresence;
use std::thread;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::runtime::Runtime;

use super::api::{
    ApproveRemoteMachineRequest, ConfigureRelayRequest, ForgetRemoteMachineRequest,
    GetProviderAuthStatusRequest, GetProviderRunRequest, LaunchProviderRunRequest,
    ListRemoteMachineKernelsRequest, LocalDaemonResponse, LogoutProviderRequest, RelayStatus,
    RemoteMachineRecord, RemoteMachineTrustStatus, RenameRemoteMachineRequest,
    StartProviderLoginRequest, UpdateProviderRunSelectionRequest,
};

pub(crate) const PROVIDER_CATALOG_CACHE_TTL: Duration = Duration::from_secs(5);

pub(crate) fn get_provider_run_response(
    app: &mut DaemonApp,
    request: GetProviderRunRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    app.providers_mut()
        .apply_finished_provider_run_selection_sync_jobs();
    app.providers_mut()
        .enqueue_run_selection_sync(&request.provider_run_id)?;
    let provider_run = app.providers().get_run(&request.provider_run_id)?;
    app.update_provider_run_projection(provider_run.clone());
    Ok(LocalDaemonResponse::ProviderRun { provider_run })
}

pub(crate) fn update_provider_run_selection_response(
    app: &mut DaemonApp,
    request: UpdateProviderRunSelectionRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let run = app.providers().get_run(&request.provider_run_id)?;
    if run.session_id() != request.session_id {
        return Err(DaemonError::ProviderRunNotInSession {
            session_id: request.session_id,
            provider_run_id: request.provider_run_id,
        });
    }
    let provider_run = app.providers_mut().update_run_selection(
        &request.provider_run_id,
        request.model,
        request.variant,
        request.clear_variant,
    )?;
    app.update_provider_run_projection(provider_run.clone());
    Ok(LocalDaemonResponse::ProviderRunSelectionUpdated { provider_run })
}

#[allow(dead_code)]
impl DaemonApp {
    pub(super) fn launch_provider_run_response(
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
        self.update_provider_run_projection(provider_run.clone());
        Ok(LocalDaemonResponse::ProviderRunLaunched { provider_run })
    }

    pub(crate) fn provider_catalog_response(&mut self) -> Result<LocalDaemonResponse, DaemonError> {
        if let Some(catalog) = self.cached_provider_catalog() {
            return Ok(LocalDaemonResponse::ProviderCatalog { catalog });
        }

        let catalog = load_provider_catalog(self.config().clone())?;
        self.cache_provider_catalog(catalog.clone());
        Ok(LocalDaemonResponse::ProviderCatalog { catalog })
    }

    pub(crate) fn cached_provider_catalog(&self) -> Option<OpenCodeProviderCatalog> {
        self.provider_catalog_cache
            .get_fresh(PROVIDER_CATALOG_CACHE_TTL)
    }

    pub(crate) fn cache_provider_catalog(&mut self, catalog: OpenCodeProviderCatalog) {
        self.provider_catalog_cache.set(catalog.clone());
        self.update_provider_catalog_projection(catalog);
    }

    pub(crate) fn invalidate_provider_catalog_cache(&mut self) {
        self.provider_catalog_cache.clear();
        self.invalidate_provider_catalog_projection();
    }

    pub(super) fn provider_command_catalogs_response_for_app(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        provider_command_catalogs_response()
    }

    pub(super) fn relay_status_response_for_app(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::RelayStatus {
            status: self.relay_status_snapshot()?,
        })
    }

    pub(super) fn configure_relay_response(
        &mut self,
        request: ConfigureRelayRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.configure_relay(request.relay_url, request.relay_token)?;
        self.invalidate_provider_catalog_cache();
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

    pub(super) fn list_remote_machines_response(
        &mut self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (machines, _) = self.remote_relay_inventory_projection_store().snapshot();
        Ok(LocalDaemonResponse::RemoteMachinesListed { machines })
    }

    pub(super) fn list_remote_machine_kernels_response(
        &mut self,
        request: ListRemoteMachineKernelsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let machine_ref = resolve_registered_or_raw_machine_ref(&request.machine_ref);
        let (_, kernels) = self.remote_relay_inventory_projection_store().snapshot();
        let kernels = kernels
            .into_iter()
            .filter(|kernel| {
                kernel.machine_id == machine_ref
                    || kernel
                        .machine_alias
                        .as_deref()
                        .is_some_and(|alias| alias == machine_ref)
            })
            .collect();
        Ok(LocalDaemonResponse::RemoteMachineKernelsListed {
            machine_ref,
            kernels,
        })
    }

    pub(super) fn approve_remote_machine_response(
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
        self.provider_catalog_cache.clear();
        let machine = record_for_machine_id(machine.machine_id, live, &config.host_machine_id)?;
        Ok(LocalDaemonResponse::RemoteMachineApproved { machine })
    }

    pub(super) fn forget_remote_machine_response(
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
        self.provider_catalog_cache.clear();
        let machine = forgotten_machine_record(machine, saved.alias, live, &config.host_machine_id);
        Ok(LocalDaemonResponse::RemoteMachineForgotten { machine })
    }

    pub(super) fn rename_remote_machine_response(
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
        self.provider_catalog_cache.clear();
        let machine = record_for_machine_id(machine, live, &config.host_machine_id)?;
        Ok(LocalDaemonResponse::RemoteMachineRenamed { machine })
    }

    pub(super) fn provider_auth_status_response_for_app(
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

    pub(super) fn start_provider_login_response_for_app(
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

    pub(super) fn logout_provider_response_for_app(
        &mut self,
        request: LogoutProviderRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request.provider.as_str() {
            "codex" => {
                let response = logout_provider_response(request)?;
                self.provider_catalog_cache.clear();
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
    if let Some(endpoint) = request.structured_endpoint {
        launch_request = launch_request.with_structured_endpoint(endpoint);
    }
    if request.native_tui {
        launch_request = launch_request.with_client_interface(ProviderClientInterface::NativeTui);
    }
    if let Some(provider_session_id) = request.provider_session_id {
        if launch_request.adapter_key == "codex" {
            launch_request = launch_request.with_resume_state(
                crate::provider::ProviderResumeState::from_codex_thread_id(provider_session_id),
            );
        } else if launch_request.adapter_key == "opencode" {
            launch_request = launch_request.with_resume_state(
                crate::provider::ProviderResumeState::from_opencode_session_id(provider_session_id),
            );
        }
    }
    let session = app.sessions().get_session(&request.session_id).ok();
    let focused_agent_id = session
        .as_ref()
        .and_then(|session| session.focused_agent_id().map(str::to_string));
    if let Some(agent_id) = request.agent_id.clone().or(focused_agent_id) {
        launch_request = if let Ok(agent) = app.agents().get_agent(&agent_id) {
            let effective_config = session.as_ref().map(|session| {
                crate::session::effective_agent_execution_config(session, Some(&agent))
            });
            let launch_request = launch_request
                .with_agent_id(agent_id)
                .with_owner_user_id(agent.owner_user_id().to_string());
            if let Some(effective_config) = effective_config {
                launch_request
                    .with_execution_mode(effective_config.mode)
                    .with_permission_level(effective_config.permission_level)
            } else {
                launch_request
            }
        } else {
            launch_request.with_agent_id(agent_id)
        };
    } else {
        if let Some(session) = session.as_ref() {
            let effective_config = crate::session::effective_agent_execution_config(session, None);
            launch_request = launch_request
                .with_execution_mode(effective_config.mode)
                .with_permission_level(effective_config.permission_level);
        }
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
    let mut source_errors = Vec::new();

    match ensure_opencode_catalog_endpoint() {
        Ok(endpoint) => match OpenCodeClient::new("catalog", endpoint) {
            Ok(client) => match client.provider_catalog() {
                Ok(catalog) => catalogs.push(catalog),
                Err(error) => source_errors.push(format!("opencode catalog request: {error}")),
            },
            Err(error) => source_errors.push(format!("opencode client: {error}")),
        },
        Err(error) => source_errors.push(format!("opencode endpoint: {error}")),
    }
    match ensure_codex_catalog_endpoint() {
        Ok(endpoint) => match CodexClient::new("catalog", endpoint) {
            Ok(client) => match client.provider_catalog() {
                Ok(catalog) => catalogs.push(catalog),
                Err(error) => source_errors.push(format!("codex catalog request: {error}")),
            },
            Err(error) => source_errors.push(format!("codex client: {error}")),
        },
        Err(error) => source_errors.push(format!("codex endpoint: {error}")),
    }

    let remote_machines = if config.relay_url.is_some() && config.relay_token.is_some() {
        match block_on_relay_query(crate::transport::relay_discovery::list_live_machines(
            &config,
        )) {
            Ok(machines) => machines,
            Err(error) => {
                source_errors.push(format!("relay live machines: {error}"));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let approved_remote_machines =
        approved_live_remote_machines(&remote_machines, &config.host_machine_id);
    if !source_errors.is_empty() {
        crate::logging::warn_with_fields(
            "daemon.local",
            "Some provider catalog sources were unavailable",
            serde_json::json!({
                "source_errors": &source_errors,
            }),
        );
    }

    let mut catalog = merge_provider_catalogs(catalogs)
        .or_else(|| {
            remote_only_provider_catalog(&approved_remote_machines, &config.host_machine_id)
        })
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "get_provider_catalog",
            message: if source_errors.is_empty() {
                "no provider catalog sources were reachable".to_string()
            } else {
                format!(
                    "no provider catalog sources were reachable: {}",
                    source_errors.join("; ")
                )
            },
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

pub(crate) fn resolve_machine_for_registry(
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

pub(crate) fn resolve_machine_id_for_registry(
    machine_ref: &str,
    live_machines: &[RelayMachinePresence],
) -> Result<String, DaemonError> {
    if let Some(machine_id) = DaemonConfig::resolve_registered_machine_ref(machine_ref) {
        return Ok(machine_id);
    }
    resolve_machine_for_registry(machine_ref, live_machines).map(|machine| machine.machine_id)
}

pub(crate) fn record_for_machine_id(
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

pub(crate) fn forgotten_machine_record(
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
    use crate::app::KernelSessionService;
    use crate::provider::OpenCodeProviderInfo;
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    use crate::session::{CreateSessionRequest, SessionAgentDefaults};

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

    #[test]
    fn launch_provider_request_inherits_session_agent_defaults() {
        let mut app =
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon app should boot");
        let defaults = SessionAgentDefaults::new("dev-stub")
            .with_model("model-a")
            .with_effort("low")
            .with_execution_mode(AgentExecutionMode::Plan)
            .with_permission_level(AgentPermissionLevel::Required);
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(
                CreateSessionRequest::new("workspace", "worktree").with_agent_defaults(defaults),
            )
            .expect("session should be created");

        let request = launch_provider_request_from_local(
            &app,
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: Some(agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: "dev-stub".to_string(),
                account_profile: "default".to_string(),
                model: "model-a".to_string(),
                variant: Some("low".to_string()),
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
        );

        assert_eq!(request.execution_mode, Some(AgentExecutionMode::Plan));
        assert_eq!(
            request.permission_level,
            Some(AgentPermissionLevel::Required)
        );
    }
}
