use crate::app::DaemonApp;
use crate::config::{DaemonConfig, PersistedMachineRegistration};
use crate::error::DaemonError;
use crate::provider::{LaunchProviderRequest, OpenCodeProviderCatalog, ProviderClientInterface};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;
use arroba_relay::protocol::RelayMachinePresence;

use super::api::{
    ApproveRemoteMachineRequest, ConfigureRelayRequest, ForgetRemoteMachineRequest,
    GetProviderAuthStatusRequest, GetProviderRunRequest, LaunchProviderRunRequest,
    ListRemoteMachineKernelsRequest, LocalDaemonResponse, LogoutProviderRequest, RelayStatus,
    RemoteMachineRecord, RemoteMachineTrustStatus, RenameRemoteMachineRequest,
    StartProviderLoginRequest, UpdateProviderRunSelectionRequest,
};

mod blocking;
mod catalog;

use blocking::block_on_relay_query;
pub(crate) use catalog::{
    load_provider_catalog, logout_provider_response, provider_auth_status_response,
    provider_command_catalogs_response, start_provider_login_response, PROVIDER_CATALOG_CACHE_TTL,
};

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
        if request.native_tui {
            if let Some(response) = remote_native_provider_run_response(self, &request)? {
                return Ok(response);
            }
        }
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

fn remote_native_provider_run_response(
    app: &mut DaemonApp,
    request: &LaunchProviderRunRequest,
) -> Result<Option<LocalDaemonResponse>, DaemonError> {
    let session = app.sessions().get_session(&request.session_id)?;
    let agent_id = request
        .agent_id
        .clone()
        .or_else(|| session.focused_agent_id().map(str::to_string));
    let Some(agent_id) = agent_id else {
        return Ok(None);
    };
    let agent = app.agents().get_agent(&agent_id)?;
    let Some(remote_execution) = agent.remote_execution().cloned() else {
        return Ok(None);
    };
    let required_mcps =
        required_remote_mcps_for_native_provider_launch(app, &request.session_id, &agent)?;
    let relay_config = app.relay_config_for_remote_execution(&remote_execution);
    let response = app.block_on_relay_future(send_peer_request_via_temporary_connection(
        &relay_config,
        ClientTarget {
            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::LaunchLeasedNativeProviderRun {
            leased_agent_id: remote_execution.leased_agent_id,
            adapter_key: request.adapter_key.clone(),
            provider: request.provider.clone(),
            account_profile: request.account_profile.clone(),
            model: request.model.clone(),
            variant: request.variant.clone(),
            structured_endpoint: request.structured_endpoint.clone(),
            provider_session_id: request.provider_session_id.clone(),
            required_mcps,
        },
    ))?;
    match response {
        RelayPeerResponse::LeasedNativeProviderRunLaunched { provider_run } => {
            let agent_id = request
                .agent_id
                .clone()
                .or_else(|| {
                    app.sessions()
                        .get_session(&request.session_id)
                        .ok()
                        .and_then(|session| session.focused_agent_id().map(str::to_string))
                })
                .or_else(|| {
                    app.agents()
                        .get_focused_agent(&request.session_id)
                        .map(|agent| agent.id().to_string())
                })
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "launch remote native provider run",
                    message: format!(
                        "no focused agent available for remote native provider run in session `{}`",
                        request.session_id
                    ),
                })?;
            let provider_run =
                provider_run.projected_for_home_agent(request.session_id.clone(), agent_id);
            app.update_provider_run_projection(provider_run.clone());
            app.sessions_mut().set_active_provider_run(
                provider_run.session_id(),
                Some(provider_run.id().to_string()),
            )?;
            Ok(Some(LocalDaemonResponse::ProviderRunLaunched {
                provider_run,
            }))
        }
        other => Err(DaemonError::LocalTransport {
            operation: "launch remote native provider run",
            message: format!("unexpected remote native provider launch response: {other:?}"),
        }),
    }
}

fn required_remote_mcps_for_native_provider_launch(
    app: &DaemonApp,
    session_id: &str,
    agent: &crate::agent::AgentInstance,
) -> Result<Vec<crate::transport::relay_peer::RequiredRemoteMcp>, DaemonError> {
    if agent.mcp_grants().is_empty() {
        return Ok(Vec::new());
    }
    let session = app.sessions().get_session(session_id)?;
    let workspace = std::path::PathBuf::from(session.workspace_id());
    let mut roots = vec![crate::mcp::ArrobaMcpRegistry::project_root(&workspace)];
    if let Some(user_root) = crate::mcp::ArrobaMcpRegistry::user_root() {
        roots.push(user_root);
    }
    let registry = crate::mcp::ArrobaMcpRegistry::new(roots);
    agent
        .mcp_grants()
        .iter()
        .map(|grant| {
            let config = registry
                .get(grant)?
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "launch remote native provider run",
                    message: format!("MCP `{grant}` is granted but is not installed"),
                })?;
            Ok(crate::transport::relay_peer::RequiredRemoteMcp {
                definition_hash: config.definition_hash()?,
                config,
            })
        })
        .collect()
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
        } else if launch_request.adapter_key == "claude" {
            launch_request = launch_request.with_resume_state(
                crate::provider::ProviderResumeState::from_claude_session_id(provider_session_id),
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
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    use crate::session::{CreateSessionRequest, SessionAgentDefaults};

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
