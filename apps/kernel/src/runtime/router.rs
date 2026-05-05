use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};

use arroba_relay::protocol::RelayKernelPresence;

use crate::agent::AgentState;
use crate::app::DaemonApp;
use crate::config::PersistedCloudRelayProfile;
use crate::error::DaemonError;
use crate::history::SessionHistoryStore;
use crate::history::{
    HistoryEventKind, HistoryEventQuery, HistoryEventRole, OperationalHistoryStore,
};
use crate::history_archive::HistoryArchiveClient;
use crate::local::provider_requests::{
    PROVIDER_CATALOG_CACHE_TTL, forgotten_machine_record, load_provider_catalog,
    logout_provider_response, provider_auth_status_response, provider_command_catalogs_response,
    record_for_machine_id, resolve_machine_for_registry, resolve_machine_id_for_registry,
    start_provider_login_response,
};
use crate::local::{
    AcceptCloudSessionInviteRequest, AgentGrantKind, ApproveRemoteMachineRequest,
    AttachWorkspaceLinkRequest, CloudCollaborator, CloudRelayLoginPoll, CloudRelayLoginPollStatus,
    CloudRelayLoginStart, CloudRelayProfile, CloudRelayRuntimeToken, CloudSessionInvite,
    CloudSessionInviteAcceptance, CloudSessionInviteDetails, CloudSessionMember,
    ConfigureRelayRequest, ConnectCloudRelayRequest, CreateCloudSessionInviteRequest,
    CreatePairingInviteRequest, CreateSessionInviteRequest, CreateTerminalPairingLinkRequest,
    CreateWorkspaceDirectoryRequest, CreateWorkspaceLinkRequest, CreateWorkspaceWorktreeRequest,
    DeleteCredentialSecretRequest, DeleteKernelRequest, DetachWorkspaceLinkRequest,
    ForgetRemoteMachineRequest, GetMcpServerRequest, GetPromptInputHistoryRequest,
    GetProviderAuthStatusRequest, GetProviderRunRequest, GetSessionHistoryRequest,
    GetSessionStateRequest, GetSkillRequest, GetUserConfigRequest, GetUserConfigSchemaRequest,
    GrantAgentCapabilityRequest, ImportMcpServersRequest, ImportSkillsRequest,
    InstallMcpServerRequest, InstallSkillRequest, IssueCloudRelayClientTokenRequest,
    JoinPairingInviteRequest, JoinSessionInviteRequest, JoinTerminalPairingLinkRequest,
    ListAgentsRequest, ListCloudCollaboratorsRequest, ListCloudSessionMembersRequest,
    ListMcpServersRequest, ListProviderProcessesRequest, ListSessionMembersRequest,
    ListSessionsRequest, ListSkillsRequest, ListWorkspaceLinksRequest,
    ListWorkspaceWorktreesRequest, LocalDaemonRequest, LocalDaemonResponse,
    LogoutCloudRelayRequest, LogoutProviderRequest, MoveAgentToRemoteRequest,
    PairCloudRelayClientRequest, PairCloudRelayMachineRequest, PairedClientRecord,
    PairingInviteIntent, PairingInviteRecord, PairingJoinRecord, PollCloudRelayLoginRequest,
    PromptInputHistoryEntry, PromptInputHistoryEntryKind, PumpTerminalOutputRequest,
    QueryHistoryRequest, RecordPairedClientRequest, RecordPromptInputHistoryRequest, RelayStatus,
    RenameRemoteMachineRequest, ResolveSessionRequest, RevokeAgentCapabilityRequest,
    RevokeCloudSessionInviteRequest, RevokePairedClientRequest, RevokeSessionInviteRequest,
    SearchHistoryRequest, SearchWorkspaceDirectoriesRequest, SessionInviteRecord,
    SetCredentialSecretRequest, SetUserConfigValueRequest, ShowCloudSessionInviteRequest,
    ShowWorkspaceLinkRequest, StartCloudRelayLoginRequest, StartProviderLoginRequest,
    TeardownProviderProcessesRequest, TerminalPairingLinkRecord, TerminalRecord, TerminalType,
    UninstallMcpServerRequest, UninstallSkillRequest, UnsetUserConfigValueRequest,
    UpdateMcpServerRequest, UpdateSkillRequest, UserConfigMutationEffect,
    UserConfigProviderReloadSummary, WaitingRoomLaunchTarget, WaitingRoomPublicAgentSummary,
    WaitingRoomPublicItemActivitySummary, WaitingRoomPublicSessionSummary,
    WaitingRoomPublicSnapshot, WaitingRoomPublicWorkflowEdgeSummary,
    WaitingRoomPublicWorkflowEndpointSummary, WaitingRoomPublicWorkflowNodeSummary,
    WaitingRoomPublicWorkflowSummary, WaitingRoomSessionActivitySummary, WorkspaceWorktreeRecord,
};
use crate::provider::{
    ProviderNativeInteractionBridge, ProviderNativeInteractionResolution,
    ProviderRunOperationLanes, ProviderRunState,
};
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::capability_executor::{
    CapabilityExecutorHealthStore, CapabilityRuntimeStore, execute_capability_request,
};
use crate::runtime::command::{
    KernelCallerKind, KernelCommand, KernelCommandPriority, KernelCommandSource,
};
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, DaemonConfigProjectionStore, DaemonHealthProjection,
    ProviderCatalogProjectionStore, ProviderProcessProjectionStore, ProviderRunProjectionStore,
    RemoteRelayInventoryProjectionStore, SessionHistoryProjectionStore,
    SessionStateProjectionStore, TransportHealthStore, page_history_entries,
};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::runtime::provider_launch_executor::ProviderLaunchCommandExecutor;
use crate::runtime::session_actor::{FocusedAgentProjection, SessionActor, SessionRuntime};
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::state::{ProviderReloadOutcome, ProviderReloadTrigger};
use crate::runtime::terminal_output_executor::TerminalOutputExecutor;
use crate::runtime::workflow_actor::{WorkflowRuntime, is_workflow_command};
use crate::runtime::workspace_coordinator::WorkspaceCoordinator;
use crate::session::{DEFAULT_LOCAL_USER_ID, PromptIdAllocator, unix_epoch_ms};
use crate::terminal::{TerminalStreamHealthStore, TerminalStreamStore};
use crate::transport::relay_client::{
    RelayClientState, refresh_remote_inventory_projection_for_app_with_relay_state,
};

pub(crate) const INTERACTIVE_COMMAND_QUEUE_LIMIT: usize = 128;
const CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS: u64 = 300_000;
const CLOUD_RELAY_TOKEN_REFRESH_WINDOW_MS: u64 = 60_000;

enum UserConfigMutation {
    Set { path: String, value: String },
    Unset { path: String },
}

struct RuntimeStateNativeInteractionBridge {
    handle: tokio::runtime::Handle,
    app: Arc<Mutex<DaemonApp>>,
    state: KernelRuntimeState,
}

impl ProviderNativeInteractionBridge for RuntimeStateNativeInteractionBridge {
    fn request_blocking(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
    ) -> Result<ProviderNativeInteractionResolution, DaemonError> {
        let session_id = session_id.to_string();
        let interaction_agent_id = interaction.agent_id().to_string();
        let remote_target = self.handle.block_on(async {
            let mut app = self.app.lock().await;
            let target = crate::app::RemoteLeaseRuntime::new(&mut app)
                .native_interaction_context_for_backing_agent(
                    &session_id,
                    &interaction_agent_id,
                    "unknown",
                );
            Ok::<_, DaemonError>(
                target.map(|(daemon_id, context)| (app.config().clone(), daemon_id, context)),
            )
        })?;
        if let Some((config, target_daemon_id, context)) = remote_target {
            let response = self.handle.block_on(async move {
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    &config,
                    arroba_relay::protocol::ClientTarget {
                        daemon_id: Some(target_daemon_id),
                        daemon_alias: None,
                    },
                    crate::transport::relay_peer::RelayPeerRequest::ForwardNativeInteraction {
                        context,
                        interaction,
                    },
                )
                .await
            })?;
            return match response {
                crate::transport::relay_peer::RelayPeerResponse::NativeInteractionResolved {
                    resolution,
                } => Ok(resolution),
                other => Err(DaemonError::LocalTransport {
                    operation: "provider_native_interaction_bridge",
                    message: format!(
                        "unexpected relay response for remote native interaction: {other:?}"
                    ),
                }),
            };
        }
        let state = self.state.clone();
        let resolution = self.handle.block_on(async move {
            let receiver = state
                .create_runtime_interaction(&session_id, interaction)
                .await?;
            receiver.await.map_err(|error| DaemonError::LocalTransport {
                operation: "provider_native_interaction_bridge",
                message: format!("interaction dropped before resolution: {error}"),
            })
        })?;
        Ok(ProviderNativeInteractionResolution {
            status: resolution.status.to_string(),
            choice_id: resolution.choice_id,
            reply: resolution.reply,
        })
    }
}

fn summarize_provider_reload_outcomes(
    outcomes: &[ProviderReloadOutcome],
) -> UserConfigProviderReloadSummary {
    let mut summary = UserConfigProviderReloadSummary {
        reloaded: 0,
        deferred: 0,
        unaffected: 0,
    };
    for outcome in outcomes {
        match outcome {
            ProviderReloadOutcome::Reloaded => summary.reloaded += 1,
            ProviderReloadOutcome::Deferred => summary.deferred += 1,
            ProviderReloadOutcome::Unaffected => summary.unaffected += 1,
        }
    }
    summary
}

fn user_config_path_requires_daemon_restart(path: &str) -> bool {
    matches!(
        path,
        "history.operational.backend"
            | "history.operational.path"
            | "state.backend"
            | "state.path"
            | "kernel.websocket_host"
            | "kernel.websocket_port"
            | "kernel.runtime_mcp_host"
            | "kernel.runtime_mcp_port"
    )
}

fn user_config_path_is_unwired(path: &str) -> bool {
    matches!(
        path,
        "providers.default"
            | "providers.model"
            | "providers.account_profile"
            | "providers.effort"
            | "ui.theme"
            | "ui.multi_agent_response_layout"
            | "ui.max_agents_per_screen"
            | "relay.url"
            | "relay.accept_remote_leases"
            | "history.operational.retention_days"
            | "history.operational.max_size_mb"
            | "history.operational.keep_pinned_sessions"
            | "history.operational.archive_inactive_after_days"
            | "history.operational.archive_deleted_agents"
            | "history.archive.archive_deleted_agents"
            | "history.archive.archive_before_delete"
            | "history.archive.delete_operational_after_verified_archive"
            | "artifacts.operational.retention_days"
    ) || path.starts_with("ui.worktree_aliases.")
}

#[derive(Clone)]
pub(crate) struct CommandRouter {
    app: Arc<Mutex<DaemonApp>>,
    runtime_state: KernelRuntimeState,
    agent_runtime: AgentRuntime,
    session_runtime: SessionRuntime,
    workflow_runtime: WorkflowRuntime,
    provider_runtime_lanes: ProviderRunOperationLanes,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    history_store: SessionHistoryStore,
    operational_history_store: OperationalHistoryStore,
    history_projection: SessionHistoryProjectionStore,
    provider_catalog_projection: ProviderCatalogProjectionStore,
    provider_run_projection: ProviderRunProjectionStore,
    provider_process_projection: ProviderProcessProjectionStore,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    capability_health: CapabilityExecutorHealthStore,
    capability_runtime: CapabilityRuntimeStore,
    transport_health: TransportHealthStore,
    terminal_health: TerminalStreamHealthStore,
    terminal_stream: TerminalStreamStore,
    terminal_output_executor: TerminalOutputExecutor,
    workspace_coordinator: WorkspaceCoordinator,
    pending_provider_launch_sessions: Arc<Mutex<HashSet<String>>>,
}

impl CommandRouter {
    #[cfg(test)]
    pub(crate) fn with_interactive_capacity(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
    ) -> Self {
        Self::with_interactive_capacity_and_provider_lanes(
            app,
            interactive_capacity,
            ProviderRunOperationLanes::default(),
        )
    }

    #[cfg(test)]
    fn with_interactive_and_session_capacity(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
        session_capacity: usize,
    ) -> Self {
        let _interactive_capacity = interactive_capacity;
        let provider_runtime_lanes = ProviderRunOperationLanes::default();
        let focus_projection = FocusedAgentProjection::default();
        let (
            history_store,
            operational_history_store,
            session_projection,
            history_projection,
            provider_catalog_projection,
            provider_run_projection,
            provider_process_projection,
            remote_relay_inventory_projection,
            agent_runtime_projection,
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            prompt_activity,
            prompt_idle_timeout,
            prompt_workspace_claims,
            structured_output_records,
            durable_state_store,
            relay_state,
            terminal_health,
            terminal_stream,
            workspace_coordinator,
            prompt_state_owner,
            prompt_id_allocator,
        ) = router_projection_stores(&app);
        let runtime_state = KernelRuntimeState::new_with_owned_state(
            Arc::clone(&app),
            config_projection.clone(),
            session_store.clone(),
            agent_store.clone(),
            attachment_store.clone(),
            provider_store.clone(),
            provider_process_tracking.clone(),
            session_projection.clone(),
            provider_run_projection.clone(),
            history_store.clone(),
            operational_history_store.clone(),
            durable_state_store.clone(),
            history_projection.clone(),
            prompt_state_owner.clone(),
            prompt_activity.clone(),
            prompt_idle_timeout,
            prompt_workspace_claims.clone(),
            structured_output_records.clone(),
            terminal_stream.clone(),
            workspace_coordinator.clone(),
        );
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            provider_store.set_native_interaction_bridge(Arc::new(
                RuntimeStateNativeInteractionBridge {
                    handle,
                    app: Arc::clone(&app),
                    state: runtime_state.clone(),
                },
            ));
        }
        let pending_provider_launch_sessions = Arc::new(Mutex::new(HashSet::new()));
        let capability_runtime = CapabilityRuntimeStore::new(runtime_state.clone());
        let agent_runtime = AgentRuntime::new(
            runtime_state.clone(),
            provider_runtime_lanes.clone(),
            focus_projection.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            prompt_state_owner.clone(),
            prompt_id_allocator.clone(),
        );
        let session_runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            runtime_state.clone(),
            session_capacity,
            focus_projection.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            terminal_stream.clone(),
        );
        let workflow_runtime = WorkflowRuntime::new(
            runtime_state.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
        );
        let terminal_output_executor = TerminalOutputExecutor::new(
            runtime_state.clone(),
            provider_runtime_lanes.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            provider_run_projection.clone(),
            terminal_stream.clone(),
        );
        Self {
            app,
            runtime_state,
            agent_runtime,
            session_runtime,
            workflow_runtime,
            provider_runtime_lanes,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            history_store,
            operational_history_store,
            history_projection,
            provider_catalog_projection,
            provider_run_projection,
            provider_process_projection,
            remote_relay_inventory_projection,
            config_projection,
            relay_state,
            capability_health: CapabilityExecutorHealthStore::default(),
            capability_runtime,
            transport_health: TransportHealthStore::default(),
            terminal_health,
            terminal_stream,
            terminal_output_executor,
            workspace_coordinator,
            pending_provider_launch_sessions,
        }
    }

    pub(crate) fn with_interactive_capacity_and_provider_lanes(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) -> Self {
        Self::with_interactive_capacity_provider_lanes_and_transport_health(
            app,
            interactive_capacity,
            provider_runtime_lanes,
            TransportHealthStore::default(),
        )
    }

    pub(crate) fn with_interactive_capacity_provider_lanes_and_transport_health(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
        provider_runtime_lanes: ProviderRunOperationLanes,
        transport_health: TransportHealthStore,
    ) -> Self {
        let _interactive_capacity = interactive_capacity;
        let focus_projection = FocusedAgentProjection::default();
        let (
            history_store,
            operational_history_store,
            session_projection,
            history_projection,
            provider_catalog_projection,
            provider_run_projection,
            provider_process_projection,
            remote_relay_inventory_projection,
            agent_runtime_projection,
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            prompt_activity,
            prompt_idle_timeout,
            prompt_workspace_claims,
            structured_output_records,
            durable_state_store,
            relay_state,
            terminal_health,
            terminal_stream,
            workspace_coordinator,
            prompt_state_owner,
            prompt_id_allocator,
        ) = router_projection_stores(&app);
        let runtime_state = KernelRuntimeState::new_with_owned_state(
            Arc::clone(&app),
            config_projection.clone(),
            session_store.clone(),
            agent_store.clone(),
            attachment_store.clone(),
            provider_store.clone(),
            provider_process_tracking.clone(),
            session_projection.clone(),
            provider_run_projection.clone(),
            history_store.clone(),
            operational_history_store.clone(),
            durable_state_store.clone(),
            history_projection.clone(),
            prompt_state_owner.clone(),
            prompt_activity.clone(),
            prompt_idle_timeout,
            prompt_workspace_claims.clone(),
            structured_output_records.clone(),
            terminal_stream.clone(),
            workspace_coordinator.clone(),
        );
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            provider_store.set_native_interaction_bridge(Arc::new(
                RuntimeStateNativeInteractionBridge {
                    handle,
                    app: Arc::clone(&app),
                    state: runtime_state.clone(),
                },
            ));
        }
        let pending_provider_launch_sessions = Arc::new(Mutex::new(HashSet::new()));
        let capability_runtime = CapabilityRuntimeStore::new(runtime_state.clone());
        let agent_runtime = AgentRuntime::new(
            runtime_state.clone(),
            provider_runtime_lanes.clone(),
            focus_projection.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            prompt_state_owner.clone(),
            prompt_id_allocator.clone(),
        );
        let session_runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            runtime_state.clone(),
            crate::runtime::session_actor::SESSION_COMMAND_QUEUE_LIMIT,
            focus_projection.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            terminal_stream.clone(),
        );
        let workflow_runtime = WorkflowRuntime::new(
            runtime_state.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
        );
        let terminal_output_executor = TerminalOutputExecutor::new(
            runtime_state.clone(),
            provider_runtime_lanes.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            provider_run_projection.clone(),
            terminal_stream.clone(),
        );
        Self {
            app,
            runtime_state,
            agent_runtime,
            session_runtime,
            workflow_runtime,
            provider_runtime_lanes,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            history_store,
            operational_history_store,
            history_projection,
            provider_catalog_projection,
            provider_run_projection,
            provider_process_projection,
            remote_relay_inventory_projection,
            config_projection,
            relay_state,
            capability_health: CapabilityExecutorHealthStore::default(),
            capability_runtime,
            transport_health,
            terminal_health,
            terminal_stream,
            terminal_output_executor,
            workspace_coordinator,
            pending_provider_launch_sessions,
        }
    }

    pub(crate) async fn local_command_caller(
        &self,
        source: KernelCommandSource,
    ) -> crate::runtime::command::KernelCaller {
        let mut caller = crate::runtime::command::KernelCaller::for_source(&source);
        let cloud_profile = {
            let app = self.app.lock().await;
            app.config().cloud_relay.clone()
        };
        if let Some(profile) = cloud_profile {
            caller.user_id = Some(profile.user_id);
            caller.client_id = profile.client_id;
            caller.machine_id = profile.machine_id;
            caller.realm_id = Some(profile.realm_id);
        }
        caller
    }

    pub(crate) fn runtime_mcp_bind_address(&self) -> (String, u16) {
        let config = self.config_projection.snapshot();
        (config.runtime_mcp_host, config.runtime_mcp_port)
    }

    pub(crate) async fn dispatch_authenticated_mcp_proxy_call(
        &self,
        auth_token: &str,
        name: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        crate::mcp::validate_registry_name(name, "mcp name")?;
        let backing = {
            let app = self.app.lock().await;
            let run = app
                .providers()
                .get_run_by_runtime_mcp_auth_token(auth_token)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "mcp.proxy.auth",
                    message: "invalid runtime MCP auth token".to_string(),
                })?;
            run.mcp_servers()
                .iter()
                .find(|server| server.name == name && server.enabled)
                .cloned()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "mcp.proxy.grant",
                    message: format!("MCP `{name}` is not granted to provider run `{}`", run.id()),
                })?
        };
        tokio::task::spawn_blocking(move || {
            crate::provider::dispatch_provider_mcp_proxy_request(&backing, payload)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "mcp.proxy.dispatch",
            message: error.to_string(),
        })?
    }

    pub(crate) fn relay_config_snapshot(&self) -> crate::config::DaemonConfig {
        self.config_projection.snapshot()
    }

    pub(crate) fn cloud_relay_token_refresh_due(&self) -> bool {
        let config = self.config_projection.snapshot();
        let Some(profile) = config.cloud_relay else {
            return false;
        };
        if profile.cloud_session_token.is_none() && profile.machine_credential.is_none() {
            return false;
        }
        config.relay_url.as_deref() != Some(profile.relay_url.as_str())
            || config.relay_token.is_none()
            || profile.token_expires_at_ms.is_none_or(|expires_at| {
                expires_at <= crate::session::unix_epoch_ms() + CLOUD_RELAY_TOKEN_REFRESH_WINDOW_MS
            })
    }

    pub(crate) async fn ensure_cloud_relay_connection(&self) -> Result<(), DaemonError> {
        let config = self.config_projection.snapshot();
        let Some(profile) = config.cloud_relay.clone() else {
            return Ok(());
        };
        if profile.cloud_session_token.is_none() && profile.machine_credential.is_none() {
            return Ok(());
        }
        let now_ms = crate::session::unix_epoch_ms();
        let token_is_fresh = config.relay_url.as_deref() == Some(profile.relay_url.as_str())
            && config.relay_token.is_some()
            && profile.token_expires_at_ms.is_some_and(|expires_at| {
                expires_at > now_ms + CLOUD_RELAY_TOKEN_REFRESH_WINDOW_MS
            });
        if token_is_fresh {
            return Ok(());
        }

        let (subject, subject_kind, machine_id) =
            if let Some(machine_id) = profile.machine_id.clone() {
                (machine_id.clone(), "machine", Some(machine_id))
            } else {
                (config.daemon_id, "kernel", None)
            };
        let issued = match issue_cloud_runtime_token(
            &profile,
            &subject,
            subject_kind,
            None,
            None,
            machine_id,
            None,
        )
        .await
        {
            Ok(issued) => issued,
            Err(error) => {
                self.clear_cloud_profile_if_stale(&error).await?;
                return Err(error);
            }
        };
        let mut updated_profile = profile.clone();
        updated_profile.token_expires_at_ms = Some(now_ms + CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS);
        {
            let mut app = self.app.lock().await;
            app.configure_relay(Some(profile.relay_url), Some(issued.token))?;
            app.persist_cloud_relay_profile(Some(updated_profile))?;
            self.config_projection.update(app.config().clone());
        }
        Ok(())
    }

    pub(crate) async fn publish_cloud_kernel_presence(
        &self,
        online: bool,
    ) -> Result<(), DaemonError> {
        let config = self.config_projection.snapshot();
        let Some(profile) = config.cloud_relay else {
            return Ok(());
        };
        let Some(machine_id) = profile.machine_id.clone() else {
            return Ok(());
        };
        if profile.cloud_session_token.is_none() && profile.machine_credential.is_none() {
            return Ok(());
        }
        let mut body = serde_json::Map::new();
        if let Some(machine_credential) = profile.machine_credential.clone() {
            body.insert(
                "machineCredential".to_string(),
                serde_json::Value::String(machine_credential),
            );
        } else if let Some(session_token) = profile.cloud_session_token.clone() {
            body.insert(
                "sessionToken".to_string(),
                serde_json::Value::String(session_token),
            );
        }
        body.insert(
            "accountId".to_string(),
            serde_json::Value::String(profile.account_id),
        );
        body.insert(
            "realmId".to_string(),
            serde_json::Value::String(profile.realm_id),
        );
        body.insert(
            "machineId".to_string(),
            serde_json::Value::String(machine_id),
        );
        body.insert(
            "kernelId".to_string(),
            serde_json::Value::String(config.daemon_id),
        );
        if let Some(alias) = config.daemon_alias {
            body.insert("kernelAlias".to_string(), serde_json::Value::String(alias));
        }
        body.insert(
            "status".to_string(),
            serde_json::Value::String(if online { "ONLINE" } else { "OFFLINE" }.to_string()),
        );
        body.insert(
            "metadata".to_string(),
            serde_json::json!({
                "host": config.kernel_websocket_host,
                "port": config.kernel_websocket_port,
            }),
        );
        let _: serde_json::Value = post_cloud_json(
            profile.api_url,
            "/kernels/presence",
            serde_json::Value::Object(body),
        )
        .await?;
        Ok(())
    }

    pub(crate) fn relay_daemon_id(&self) -> String {
        self.config_projection.snapshot().daemon_id
    }

    pub(crate) fn relay_private_key(&self) -> String {
        self.config_projection.snapshot().relay_private_key
    }

    pub(crate) async fn relay_registration(&self) -> arroba_relay::protocol::DaemonRegistration {
        let mut app = self.app.lock().await;
        app.relay_registration()
    }

    pub(crate) async fn ensure_relay_subscription_attachment(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<(), DaemonError> {
        if let Some(session) = self.session_projection.get(session_id) {
            if session.has_attachment(attachment_id) {
                return Ok(());
            }
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        let app = self.app.lock().await;
        crate::app::KernelSessionReadService::new(&app)
            .ensure_attachment_in_session(session_id, attachment_id)
            .map(|_| ())
    }

    pub(crate) async fn relay_watch_subscription_state(
        &self,
        session_id: &str,
        attachment_id: &str,
        tick: u64,
        previous_snapshot: Option<crate::runtime::projection::SessionSnapshotProjection>,
    ) -> crate::runtime_transport::WatchResult {
        let mut app = self.app.lock().await;
        crate::runtime_transport::watch_subscription_state(
            &mut app,
            session_id,
            attachment_id,
            tick,
            previous_snapshot,
        )
    }

    pub(crate) async fn relay_create_execution_lease(
        &self,
        home_kernel_id: &str,
        home_session_id: &str,
        home_agent_id: &str,
        owner_user_id: &str,
    ) -> Result<crate::execution_lease::ExecutionLease, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).create_execution_lease(
            home_kernel_id,
            home_session_id,
            home_agent_id,
            owner_user_id,
        )
    }

    pub(crate) async fn relay_destroy_execution_lease(
        &self,
        lease_id: &str,
    ) -> Result<crate::execution_lease::ExecutionLease, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).destroy_execution_lease(lease_id)
    }

    pub(crate) async fn relay_create_leased_agent(
        &self,
        lease_id: &str,
        provider: &str,
        model: Option<String>,
        effort: Option<String>,
        execution_mode: Option<crate::provider::AgentExecutionMode>,
        permission_level: Option<crate::provider::AgentPermissionLevel>,
        worktree_id: Option<String>,
        worktree_placement: Option<crate::agent::GitWorktreePlacement>,
    ) -> Result<crate::execution_lease::LeasedAgent, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).create_leased_agent(
            lease_id,
            provider,
            model,
            effort,
            execution_mode,
            permission_level,
            worktree_id,
            worktree_placement,
        )
    }

    pub(crate) async fn relay_destroy_leased_agent(
        &self,
        leased_agent_id: &str,
    ) -> Result<crate::execution_lease::LeasedAgent, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).destroy_leased_agent(leased_agent_id)
    }

    pub(crate) async fn relay_submit_leased_prompt(
        &self,
        leased_agent_id: &str,
        prompt: &str,
        attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
        workflow_context: Option<crate::execution_lease::RemoteWorkflowTurnContext>,
        git_context: Option<crate::transport::relay_peer::RemoteGitTurnContext>,
        required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
    ) -> Result<(String, crate::session::PromptSubmissionOutcome), DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).submit_leased_prompt_with_workflow_context(
            leased_agent_id,
            prompt,
            attachments,
            workflow_context,
            git_context,
            required_mcps,
        )
    }

    pub(crate) async fn relay_ensure_remote_skill_packages(
        &self,
        context: crate::transport::relay_peer::RemoteSkillSyncContext,
        packages: Vec<crate::skill::ArrobaSkillPackage>,
    ) -> Result<Vec<crate::transport::relay_peer::RemoteSkillMaterialization>, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app)
            .ensure_remote_skill_packages(context, packages)
    }

    pub(crate) async fn relay_check_remote_mcp_availability(
        &self,
        context: crate::transport::relay_peer::RemoteMcpCheckContext,
        required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
    ) -> Result<Vec<crate::transport::relay_peer::RemoteMcpAvailability>, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app)
            .check_remote_mcp_availability(context, required_mcps)
    }

    pub(crate) async fn relay_complete_leased_prompt(
        &self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).complete_leased_prompt(leased_agent_id)
    }

    pub(crate) async fn relay_observe_leased_git_after(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
    ) -> Result<Vec<crate::transport::relay_peer::RemoteGitObservation>, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app)
            .observe_leased_git_after(leased_agent_id, provider_run_id)
    }

    pub(crate) async fn relay_cancel_leased_prompt(
        &self,
        leased_agent_id: &str,
    ) -> Result<crate::session::PromptCancellation, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).cancel_leased_prompt(leased_agent_id)
    }

    pub(crate) async fn relay_leased_agent_provider_run_id(
        &self,
        leased_agent_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).leased_agent_provider_run_id(leased_agent_id)
    }

    pub(crate) async fn relay_provider_run_terminal_diagnostic(
        &self,
        provider_run_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let app = self.app.lock().await;
        Ok(app
            .providers()
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.terminal_diagnostic().map(str::to_string))
            .filter(|message| !message.trim().is_empty()))
    }

    pub(crate) async fn relay_pump_leased_runtime_projections(
        &self,
    ) -> Result<Vec<(String, crate::transport::relay_peer::RelayPeerEvent)>, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).pump_leased_runtime_projections()
    }

    pub(crate) async fn relay_drain_leased_runtime_projection(
        &self,
        leased_agent_id: &str,
        provider_run_id: &str,
        pump_output: bool,
    ) -> Result<Option<(String, crate::transport::relay_peer::RelayPeerEvent)>, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).drain_leased_runtime_projection(
            leased_agent_id,
            provider_run_id,
            pump_output,
        )
    }

    pub(crate) async fn relay_project_remote_runtime_projection(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        output_chunks: Vec<crate::transport::relay_peer::RelayProjectedOutputChunk>,
        notices: Vec<String>,
        completions: Vec<crate::transport::relay_peer::RelayProjectedCompletion>,
    ) -> Result<(), DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).project_remote_runtime_projection(
            session_id,
            agent_id,
            provider_run_id,
            output_chunks,
            notices,
            completions,
        )
    }

    pub(crate) async fn relay_forward_native_interaction(
        &self,
        context: crate::transport::relay_peer::RemoteNativeInteractionContext,
        interaction: crate::session::RuntimeInteraction,
    ) -> Result<crate::provider::ProviderNativeInteractionResolution, DaemonError> {
        let interaction = interaction.with_agent_id(context.home_agent_id.clone());
        let timeout = interaction
            .timeout_sec()
            .map(std::time::Duration::from_secs);
        let timeout_session_id = context.home_session_id.clone();
        let timeout_interaction_id = interaction.id().to_string();
        let receiver = self
            .runtime_state
            .create_runtime_interaction(&context.home_session_id, interaction)
            .await?;
        if let Some(timeout) = timeout {
            let state = self.runtime_state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(timeout).await;
                let _ = state
                    .timeout_runtime_interaction(&timeout_session_id, &timeout_interaction_id)
                    .await;
            });
        }
        let resolution = receiver
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "relay_forward_native_interaction",
                message: format!("interaction dropped before resolution: {error}"),
            })?;
        Ok(crate::provider::ProviderNativeInteractionResolution {
            status: resolution.status.to_string(),
            choice_id: resolution.choice_id,
            reply: resolution.reply,
        })
    }

    pub(crate) async fn dispatch_authenticated_runtime_tool_call(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .dispatch_authenticated_runtime_tool_call(auth_token, tool_name, arguments)
            .await
    }

    pub(crate) async fn dispatch_forwarded_workflow_runtime_tool_call(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .dispatch_forwarded_workflow_runtime_tool_call(context, tool_name, arguments)
            .await
    }

    pub(crate) async fn dispatch_forwarded_workflow_provider_failure(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        message: String,
    ) -> Result<(), DaemonError> {
        self.runtime_state
            .dispatch_forwarded_workflow_provider_failure(context, message)
            .await
    }

    pub(crate) async fn dispatch_forwarded_managed_io_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteManagedIoContext,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
        ),
        DaemonError,
    > {
        self.runtime_state
            .dispatch_forwarded_managed_io_runtime_tool_call(
                context,
                tool_name,
                arguments,
                artifact_states,
            )
            .await
    }

    pub(crate) async fn dispatch_forwarded_capability_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteManagedIoContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::ArrobaSkillPackage>,
        ),
        DaemonError,
    > {
        self.runtime_state
            .dispatch_forwarded_capability_runtime_tool_call(context, tool_name, arguments)
            .await
    }

    pub(crate) async fn dispatch(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let focus_refresh = focus_projection_refresh(&request);
        let caller_user_id = self
            .authorize_session_membership(&command, &request)
            .await?;
        if let LocalDaemonRequest::GetSessionState(request) = &request {
            if !self
                .has_unsettled_pending_provider_launch(&request.session_id)
                .await
            {
                if let Some(session) = self.session_projection.get(&request.session_id) {
                    if !session.has_member(&caller_user_id) {
                        return Err(DaemonError::SessionAccessDenied {
                            session_id: session.id().to_string(),
                            user_id: caller_user_id.clone(),
                        });
                    }
                    return Ok(LocalDaemonResponse::SessionState {
                        session: session.redacted_for_user(&caller_user_id),
                    });
                }
                if self.session_projection.has_warmed_list() {
                    return Err(DaemonError::SessionNotFound {
                        session_id: request.session_id.clone(),
                    });
                }
            }
        }
        if let LocalDaemonRequest::ResolveSession(request) = &request {
            if let Some(session) = self
                .session_projection
                .resolve_session_ref(&request.session_ref, request.workspace_id.as_deref())
            {
                if !session.has_member(&caller_user_id) {
                    return Err(DaemonError::SessionAccessDenied {
                        session_id: session.id().to_string(),
                        user_id: caller_user_id.clone(),
                    });
                }
                return Ok(LocalDaemonResponse::SessionResolved {
                    session: session.redacted_for_user(&caller_user_id),
                });
            }
            if let Some(result) = self
                .session_projection
                .resolve_session_ref_id_from_warmed_list(
                    &request.session_ref,
                    request.workspace_id.as_deref(),
                )
            {
                let session_id = result?;
                let session = self.session_projection.get(&session_id).ok_or_else(|| {
                    DaemonError::SessionNotFound {
                        session_id: session_id.clone(),
                    }
                })?;
                if !session.has_member(&caller_user_id) {
                    return Err(DaemonError::SessionAccessDenied {
                        session_id: session.id().to_string(),
                        user_id: caller_user_id.clone(),
                    });
                }
                return Ok(LocalDaemonResponse::SessionResolved {
                    session: session.redacted_for_user(&caller_user_id),
                });
            }
        }
        if matches!(request, LocalDaemonRequest::ListSessions(_)) {
            if let Some(sessions) = self.session_projection.list() {
                let sessions = sessions
                    .into_iter()
                    .filter(|session| session.has_member(&caller_user_id))
                    .map(|session| session.redacted_for_user(&caller_user_id))
                    .collect();
                return Ok(LocalDaemonResponse::SessionsListed { sessions });
            }
            let sessions: Vec<_> = {
                let app = self.app.lock().await;
                app.sessions()
                    .list_sessions()
                    .into_iter()
                    .filter(|session| session.has_member(&caller_user_id))
                    .collect()
            };
            self.session_projection.update_list(sessions.clone());
            return Ok(LocalDaemonResponse::SessionsListed {
                sessions: sessions
                    .into_iter()
                    .map(|session| session.redacted_for_user(&caller_user_id))
                    .collect(),
            });
        }
        match &request {
            LocalDaemonRequest::RelayStatus(_) => {
                return self.projected_relay_status_response().await;
            }
            LocalDaemonRequest::ListRemoteMachines(_) => {
                return self.projected_remote_machines_response().await;
            }
            LocalDaemonRequest::ListRemoteMachineKernels(request) => {
                return self
                    .projected_remote_machine_kernels_response(request.machine_ref.clone())
                    .await;
            }
            LocalDaemonRequest::SearchWorkspaceDirectories(request) => {
                return self
                    .execute_search_workspace_directories_request(request.clone())
                    .await;
            }
            LocalDaemonRequest::CreateWorkspaceDirectory(request) => {
                return self
                    .execute_create_workspace_directory_request(request.clone())
                    .await;
            }
            LocalDaemonRequest::ListWorkspaceWorktrees(request) => {
                return self
                    .execute_list_workspace_worktrees_request(request.clone())
                    .await;
            }
            LocalDaemonRequest::CreateWorkspaceWorktree(request) => {
                return self
                    .execute_create_workspace_worktree_request(request.clone())
                    .await;
            }
            LocalDaemonRequest::GetProviderCommandCatalogs(_) => {
                return provider_command_catalogs_response();
            }
            LocalDaemonRequest::InstallMcpServer(request) => {
                return self
                    .execute_install_mcp_server_request(request.clone())
                    .await;
            }
            LocalDaemonRequest::UpdateMcpServer(request) => {
                return self
                    .execute_update_mcp_server_request(request.clone())
                    .await;
            }
            LocalDaemonRequest::UninstallMcpServer(request) => {
                return self
                    .execute_uninstall_mcp_server_request(request.clone())
                    .await;
            }
            LocalDaemonRequest::ImportMcpServers(request) => {
                return self
                    .execute_import_mcp_servers_request(request.clone())
                    .await;
            }
            LocalDaemonRequest::GetMcpServer(request) => {
                return self.execute_get_mcp_server_request(request.clone()).await;
            }
            LocalDaemonRequest::ListMcpServers(request) => {
                return self.execute_list_mcp_servers_request(request.clone()).await;
            }
            LocalDaemonRequest::InstallSkill(request) => {
                return self.execute_install_skill_request(request.clone()).await;
            }
            LocalDaemonRequest::UpdateSkill(request) => {
                return self.execute_update_skill_request(request.clone()).await;
            }
            LocalDaemonRequest::UninstallSkill(request) => {
                return self.execute_uninstall_skill_request(request.clone()).await;
            }
            LocalDaemonRequest::ImportSkills(request) => {
                return self.execute_import_skills_request(request.clone()).await;
            }
            LocalDaemonRequest::GetSkill(request) => {
                return self.execute_get_skill_request(request.clone()).await;
            }
            LocalDaemonRequest::ListSkills(request) => {
                return self.execute_list_skills_request(request.clone()).await;
            }
            _ => {}
        }
        if let Some(response) =
            self.projected_session_inspection_response(&request, &caller_user_id)
        {
            return response;
        }
        if let LocalDaemonRequest::PumpTerminalOutput(request) = &request {
            if let Some(response) = self.projected_terminal_output_response(request) {
                return response;
            }
        }
        if let LocalDaemonRequest::GetSessionHistory(request) = &request {
            if let Some(response) = self.projected_session_history_response(request).await {
                return response;
            }
        }
        if let LocalDaemonRequest::CompletePrompt(request) = &request {
            return self
                .agent_runtime
                .dispatch_prompt_complete(&command, request.clone())
                .await;
        }
        if is_workflow_command(&request) {
            return self
                .workflow_runtime
                .dispatch_workflow_command(command, request)
                .await;
        }
        if let LocalDaemonRequest::GetProviderRun(request) = &request {
            if let Some(provider_run) = self.provider_run_projection.get(&request.provider_run_id) {
                ensure_provider_run_visible_to_user(&provider_run, &caller_user_id)?;
                if provider_run.adapter_key() != "opencode" {
                    return Ok(LocalDaemonResponse::ProviderRun { provider_run });
                }
            }
        }
        if let LocalDaemonRequest::ListProviderProcesses(request) = &request {
            if let Some(processes) = self
                .provider_process_projection
                .list(request.provider.as_deref())
            {
                return Ok(LocalDaemonResponse::ProviderProcessesListed {
                    processes: self.provider_processes_visible_to_user(processes, &caller_user_id),
                });
            }
        }
        if matches!(request, LocalDaemonRequest::GetProviderCatalog(_)) {
            return self.projected_provider_catalog_response().await;
        }
        if matches!(request, LocalDaemonRequest::GetDaemonHealth(_)) {
            return Ok(LocalDaemonResponse::DaemonHealth {
                projection: self.daemon_health_projection(0).await,
            });
        }

        let session_refresh = session_projection_refresh(&request);
        let result = match request {
            LocalDaemonRequest::ConfigureRelay(request) => {
                self.execute_configure_relay_request(request).await
            }
            LocalDaemonRequest::CloudRelayStatus(_) => {
                self.execute_cloud_relay_status_request().await
            }
            LocalDaemonRequest::StartCloudRelayLogin(request) => {
                self.execute_start_cloud_relay_login_request(request).await
            }
            LocalDaemonRequest::PollCloudRelayLogin(request) => {
                self.execute_poll_cloud_relay_login_request(request).await
            }
            LocalDaemonRequest::LogoutCloudRelay(request) => {
                self.execute_logout_cloud_relay_request(request).await
            }
            LocalDaemonRequest::PairCloudRelayClient(request) => {
                self.execute_pair_cloud_relay_client_request(request).await
            }
            LocalDaemonRequest::PairCloudRelayMachine(request) => {
                self.execute_pair_cloud_relay_machine_request(request).await
            }
            LocalDaemonRequest::ConnectCloudRelay(request) => {
                self.execute_connect_cloud_relay_request(request).await
            }
            LocalDaemonRequest::IssueCloudRelayClientToken(request) => {
                self.execute_issue_cloud_relay_client_token_request(request)
                    .await
            }
            LocalDaemonRequest::CreateCloudSessionInvite(request) => {
                self.execute_create_cloud_session_invite_request(request)
                    .await
            }
            LocalDaemonRequest::ShowCloudSessionInvite(request) => {
                self.execute_show_cloud_session_invite_request(request)
                    .await
            }
            LocalDaemonRequest::AcceptCloudSessionInvite(request) => {
                self.execute_accept_cloud_session_invite_request(request)
                    .await
            }
            LocalDaemonRequest::RevokeCloudSessionInvite(request) => {
                self.execute_revoke_cloud_session_invite_request(request)
                    .await
            }
            LocalDaemonRequest::ListCloudSessionMembers(request) => {
                self.execute_list_cloud_session_members_request(request)
                    .await
            }
            LocalDaemonRequest::ListCloudCollaborators(request) => {
                self.execute_list_cloud_collaborators_request(request).await
            }
            LocalDaemonRequest::GetUserConfig(request) => {
                self.execute_get_user_config_request(request).await
            }
            LocalDaemonRequest::GetUserConfigSchema(request) => {
                self.execute_get_user_config_schema_request(request).await
            }
            LocalDaemonRequest::SetUserConfigValue(request) => {
                self.execute_set_user_config_value_request(request).await
            }
            LocalDaemonRequest::UnsetUserConfigValue(request) => {
                self.execute_unset_user_config_value_request(request).await
            }
            LocalDaemonRequest::SetCredentialSecret(request) => {
                self.execute_set_credential_secret_request(request).await
            }
            LocalDaemonRequest::DeleteCredentialSecret(request) => {
                self.execute_delete_credential_secret_request(request).await
            }
            LocalDaemonRequest::DeleteKernel(request) => {
                self.execute_delete_kernel_request(request).await
            }
            LocalDaemonRequest::ApproveRemoteMachine(request) => {
                self.execute_approve_remote_machine_request(request).await
            }
            LocalDaemonRequest::ForgetRemoteMachine(request) => {
                self.execute_forget_remote_machine_request(request).await
            }
            LocalDaemonRequest::RenameRemoteMachine(request) => {
                self.execute_rename_remote_machine_request(request).await
            }
            LocalDaemonRequest::ListSessionMembers(request) => {
                self.execute_list_session_members_request(request).await
            }
            LocalDaemonRequest::CreateSessionInvite(request) => {
                self.execute_create_session_invite_request(&command, request)
                    .await
            }
            LocalDaemonRequest::JoinSessionInvite(request) => {
                self.execute_join_session_invite_request(request).await
            }
            LocalDaemonRequest::RevokeSessionInvite(request) => {
                self.execute_revoke_session_invite_request(request).await
            }
            LocalDaemonRequest::CreateWorkspaceLink(request) => {
                self.execute_create_workspace_link_request(&command, request)
                    .await
            }
            LocalDaemonRequest::ListWorkspaceLinks(request) => {
                self.execute_list_workspace_links_request(request).await
            }
            LocalDaemonRequest::ShowWorkspaceLink(request) => {
                self.execute_show_workspace_link_request(request).await
            }
            LocalDaemonRequest::AttachWorkspaceLink(request) => {
                self.execute_attach_workspace_link_request(&command, request)
                    .await
            }
            LocalDaemonRequest::DetachWorkspaceLink(request) => {
                self.execute_detach_workspace_link_request(&command, request)
                    .await
            }
            LocalDaemonRequest::CreatePairingInvite(request) => {
                self.execute_create_pairing_invite_request(request).await
            }
            LocalDaemonRequest::JoinPairingInvite(request) => {
                self.execute_join_pairing_invite_request(request).await
            }
            LocalDaemonRequest::CreateTerminalPairingLink(request) => {
                self.execute_create_terminal_pairing_link_request(request)
                    .await
            }
            LocalDaemonRequest::JoinTerminalPairingLink(request) => {
                self.execute_join_terminal_pairing_link_request(request)
                    .await
            }
            LocalDaemonRequest::ListTerminals(_) => self.execute_list_terminals_request().await,
            LocalDaemonRequest::ListPairedClients(_) => {
                self.execute_list_paired_clients_request().await
            }
            LocalDaemonRequest::RecordPairedClient(request) => {
                self.execute_record_paired_client_request(request).await
            }
            LocalDaemonRequest::RevokePairedClient(request) => {
                self.execute_revoke_paired_client_request(request).await
            }
            LocalDaemonRequest::GetSessionHistory(request) => {
                self.execute_session_history_request(request).await
            }
            LocalDaemonRequest::GetPromptInputHistory(request) => {
                self.execute_prompt_input_history_request(request).await
            }
            LocalDaemonRequest::RecordPromptInputHistory(request) => {
                self.execute_record_prompt_input_history_request(request)
                    .await
            }
            LocalDaemonRequest::QueryHistory(request) => {
                self.execute_query_history_request(history_query_from_request(request))
                    .await
            }
            LocalDaemonRequest::SearchHistory(request) => {
                self.execute_query_history_request(history_query_from_search_request(request))
                    .await
            }
            LocalDaemonRequest::PumpTerminalOutput(request) => {
                self.execute_terminal_output_request(request).await
            }
            LocalDaemonRequest::TeardownProviderProcesses(request) => {
                self.execute_teardown_provider_processes_request(request)
                    .await
            }
            request => match command.priority {
                KernelCommandPriority::Interactive => {
                    self.dispatch_interactive(command, request).await
                }
                KernelCommandPriority::Normal | KernelCommandPriority::Background => {
                    self.dispatch_normal_or_background(command, request).await
                }
            },
        };
        self.apply_session_projection_refresh(session_refresh, &result)
            .await;
        self.apply_focus_projection_refresh(focus_refresh, &result)
            .await;
        self.apply_provider_run_projection_refresh(&result).await;
        self.apply_provider_launch_projection_state(&result).await;
        self.apply_agent_lane_cleanup(&result).await;
        self.redact_result_for_user(result, &caller_user_id)
    }

    fn redact_result_for_user(
        &self,
        result: Result<LocalDaemonResponse, DaemonError>,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        result.and_then(|response| self.redact_response_for_user(response, caller_user_id))
    }

    fn redact_response_for_user(
        &self,
        response: LocalDaemonResponse,
        caller_user_id: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(match response {
            LocalDaemonResponse::SessionCreated { session, agent } => {
                LocalDaemonResponse::SessionCreated {
                    session: session.redacted_for_user(caller_user_id),
                    agent,
                }
            }
            LocalDaemonResponse::SessionResolved { session } => {
                LocalDaemonResponse::SessionResolved {
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::SessionState { session } => LocalDaemonResponse::SessionState {
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::SessionsListed { sessions } => {
                LocalDaemonResponse::SessionsListed {
                    sessions: sessions
                        .into_iter()
                        .map(|session| session.redacted_for_user(caller_user_id))
                        .collect(),
                }
            }
            LocalDaemonResponse::SessionInviteCreated { invite, session } => {
                LocalDaemonResponse::SessionInviteCreated {
                    invite,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::SessionInviteJoined { member, session } => {
                LocalDaemonResponse::SessionInviteJoined {
                    member,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::SessionInviteRevoked { invite, session } => {
                LocalDaemonResponse::SessionInviteRevoked {
                    invite,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::WorkspaceLinkCreated { link, session } => {
                LocalDaemonResponse::WorkspaceLinkCreated {
                    link,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::WorkspaceLinkAttached {
                link,
                attachment,
                session,
            } => LocalDaemonResponse::WorkspaceLinkAttached {
                link,
                attachment,
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkspaceLinkDetached {
                link,
                detached,
                session,
            } => LocalDaemonResponse::WorkspaceLinkDetached {
                link,
                detached,
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::PromptSubmitted { outcome, session } => {
                LocalDaemonResponse::PromptSubmitted {
                    outcome,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::SessionConfigUpdated { config, session } => {
                LocalDaemonResponse::SessionConfigUpdated {
                    config,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::AgentConfigUpdated { agent, session } => {
                LocalDaemonResponse::AgentConfigUpdated {
                    agent,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::AgentProfileUpdated { agent, session } => {
                LocalDaemonResponse::AgentProfileUpdated {
                    agent,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::AgentAliased { agent, session } => {
                LocalDaemonResponse::AgentAliased {
                    agent,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::SessionEnded { session } => LocalDaemonResponse::SessionEnded {
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::SessionDeleted { session } => {
                LocalDaemonResponse::SessionDeleted {
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::SessionAliased { session } => {
                LocalDaemonResponse::SessionAliased {
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::AgentsListed { agents } => LocalDaemonResponse::AgentsListed {
                agents: agents
                    .into_iter()
                    .filter(|agent| agent.owner_user_id() == caller_user_id)
                    .collect(),
            },
            LocalDaemonResponse::ProviderRun { provider_run } => {
                ensure_provider_run_visible_to_user(&provider_run, caller_user_id)?;
                LocalDaemonResponse::ProviderRun { provider_run }
            }
            LocalDaemonResponse::ProviderProcessesListed { processes } => {
                LocalDaemonResponse::ProviderProcessesListed {
                    processes: self.provider_processes_visible_to_user(processes, caller_user_id),
                }
            }
            LocalDaemonResponse::ProviderProcessesTornDown { processes } => {
                LocalDaemonResponse::ProviderProcessesTornDown {
                    processes: self.provider_processes_visible_to_user(processes, caller_user_id),
                }
            }
            LocalDaemonResponse::WorkflowCreated { workflow, session } => {
                LocalDaemonResponse::WorkflowCreated {
                    workflow: workflow.redacted_for_user(caller_user_id),
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::WorkflowAliased { workflow, session } => {
                LocalDaemonResponse::WorkflowAliased {
                    workflow: workflow.redacted_for_user(caller_user_id),
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::WorkflowsListed { workflows } => {
                LocalDaemonResponse::WorkflowsListed {
                    workflows: workflows
                        .into_iter()
                        .map(|workflow| workflow.redacted_for_user(caller_user_id))
                        .collect(),
                }
            }
            LocalDaemonResponse::WorkflowResolved { workflow } => {
                LocalDaemonResponse::WorkflowResolved {
                    workflow: workflow.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::WorkflowEndpointCreated {
                endpoint,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowEndpointCreated {
                endpoint,
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowEndpointAliased {
                endpoint,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowEndpointAliased {
                endpoint,
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowEndpointBound {
                endpoint,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowEndpointBound {
                endpoint,
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowNodeAdded {
                node,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowNodeAdded {
                node: node.redacted_for_user(caller_user_id),
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowNodeRemoved {
                node,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowNodeRemoved {
                node: node.redacted_for_user(caller_user_id),
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
                node,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
                node: node.redacted_for_user(caller_user_id),
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
                node,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
                node: node.redacted_for_user(caller_user_id),
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                node,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                node: node.redacted_for_user(caller_user_id),
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                node,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                node: node.redacted_for_user(caller_user_id),
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
                node,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
                node: node.redacted_for_user(caller_user_id),
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowEdgeAdded {
                edge,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowEdgeAdded {
                edge,
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowEdgeRemoved {
                edge,
                workflow,
                session,
            } => LocalDaemonResponse::WorkflowEdgeRemoved {
                edge,
                workflow: workflow.redacted_for_user(caller_user_id),
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowRunInvoked {
                workflow_run,
                workflow,
                endpoint,
                session,
            } => LocalDaemonResponse::WorkflowRunInvoked {
                workflow_run: workflow_run.redacted_for_user(Some(&workflow), caller_user_id),
                workflow: workflow.redacted_for_user(caller_user_id),
                endpoint,
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowRunQueued {
                queued_launch,
                workflow,
                endpoint,
                session,
            } => LocalDaemonResponse::WorkflowRunQueued {
                queued_launch,
                workflow: workflow.redacted_for_user(caller_user_id),
                endpoint,
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => {
                LocalDaemonResponse::WorkflowRunsListed {
                    workflow_runs: workflow_runs
                        .into_iter()
                        .map(|workflow_run| workflow_run.redacted_for_user(None, caller_user_id))
                        .collect(),
                }
            }
            LocalDaemonResponse::WorkflowRun { workflow_run } => LocalDaemonResponse::WorkflowRun {
                workflow_run: workflow_run.redacted_for_user(None, caller_user_id),
            },
            LocalDaemonResponse::WorkflowRunCancelled {
                workflow_run,
                session,
            } => {
                let redacted_run = {
                    let workflow = session
                        .workflows()
                        .iter()
                        .find(|workflow| workflow.id() == workflow_run.workflow_id());
                    workflow_run.redacted_for_user(workflow, caller_user_id)
                };
                LocalDaemonResponse::WorkflowRunCancelled {
                    workflow_run: redacted_run,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::WorkflowRunResumed {
                workflow_run,
                session,
            } => {
                let redacted_run = {
                    let workflow = session
                        .workflows()
                        .iter()
                        .find(|workflow| workflow.id() == workflow_run.workflow_id());
                    workflow_run.redacted_for_user(workflow, caller_user_id)
                };
                LocalDaemonResponse::WorkflowRunResumed {
                    workflow_run: redacted_run,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session } => {
                LocalDaemonResponse::WorkflowFlushContextUpdated {
                    workflow: workflow.redacted_for_user(caller_user_id),
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session } => {
                LocalDaemonResponse::WorkflowRunOutputSchemaUpdated {
                    workflow: workflow.redacted_for_user(caller_user_id),
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { workflow, session } => {
                LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated {
                    workflow: workflow.redacted_for_user(caller_user_id),
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session } => {
                LocalDaemonResponse::WorkflowLaunchPolicyUpdated {
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
                queued_launch,
                session,
            } => LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
                queued_launch,
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
                queued_launches,
                session,
            } => LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
                queued_launches,
                session: session.redacted_for_user(caller_user_id),
            },
            LocalDaemonResponse::WorkflowTurnAcknowledged {
                workflow_run,
                session,
            } => {
                let redacted_run = {
                    let workflow = session
                        .workflows()
                        .iter()
                        .find(|workflow| workflow.id() == workflow_run.workflow_id());
                    workflow_run.redacted_for_user(workflow, caller_user_id)
                };
                LocalDaemonResponse::WorkflowTurnAcknowledged {
                    workflow_run: redacted_run,
                    session: session.redacted_for_user(caller_user_id),
                }
            }
            other => other,
        })
    }

    fn provider_processes_visible_to_user(
        &self,
        processes: Vec<crate::provider::ProviderProcessInfo>,
        caller_user_id: &str,
    ) -> Vec<crate::provider::ProviderProcessInfo> {
        processes
            .into_iter()
            .filter(|process| {
                process.owner_provider_run_ids.iter().any(|run_id| {
                    self.provider_run_projection
                        .get(run_id)
                        .is_some_and(|run| run.owned_by(caller_user_id))
                })
            })
            .collect()
    }

    fn projected_session_inspection_response(
        &self,
        request: &LocalDaemonRequest,
        caller_user_id: &str,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        match request {
            LocalDaemonRequest::ListAgents(request) => {
                let session = match self.projected_session_or_absence(&request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
                Some(Ok(LocalDaemonResponse::AgentsListed {
                    agents: session
                        .agents()
                        .iter()
                        .filter(|agent| agent.owner_user_id() == caller_user_id)
                        .cloned()
                        .collect(),
                }))
            }
            LocalDaemonRequest::ListWorkflows(request) => {
                let session = match self.projected_session_or_absence(&request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
                Some(Ok(LocalDaemonResponse::WorkflowsListed {
                    workflows: session
                        .workflows()
                        .iter()
                        .cloned()
                        .map(|workflow| workflow.redacted_for_user(caller_user_id))
                        .collect(),
                }))
            }
            LocalDaemonRequest::ResolveWorkflow(request) => {
                let session = match self.projected_session_or_absence(&request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
                Some(
                    projected_resolve_workflow(&session, &request.workflow_ref).map(|workflow| {
                        LocalDaemonResponse::WorkflowResolved {
                            workflow: workflow.redacted_for_user(caller_user_id),
                        }
                    }),
                )
            }
            LocalDaemonRequest::ListWorkflowRuns(request) => {
                let session = match self.projected_session_or_absence(&request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
                Some(
                    projected_workflow_id(&session, request.workflow_ref.as_deref()).map(
                        |workflow_id| {
                            let workflow_runs = session
                                .workflow_runs()
                                .iter()
                                .filter(|workflow_run| {
                                    workflow_id
                                        .as_deref()
                                        .is_none_or(|id| workflow_run.workflow_id() == id)
                                })
                                .cloned()
                                .map(|workflow_run| {
                                    let workflow = workflow_id.as_deref().and_then(|id| {
                                        session
                                            .workflows()
                                            .iter()
                                            .find(|workflow| workflow.id() == id)
                                    });
                                    workflow_run.redacted_for_user(workflow, caller_user_id)
                                })
                                .collect();
                            LocalDaemonResponse::WorkflowRunsListed { workflow_runs }
                        },
                    ),
                )
            }
            LocalDaemonRequest::GetWorkflowRun(request) => {
                let session = match self.projected_session_or_absence(&request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
                Some(
                    projected_resolve_workflow_run(&session, &request.workflow_run_ref).map(
                        |workflow_run| {
                            let workflow = session
                                .workflows()
                                .iter()
                                .find(|workflow| workflow.id() == workflow_run.workflow_id());
                            LocalDaemonResponse::WorkflowRun {
                                workflow_run: workflow_run
                                    .redacted_for_user(workflow, caller_user_id),
                            }
                        },
                    ),
                )
            }
            LocalDaemonRequest::ListWorkflowWatchdogs(request) => {
                let session = match self.projected_session_or_absence(&request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
                Some(
                    projected_workflow_id(&session, request.workflow_ref.as_deref()).map(
                        |workflow_id| {
                            let watchdogs = session
                                .workflow_watchdogs()
                                .iter()
                                .filter(|watchdog| {
                                    workflow_id
                                        .as_deref()
                                        .is_none_or(|id| watchdog.workflow_id() == id)
                                })
                                .cloned()
                                .collect();
                            LocalDaemonResponse::WorkflowWatchdogsListed { watchdogs }
                        },
                    ),
                )
            }
            LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => {
                let session = match self.projected_session_or_absence(&request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
                Some(Ok(LocalDaemonResponse::QueuedWorkflowLaunchesListed {
                    queued_launches: session.queued_workflow_launches().iter().cloned().collect(),
                }))
            }
            _ => None,
        }
    }

    fn projected_terminal_output_response(
        &self,
        request: &PumpTerminalOutputRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        let session = match self.projected_session_or_absence(&request.session_id)? {
            Ok(session) => session,
            Err(error) => return Some(Err(error)),
        };
        if !session.has_attachment(&request.attachment_id) {
            return Some(Err(DaemonError::AttachmentNotInSession {
                session_id: request.session_id.clone(),
                attachment_id: request.attachment_id.clone(),
            }));
        }
        let active_provider_run_id = session.active_provider_run_id();
        if active_provider_run_id.is_none()
            || active_provider_run_id.is_some_and(|provider_run_id| {
                self.provider_run_projection
                    .get(provider_run_id)
                    .is_some_and(|run| {
                        run.session_id() == request.session_id
                            && matches!(
                                run.state(),
                                ProviderRunState::Ended | ProviderRunState::Parked
                            )
                    })
            })
        {
            return Some(Ok(LocalDaemonResponse::TerminalOutput {
                records: self
                    .terminal_stream
                    .drain_output_records(&request.session_id, &request.attachment_id),
            }));
        }
        None
    }

    async fn projected_relay_status_response(&self) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::RelayStatus {
            status: self.projected_relay_status().await,
        })
    }

    async fn projected_waiting_room_inventory_response(
        &self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WaitingRoomInventory {
            snapshot: self.projected_waiting_room_public_snapshot().await?.into(),
        })
    }

    async fn projected_waiting_room_public_snapshot_response(
        &self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::WaitingRoomPublicSnapshot {
            snapshot: self.projected_waiting_room_public_snapshot().await?,
        })
    }

    async fn projected_waiting_room_public_snapshot(
        &self,
    ) -> Result<WaitingRoomPublicSnapshot, DaemonError> {
        self.request_remote_relay_inventory_projection_refresh()
            .await;
        let runtime_sessions = match self
            .execute_cold_list_sessions_request(ListSessionsRequest)
            .await?
        {
            LocalDaemonResponse::SessionsListed { sessions } => sessions,
            _response => {
                return Err(DaemonError::LocalTransport {
                    operation: "build waiting room inventory",
                    message: format!("list sessions produced unexpected response `{}`", "unknown"),
                });
            }
        };
        let sessions = waiting_room_session_summaries(runtime_sessions);
        let relay_status = self.projected_relay_status().await;
        let (remote_machines, remote_kernels) = self.remote_relay_inventory_projection.snapshot();
        let launch_target = infer_waiting_room_launch_target();
        let terminals = paired_terminal_records();
        let generated_at_ms = unix_epoch_ms();
        let inventory_version = waiting_room_inventory_version(
            &sessions,
            &relay_status,
            &remote_machines,
            &remote_kernels,
            &terminals,
            launch_target.as_ref(),
        )?;
        Ok(WaitingRoomPublicSnapshot {
            schema_version: 3,
            inventory_version,
            generated_at_ms,
            sessions,
            relay_status,
            remote_machines,
            remote_kernels,
            terminals,
            launch_target,
        })
    }

    pub(crate) async fn waiting_room_inventory_version(&self) -> Result<String, DaemonError> {
        match self.projected_waiting_room_inventory_response().await? {
            LocalDaemonResponse::WaitingRoomInventory { snapshot } => {
                Ok(snapshot.inventory_version)
            }
            _response => Err(DaemonError::LocalTransport {
                operation: "build waiting room inventory version",
                message: "waiting room inventory request produced unexpected response".to_string(),
            }),
        }
    }

    async fn projected_remote_machines_response(&self) -> Result<LocalDaemonResponse, DaemonError> {
        self.request_remote_relay_inventory_projection_refresh()
            .await;
        let (machines, _) = self.remote_relay_inventory_projection.snapshot();
        Ok(LocalDaemonResponse::RemoteMachinesListed { machines })
    }

    async fn projected_remote_machine_kernels_response(
        &self,
        machine_ref: String,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.request_remote_relay_inventory_projection_refresh()
            .await;
        let machine_ref =
            crate::local::provider_requests::resolve_registered_or_raw_machine_ref(&machine_ref);
        let (_, kernels) = self.remote_relay_inventory_projection.snapshot();
        let kernels = kernels
            .into_iter()
            .filter(|kernel| {
                kernel.machine_id == machine_ref
                    || kernel.machine_alias.as_deref() == Some(machine_ref.as_str())
                    || kernel.relay_alias.as_deref() == Some(machine_ref.as_str())
                    || kernel.kernel_alias.as_deref() == Some(machine_ref.as_str())
            })
            .collect();
        Ok(LocalDaemonResponse::RemoteMachineKernelsListed {
            machine_ref,
            kernels,
        })
    }

    async fn execute_search_workspace_directories_request(
        &self,
        request: SearchWorkspaceDirectoriesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let limit = request.limit.unwrap_or(12).clamp(1, 50);
        let directories = search_workspace_directories(&request.query, limit)?;
        Ok(LocalDaemonResponse::WorkspaceDirectoriesSearched { directories })
    }

    async fn execute_create_workspace_directory_request(
        &self,
        request: CreateWorkspaceDirectoryRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let directory = create_workspace_directory(&request.path)?;
        Ok(LocalDaemonResponse::WorkspaceDirectoryCreated { directory })
    }

    async fn execute_list_workspace_worktrees_request(
        &self,
        request: ListWorkspaceWorktreesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let launch_target = infer_waiting_room_launch_target();
        let worktrees = list_workspace_worktrees(
            &request.workspace_id,
            launch_target
                .as_ref()
                .map(|target| target.worktree_id.as_str()),
        )?;
        Ok(LocalDaemonResponse::WorkspaceWorktreesListed {
            workspace_id: request.workspace_id,
            worktrees,
        })
    }

    async fn execute_create_workspace_worktree_request(
        &self,
        request: CreateWorkspaceWorktreeRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let path = create_waiting_room_worktree(
            &request.workspace_id,
            request.path.as_deref(),
            request.branch.as_deref(),
            request.base_ref.as_deref(),
        )?;
        let launch_target = infer_waiting_room_launch_target();
        let branch = detect_git_branch(&path).ok();
        let worktree = WorkspaceWorktreeRecord {
            current: launch_target
                .as_ref()
                .map(|target| target.worktree_id == path)
                .unwrap_or(false),
            branch: branch.clone(),
            label: worktree_display_label(
                &path,
                launch_target
                    .as_ref()
                    .map(|target| target.workspace_id.as_str())
                    .unwrap_or(&request.workspace_id),
                branch.as_deref(),
            ),
            path,
        };
        Ok(LocalDaemonResponse::WorkspaceWorktreeCreated {
            workspace_id: request.workspace_id,
            worktree,
        })
    }

    async fn request_remote_relay_inventory_projection_refresh(&self) {
        let connected = self.relay_state.read().await.connected();
        if !connected {
            return;
        }
        let config = self.config_projection.snapshot();
        let now_ms = crate::session::unix_epoch_ms();
        let stale_after_ms = (config.relay_heartbeat_ms.saturating_mul(2)).max(1_000);
        let cooldown_ms = 1_000;
        if !self
            .remote_relay_inventory_projection
            .should_request_refresh(now_ms, stale_after_ms, cooldown_ms)
        {
            return;
        }
        let app = Arc::clone(&self.app);
        tokio::spawn(async move {
            if let Err(error) =
                refresh_remote_inventory_projection_for_app_with_relay_state(&app).await
            {
                crate::logging::warn_with_fields(
                    "daemon.router",
                    "remote relay inventory refresh on demand failed",
                    serde_json::json!({
                        "error": error.to_string(),
                        "stale_after_ms": stale_after_ms,
                        "cooldown_ms": cooldown_ms,
                    }),
                );
            }
        });
    }

    async fn projected_relay_status(&self) -> RelayStatus {
        let config = self.config_projection.snapshot();
        let connected = self.relay_state.read().await.connected();
        RelayStatus {
            configured: config.relay_url.is_some() && config.relay_token.is_some(),
            connected,
            relay_url: config.relay_url,
            relay_token_configured: config.relay_token.is_some(),
            daemon_id: config.daemon_id,
            machine_id: config.host_machine_id,
            machine_alias: config.host_machine_alias,
        }
    }

    async fn invalidate_provider_catalog_caches(&self) {
        self.provider_catalog_projection.invalidate();
        if let Ok(mut app) = self.app.try_lock() {
            app.invalidate_provider_catalog_cache();
        }
    }

    async fn execute_cold_list_sessions_request(
        &self,
        _request: ListSessionsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let app = self.app.lock().await;
        crate::app::KernelSessionReadService::new(&app).list_sessions_response()
    }

    async fn execute_cold_resolve_session_request(
        &self,
        request: ResolveSessionRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let app = self.app.lock().await;
        crate::app::KernelSessionReadService::new(&app).resolve_session_response(request)
    }

    async fn execute_cold_get_session_state_request(
        &self,
        request: GetSessionStateRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let app = self.app.lock().await;
        crate::app::KernelSessionReadService::new(&app).get_session_state_response(request)
    }

    async fn execute_cold_list_agents_request(
        &self,
        request: ListAgentsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let app = self.app.lock().await;
        crate::app::KernelSessionReadService::new(&app).list_agents_response(request)
    }

    async fn execute_get_provider_run_request(
        &self,
        request: GetProviderRunRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut app = self.app.lock().await;
        crate::local::provider_requests::get_provider_run_response(&mut app, request)
    }

    async fn execute_get_provider_auth_status_request(
        request: GetProviderAuthStatusRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        tokio::task::spawn_blocking(move || provider_auth_status_response(request))
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "get provider auth status",
                message: error.to_string(),
            })?
    }

    async fn execute_start_provider_login_request(
        request: StartProviderLoginRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        tokio::task::spawn_blocking(move || start_provider_login_response(request))
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "start provider login",
                message: error.to_string(),
            })?
    }

    async fn execute_logout_provider_request(
        &self,
        request: LogoutProviderRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let response = tokio::task::spawn_blocking(move || logout_provider_response(request))
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "logout provider",
                message: error.to_string(),
            })??;
        self.invalidate_provider_catalog_caches().await;
        Ok(response)
    }

    async fn execute_capability_request(
        &self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        execute_capability_request(
            &self.capability_runtime,
            self.capability_health.clone(),
            request,
        )
        .await
        .unwrap_or_else(|| {
            Err(DaemonError::LocalTransport {
                operation: "route capability request",
                message: "capability request was not handled by executor".to_string(),
            })
        })
    }

    async fn execute_configure_relay_request(
        &self,
        request: ConfigureRelayRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        {
            let mut app = self.app.lock().await;
            app.configure_relay(request.relay_url, request.relay_token)?;
            app.invalidate_provider_catalog_cache();
            self.config_projection.update(app.config().clone());
        }
        self.provider_catalog_projection.invalidate();
        Ok(LocalDaemonResponse::RelayConfigured {
            status: self.projected_relay_status().await,
        })
    }

    async fn execute_cloud_relay_status_request(&self) -> Result<LocalDaemonResponse, DaemonError> {
        let profile = self
            .config_projection
            .snapshot()
            .cloud_relay
            .as_ref()
            .map(cloud_profile_from_persisted);
        Ok(LocalDaemonResponse::CloudRelayStatus { profile })
    }

    async fn execute_start_cloud_relay_login_request(
        &self,
        request: StartCloudRelayLoginRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let api_url = normalize_cloud_api_url(&request.api_url)?;
        let response: CloudDeviceStartResponse = post_cloud_json(
            api_url.clone(),
            "/auth/device/start",
            serde_json::json!({
                "clientId": request.client_id,
                "clientAlias": request.client_alias,
                "machineId": request.machine_id,
                "machineAlias": request.machine_alias,
            }),
        )
        .await?;
        Ok(LocalDaemonResponse::CloudRelayLoginStarted {
            login: CloudRelayLoginStart {
                api_url,
                device_code: response.device_code,
                user_code: response.user_code,
                verification_url: response.verification_url,
                expires_at: response.expires_at,
                interval_seconds: response.interval_seconds,
            },
        })
    }

    async fn execute_poll_cloud_relay_login_request(
        &self,
        request: PollCloudRelayLoginRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let api_url = normalize_cloud_api_url(&request.api_url)?;
        let response: CloudDevicePollResponse = post_cloud_json(
            api_url.clone(),
            "/auth/device/poll",
            serde_json::json!({ "deviceCode": request.device_code }),
        )
        .await?;
        let result = match response.status.as_str() {
            "authorization_pending" => CloudRelayLoginPoll {
                status: CloudRelayLoginPollStatus::AuthorizationPending,
                interval_seconds: response.interval_seconds,
                expires_at: response.expires_at,
                profile: None,
            },
            "expired_token" => CloudRelayLoginPoll {
                status: CloudRelayLoginPollStatus::ExpiredToken,
                interval_seconds: None,
                expires_at: None,
                profile: None,
            },
            "approved" => {
                let profile = response
                    .profile
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "poll cloud relay login",
                        message: "cloud approval response did not include a profile".to_string(),
                    })?;
                let session_token =
                    response
                        .cloud_session_token
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "poll cloud relay login",
                            message: "cloud approval response did not include a session token"
                                .to_string(),
                        })?;
                let persisted = PersistedCloudRelayProfile {
                    api_url,
                    email: profile.email,
                    account_id: profile.account_id,
                    user_id: profile.user_id,
                    account_slug: profile.account_slug,
                    realm_id: profile.realm_id,
                    relay_url: profile.relay_url,
                    issuer_id: profile.issuer_id,
                    client_id: profile.client_id,
                    client_alias: profile.client_alias,
                    machine_id: profile.machine_id,
                    machine_alias: profile.machine_alias,
                    machine_credential: response.machine_credential,
                    cloud_session_token: Some(session_token),
                    cloud_session_expires_at_ms: None,
                    token_expires_at_ms: None,
                };
                {
                    let mut app = self.app.lock().await;
                    app.persist_cloud_relay_profile(Some(persisted.clone()))?;
                }
                self.config_projection.update({
                    let app = self.app.lock().await;
                    app.config().clone()
                });
                CloudRelayLoginPoll {
                    status: CloudRelayLoginPollStatus::Approved,
                    interval_seconds: None,
                    expires_at: response.cloud_session_expires_at,
                    profile: Some(cloud_profile_from_persisted(&persisted)),
                }
            }
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "poll cloud relay login",
                    message: format!("cloud returned unknown device login status `{other}`"),
                });
            }
        };
        Ok(LocalDaemonResponse::CloudRelayLoginPolled { result })
    }

    async fn execute_logout_cloud_relay_request(
        &self,
        request: LogoutCloudRelayRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let profile = self.config_projection.snapshot().cloud_relay;
        if let Some(profile) = profile.as_ref() {
            let _ = post_cloud_json::<serde_json::Value>(
                profile.api_url.clone(),
                "/auth/logout",
                serde_json::json!({
                    "sessionToken": profile.cloud_session_token,
                    "accountId": profile.account_id,
                    "clientId": profile.client_id,
                    "machineId": profile.machine_id,
                    "revokeClient": request.revoke_client,
                    "revokeMachine": request.revoke_machine,
                }),
            )
            .await;
        }
        {
            let mut app = self.app.lock().await;
            app.persist_cloud_relay_profile(None)?;
        }
        self.config_projection.update({
            let app = self.app.lock().await;
            app.config().clone()
        });
        Ok(LocalDaemonResponse::CloudRelayLoggedOut)
    }

    async fn execute_pair_cloud_relay_client_request(
        &self,
        request: PairCloudRelayClientRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut profile = self.required_cloud_relay_profile()?;
        let pairing: CloudPairingTokenResponse = post_cloud_json(
            profile.api_url.clone(),
            "/pairing-tokens",
            serde_json::json!({
                "accountId": profile.account_id,
                "createdByUserId": profile.user_id,
                "subjectKind": "client",
            }),
        )
        .await?;
        post_cloud_json::<serde_json::Value>(
            profile.api_url.clone(),
            "/clients/pair",
            serde_json::json!({
                "accountId": profile.account_id,
                "token": pairing.token,
                "clientId": request.client_id,
                "userId": profile.user_id,
                "alias": request.alias,
            }),
        )
        .await?;
        profile.client_id = Some(request.client_id);
        if request.alias.is_some() {
            profile.client_alias = request.alias;
        }
        let saved = self.persist_cloud_profile(profile).await?;
        Ok(LocalDaemonResponse::CloudRelayClientPaired {
            profile: cloud_profile_from_persisted(&saved),
        })
    }

    async fn execute_pair_cloud_relay_machine_request(
        &self,
        request: PairCloudRelayMachineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut profile = self.required_cloud_relay_profile()?;
        let pairing: CloudPairingTokenResponse = post_cloud_json(
            profile.api_url.clone(),
            "/pairing-tokens",
            serde_json::json!({
                "accountId": profile.account_id,
                "createdByUserId": profile.user_id,
                "subjectKind": "machine",
            }),
        )
        .await?;
        post_cloud_json::<serde_json::Value>(
            profile.api_url.clone(),
            "/machines/pair",
            serde_json::json!({
                "accountId": profile.account_id,
                "token": pairing.token,
                "machineId": request.machine_id,
                "userId": profile.user_id,
                "alias": request.alias,
                "runtimeProfile": self.machine_runtime_profile_payload().await,
            }),
        )
        .await?;
        profile.machine_id = Some(request.machine_id);
        if request.alias.is_some() {
            profile.machine_alias = request.alias;
        }
        let saved = self.persist_cloud_profile(profile).await?;
        Ok(LocalDaemonResponse::CloudRelayMachinePaired {
            profile: cloud_profile_from_persisted(&saved),
        })
    }

    async fn execute_connect_cloud_relay_request(
        &self,
        _request: ConnectCloudRelayRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut profile = self.required_cloud_relay_profile()?;
        let daemon_id = self.config_projection.snapshot().daemon_id;
        let (subject, subject_kind, machine_id) =
            if let Some(machine_id) = profile.machine_id.clone() {
                (machine_id.clone(), "machine", Some(machine_id))
            } else {
                (daemon_id, "kernel", None)
            };
        let issued = issue_cloud_runtime_token(
            &profile,
            &subject,
            subject_kind,
            None,
            None,
            machine_id,
            None,
        )
        .await?;
        profile.token_expires_at_ms =
            Some(crate::session::unix_epoch_ms() + CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS);
        let saved = self.persist_cloud_profile(profile.clone()).await?;
        {
            let mut app = self.app.lock().await;
            app.configure_relay(Some(profile.relay_url.clone()), Some(issued.token.clone()))?;
            app.invalidate_provider_catalog_cache();
            self.config_projection.update(app.config().clone());
        }
        self.provider_catalog_projection.invalidate();
        let token = CloudRelayRuntimeToken {
            relay_url: profile.relay_url,
            relay_token: issued.token,
            token_expires_at: issued.expires_at,
        };
        Ok(LocalDaemonResponse::CloudRelayConnected {
            status: self.projected_relay_status().await,
            profile: cloud_profile_from_persisted(&saved),
            token,
        })
    }

    async fn execute_issue_cloud_relay_client_token_request(
        &self,
        request: IssueCloudRelayClientTokenRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let mut profile = self.required_cloud_relay_profile()?;
        if profile.client_id.is_none() {
            let pairing: CloudPairingTokenResponse = match post_cloud_json(
                profile.api_url.clone(),
                "/pairing-tokens",
                serde_json::json!({
                    "accountId": profile.account_id,
                    "createdByUserId": profile.user_id,
                    "subjectKind": "client",
                }),
            )
            .await
            {
                Ok(pairing) => pairing,
                Err(error) => {
                    self.clear_cloud_profile_if_stale(&error).await?;
                    return Err(error);
                }
            };
            if let Err(error) = post_cloud_json::<serde_json::Value>(
                profile.api_url.clone(),
                "/clients/pair",
                serde_json::json!({
                    "accountId": profile.account_id,
                    "token": pairing.token,
                    "clientId": request.client_id,
                    "userId": profile.user_id,
                }),
            )
            .await
            {
                self.clear_cloud_profile_if_stale(&error).await?;
                return Err(error);
            }
            profile.client_id = Some(request.client_id.clone());
            profile = self.persist_cloud_profile(profile).await?;
        }
        let client_id = profile
            .client_id
            .clone()
            .unwrap_or_else(|| request.client_id.clone());
        let issued = match issue_cloud_runtime_token(
            &profile,
            &client_id,
            "client",
            Some(vec![request.target_daemon_alias]),
            Some(client_id.clone()),
            profile
                .machine_credential
                .as_ref()
                .and(profile.machine_id.clone()),
            request.session_id,
        )
        .await
        {
            Ok(issued) => issued,
            Err(error) => {
                self.clear_cloud_profile_if_stale(&error).await?;
                return Err(error);
            }
        };
        let token = CloudRelayRuntimeToken {
            relay_url: profile.relay_url.clone(),
            relay_token: issued.token,
            token_expires_at: issued.expires_at,
        };
        Ok(LocalDaemonResponse::CloudRelayClientTokenIssued {
            profile: cloud_profile_from_persisted(&profile),
            token,
        })
    }

    async fn execute_create_cloud_session_invite_request(
        &self,
        request: CreateCloudSessionInviteRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let profile = self.required_cloud_relay_profile_with_session()?;
        let invite: CloudSessionInviteResponse = match post_cloud_json(
            profile.api_url.clone(),
            "/sessions/invites",
            serde_json::json!({
                "sessionToken": profile.cloud_session_token,
                "accountId": profile.account_id,
                "sessionId": request.session_id,
                "displayName": request.display_name,
                "expiresInMs": request.expires_in_ms,
                "maxUses": request.max_uses,
            }),
        )
        .await
        {
            Ok(invite) => invite,
            Err(error) => {
                self.clear_cloud_profile_if_stale(&error).await?;
                return Err(error);
            }
        };
        Ok(LocalDaemonResponse::CloudSessionInviteCreated {
            invite: cloud_session_invite_from_response(invite),
        })
    }

    async fn execute_show_cloud_session_invite_request(
        &self,
        request: ShowCloudSessionInviteRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let profile = self.required_cloud_relay_profile()?;
        let invite: CloudSessionInviteDetailsResponse = get_cloud_json(
            profile.api_url.clone(),
            format!(
                "/sessions/invites/{}",
                cloud_url_component(&request.invite_token)
            ),
        )
        .await?;
        Ok(LocalDaemonResponse::CloudSessionInviteShown {
            invite: cloud_session_invite_details_from_response(invite),
        })
    }

    async fn execute_accept_cloud_session_invite_request(
        &self,
        request: AcceptCloudSessionInviteRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let profile = self.required_cloud_relay_profile_with_session()?;
        let acceptance: CloudSessionInviteAcceptanceResponse = match post_cloud_json_dynamic(
            profile.api_url.clone(),
            format!(
                "/sessions/invites/{}/accept",
                cloud_url_component(&request.invite_token)
            ),
            serde_json::json!({
                "sessionToken": profile.cloud_session_token,
            }),
        )
        .await
        {
            Ok(acceptance) => acceptance,
            Err(error) => {
                self.clear_cloud_profile_if_stale(&error).await?;
                return Err(error);
            }
        };
        Ok(LocalDaemonResponse::CloudSessionInviteAccepted {
            acceptance: cloud_session_invite_acceptance_from_response(acceptance),
        })
    }

    async fn execute_revoke_cloud_session_invite_request(
        &self,
        request: RevokeCloudSessionInviteRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let profile = self.required_cloud_relay_profile_with_session()?;
        let revoked: CloudSessionInviteRevokedResponse = match post_cloud_json(
            profile.api_url.clone(),
            "/sessions/invites/revoke",
            serde_json::json!({
                "sessionToken": profile.cloud_session_token,
                "accountId": profile.account_id,
                "sessionId": request.session_id,
                "inviteId": request.invite_id,
            }),
        )
        .await
        {
            Ok(revoked) => revoked,
            Err(error) => {
                self.clear_cloud_profile_if_stale(&error).await?;
                return Err(error);
            }
        };
        Ok(LocalDaemonResponse::CloudSessionInviteRevoked {
            invite_id: revoked.invite_id,
            status: revoked.status,
        })
    }

    async fn execute_list_cloud_session_members_request(
        &self,
        request: ListCloudSessionMembersRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let profile = self.required_cloud_relay_profile_with_session()?;
        let listed: CloudSessionMembersResponse = match get_cloud_json(
            profile.api_url.clone(),
            format!(
                "/sessions/members?sessionToken={}&accountId={}&sessionId={}",
                cloud_url_component(profile.cloud_session_token.as_deref().unwrap_or_default()),
                cloud_url_component(&profile.account_id),
                cloud_url_component(&request.session_id),
            ),
        )
        .await
        {
            Ok(listed) => listed,
            Err(error) => {
                self.clear_cloud_profile_if_stale(&error).await?;
                return Err(error);
            }
        };
        Ok(LocalDaemonResponse::CloudSessionMembersListed {
            session_id: listed.session_id,
            members: listed
                .members
                .into_iter()
                .map(cloud_session_member_from_response)
                .collect(),
        })
    }

    async fn execute_list_cloud_collaborators_request(
        &self,
        _request: ListCloudCollaboratorsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let profile = self.required_cloud_relay_profile_with_session()?;
        let listed: CloudCollaboratorsResponse = match get_cloud_json(
            profile.api_url.clone(),
            format!(
                "/collaborators/recent?sessionToken={}&accountId={}",
                cloud_url_component(profile.cloud_session_token.as_deref().unwrap_or_default()),
                cloud_url_component(&profile.account_id),
            ),
        )
        .await
        {
            Ok(listed) => listed,
            Err(error) => {
                self.clear_cloud_profile_if_stale(&error).await?;
                return Err(error);
            }
        };
        Ok(LocalDaemonResponse::CloudCollaboratorsListed {
            collaborators: listed
                .collaborators
                .into_iter()
                .map(cloud_collaborator_from_response)
                .collect(),
        })
    }

    fn required_cloud_relay_profile(&self) -> Result<PersistedCloudRelayProfile, DaemonError> {
        self.config_projection
            .snapshot()
            .cloud_relay
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "load cloud relay profile",
                message: "cloud relay profile missing; run /relay cloud login first".to_string(),
            })
    }

    fn required_cloud_relay_profile_with_session(
        &self,
    ) -> Result<PersistedCloudRelayProfile, DaemonError> {
        let profile = self.required_cloud_relay_profile()?;
        if profile
            .cloud_session_token
            .as_deref()
            .unwrap_or("")
            .is_empty()
        {
            return Err(DaemonError::LocalTransport {
                operation: "load cloud relay session",
                message: "cloud session token missing; run /relay cloud login first".to_string(),
            });
        }
        Ok(profile)
    }

    async fn machine_runtime_profile_payload(&self) -> serde_json::Value {
        let config = self.config_projection.snapshot();
        let user_config = config.user_config.clone();
        let provider_catalog = if let Some(catalog) = self
            .provider_catalog_projection
            .get(PROVIDER_CATALOG_CACHE_TTL)
        {
            serde_json::to_value(catalog).ok()
        } else {
            tokio::task::spawn_blocking({
                let config = config.clone();
                move || load_provider_catalog(config)
            })
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|catalog| serde_json::to_value(catalog).ok())
        };
        let launch_target = infer_waiting_room_launch_target();
        serde_json::json!({
            "profileVersion": 1,
            "providerCatalog": provider_catalog,
            "userConfig": {
                "providers": user_config.providers,
                "ui": user_config.ui,
            },
            "defaultWorkspaceId": launch_target.as_ref().map(|target| target.workspace_id.clone()),
            "defaultWorktreeId": launch_target.as_ref().map(|target| target.worktree_id.clone()),
            "workspaces": launch_target.as_ref().map(|target| serde_json::json!([{
                "workspaceId": target.workspace_id,
                "worktreeId": target.worktree_id,
            }])),
            "os": std::env::consts::OS,
            "homeDir": std::env::var("HOME").ok(),
        })
    }

    async fn persist_cloud_profile(
        &self,
        profile: PersistedCloudRelayProfile,
    ) -> Result<PersistedCloudRelayProfile, DaemonError> {
        {
            let mut app = self.app.lock().await;
            app.persist_cloud_relay_profile(Some(profile.clone()))?;
        }
        self.config_projection.update({
            let app = self.app.lock().await;
            app.config().clone()
        });
        Ok(profile)
    }

    async fn clear_cloud_profile_if_stale(&self, error: &DaemonError) -> Result<(), DaemonError> {
        if !is_stale_cloud_link_error(error) {
            return Ok(());
        }
        {
            let mut app = self.app.lock().await;
            app.persist_cloud_relay_profile(None)?;
        }
        self.config_projection.update({
            let app = self.app.lock().await;
            app.config().clone()
        });
        Ok(())
    }

    async fn execute_get_user_config_request(
        &self,
        _request: GetUserConfigRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config_projection.snapshot();
        Ok(LocalDaemonResponse::UserConfig {
            path: config.user_config_path().clone(),
            config: config.user_config,
        })
    }

    async fn execute_get_user_config_schema_request(
        &self,
        _request: GetUserConfigSchemaRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::UserConfigSchema {
            entries: crate::config::DaemonConfig::user_config_schema(),
        })
    }

    async fn execute_set_user_config_value_request(
        &self,
        request: SetUserConfigValueRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (config, effects) = self
            .apply_user_config_mutation(UserConfigMutation::Set {
                path: request.path,
                value: request.value,
            })
            .await?;
        Ok(LocalDaemonResponse::UserConfigUpdated {
            path: config.user_config_path().clone(),
            config: config.user_config,
            effects,
        })
    }

    async fn execute_unset_user_config_value_request(
        &self,
        request: UnsetUserConfigValueRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (config, effects) = self
            .apply_user_config_mutation(UserConfigMutation::Unset { path: request.path })
            .await?;
        Ok(LocalDaemonResponse::UserConfigUpdated {
            path: config.user_config_path().clone(),
            config: config.user_config,
            effects,
        })
    }

    async fn apply_user_config_mutation(
        &self,
        mutation: UserConfigMutation,
    ) -> Result<(crate::config::DaemonConfig, Vec<UserConfigMutationEffect>), DaemonError> {
        let changed_path = match &mutation {
            UserConfigMutation::Set { path, .. } | UserConfigMutation::Unset { path } => {
                path.trim().to_string()
            }
        };
        let config = {
            let mut app = self.app.lock().await;
            match mutation {
                UserConfigMutation::Set { path, value } => {
                    app.set_user_config_value(path, value)?;
                }
                UserConfigMutation::Unset { path } => {
                    app.unset_user_config_value(path)?;
                }
            }
            app.config().clone()
        };
        self.config_projection.update(config.clone());
        let effects = self
            .apply_user_config_mutation_effects(&changed_path)
            .await?;
        Ok((config, effects))
    }

    async fn apply_user_config_mutation_effects(
        &self,
        path: &str,
    ) -> Result<Vec<UserConfigMutationEffect>, DaemonError> {
        if path == "providers.managed_io" {
            let outcomes = self
                .runtime_state
                .apply_provider_reload_policy(ProviderReloadTrigger::UserConfigChanged {
                    path: path.to_string(),
                })
                .await?;
            let summary = summarize_provider_reload_outcomes(&outcomes);
            let message = if summary.reloaded == 0 && summary.deferred == 0 {
                "managed I/O policy updated; no running provider needed reload".to_string()
            } else {
                format!(
                    "managed I/O policy updated; provider reloads: {} reloaded, {} deferred, {} unaffected",
                    summary.reloaded, summary.deferred, summary.unaffected
                )
            };
            return Ok(vec![UserConfigMutationEffect {
                kind: "provider_reload".to_string(),
                path: path.to_string(),
                message,
                provider_reload: Some(summary),
            }]);
        }

        if user_config_path_requires_daemon_restart(path) {
            return Ok(vec![UserConfigMutationEffect {
                kind: "restart_required".to_string(),
                path: path.to_string(),
                message: format!("`{path}` was updated; restart the daemon for it to take effect"),
                provider_reload: None,
            }]);
        }

        if user_config_path_is_unwired(path) {
            return Ok(vec![UserConfigMutationEffect {
                kind: "no_runtime_effect".to_string(),
                path: path.to_string(),
                message: format!(
                    "`{path}` was updated, but this key is not currently wired to runtime behavior"
                ),
                provider_reload: None,
            }]);
        }

        Ok(Vec::new())
    }

    async fn execute_set_credential_secret_request(
        &self,
        request: SetCredentialSecretRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let user_config = self.config_projection.snapshot().user_config;
        let service = crate::secret::RuntimeSecretService::with_vault_service(
            user_config.credentials,
            user_config.credential_vault.service,
        );
        service.set_vault_secret(&request.key, &request.value)?;
        Ok(LocalDaemonResponse::CredentialSecretStored { key: request.key })
    }

    async fn execute_delete_credential_secret_request(
        &self,
        request: DeleteCredentialSecretRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let user_config = self.config_projection.snapshot().user_config;
        let service = crate::secret::RuntimeSecretService::with_vault_service(
            user_config.credentials,
            user_config.credential_vault.service,
        );
        service.delete_vault_secret(&request.key)?;
        Ok(LocalDaemonResponse::CredentialSecretDeleted { key: request.key })
    }

    async fn execute_delete_kernel_request(
        &self,
        _request: DeleteKernelRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let kernel_id = self.config_projection.snapshot().daemon_id;
        let deleted_sessions = self.runtime_state.delete_current_kernel_sessions().await?;
        Ok(LocalDaemonResponse::KernelDeleted {
            kernel_id,
            deleted_sessions,
        })
    }

    async fn execute_approve_remote_machine_request(
        &self,
        request: ApproveRemoteMachineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config_projection.snapshot();
        let live = crate::transport::relay_discovery::list_live_machines(&config)
            .await
            .unwrap_or_default();
        let machine = resolve_machine_for_registry(&request.machine_ref, &live)?;
        crate::config::DaemonConfig::approve_remote_machine(
            machine.machine_id.clone(),
            machine.machine_alias.clone(),
        )?;
        self.invalidate_provider_catalog_caches().await;
        let machine = record_for_machine_id(machine.machine_id, live, &config.host_machine_id)?;
        Ok(LocalDaemonResponse::RemoteMachineApproved { machine })
    }

    async fn execute_forget_remote_machine_request(
        &self,
        request: ForgetRemoteMachineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config_projection.snapshot();
        let live = crate::transport::relay_discovery::list_live_machines(&config)
            .await
            .unwrap_or_default();
        let machine = resolve_machine_id_for_registry(&request.machine_ref, &live)?;
        let saved = crate::config::DaemonConfig::forget_remote_machine(machine.clone())?;
        self.invalidate_provider_catalog_caches().await;
        let machine = forgotten_machine_record(machine, saved.alias, live, &config.host_machine_id);
        Ok(LocalDaemonResponse::RemoteMachineForgotten { machine })
    }

    async fn execute_rename_remote_machine_request(
        &self,
        request: RenameRemoteMachineRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config_projection.snapshot();
        let live = crate::transport::relay_discovery::list_live_machines(&config)
            .await
            .unwrap_or_default();
        let machine = resolve_machine_id_for_registry(&request.machine_ref, &live)?;
        crate::config::DaemonConfig::rename_remote_machine(machine.clone(), request.alias)?;
        self.invalidate_provider_catalog_caches().await;
        let machine = record_for_machine_id(machine, live, &config.host_machine_id)?;
        Ok(LocalDaemonResponse::RemoteMachineRenamed { machine })
    }

    async fn execute_list_session_members_request(
        &self,
        request: ListSessionMembersRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let app = self.app.lock().await;
        let (members, invites) = app.sessions().list_session_members(&request.session_id)?;
        Ok(LocalDaemonResponse::SessionMembersListed { members, invites })
    }

    async fn execute_create_session_invite_request(
        &self,
        command: &KernelCommand,
        request: CreateSessionInviteRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let now_ms = current_unix_ms();
        let expires_at_ms = request
            .expires_in_ms
            .map(|expires_in_ms| now_ms.saturating_add(expires_in_ms));
        let invite_id = random_hex_id();
        let created_by_user_id = command_caller_user_id(command);
        let (session, invite) = {
            let app = self.app.lock().await;
            let result = app.sessions_mut().create_session_invite(
                &request.session_id,
                invite_id,
                created_by_user_id,
                expires_at_ms,
                request.max_uses.or(Some(1)),
            )?;
            result
        };
        let invite_token = encode_session_invite_token(&SessionInviteToken {
            version: 1,
            session_id: session.id().to_string(),
            invite_id: invite.invite_id().to_string(),
            created_by_user_id: invite.created_by_user_id().to_string(),
            issued_at_ms: invite.created_at_ms(),
            expires_at_ms: invite.expires_at_ms(),
            max_uses: invite.max_uses(),
        })?;
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::SessionInviteCreated {
            invite: SessionInviteRecord {
                invite,
                invite_token,
            },
            session,
        })
    }

    async fn execute_join_session_invite_request(
        &self,
        request: JoinSessionInviteRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let token = decode_session_invite_token(&request.invite_token)?;
        let now_ms = current_unix_ms();
        if token
            .expires_at_ms
            .is_some_and(|expires_at_ms| expires_at_ms <= now_ms)
        {
            return Err(DaemonError::LocalTransport {
                operation: "join session invite",
                message: "session invite is expired".to_string(),
            });
        }
        let (session, member) = {
            let app = self.app.lock().await;
            let result = app.sessions_mut().join_session_invite(
                &token.session_id,
                &token.invite_id,
                request.user_id,
                now_ms,
            )?;
            result
        };
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::SessionInviteJoined { member, session })
    }

    async fn execute_revoke_session_invite_request(
        &self,
        request: RevokeSessionInviteRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (session, invite) = {
            let app = self.app.lock().await;
            let result = app
                .sessions_mut()
                .revoke_session_invite(&request.session_id, &request.invite_ref)?;
            result
        };
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::SessionInviteRevoked { invite, session })
    }

    async fn execute_create_workspace_link_request(
        &self,
        command: &KernelCommand,
        request: CreateWorkspaceLinkRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let created_by_user_id = command_caller_user_id(command);
        let (session, link) = {
            let app = self.app.lock().await;
            let result = app.sessions_mut().create_workspace_link(
                &request.session_id,
                request.name,
                created_by_user_id,
            )?;
            result
        };
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::WorkspaceLinkCreated { link, session })
    }

    async fn execute_list_workspace_links_request(
        &self,
        request: ListWorkspaceLinksRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let app = self.app.lock().await;
        let links = app.sessions().list_workspace_links(&request.session_id)?;
        Ok(LocalDaemonResponse::WorkspaceLinksListed { links })
    }

    async fn execute_show_workspace_link_request(
        &self,
        request: ShowWorkspaceLinkRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let app = self.app.lock().await;
        let link = app
            .sessions()
            .resolve_workspace_link_ref(&request.session_id, &request.link_ref)?;
        Ok(LocalDaemonResponse::WorkspaceLinkShown { link })
    }

    async fn execute_attach_workspace_link_request(
        &self,
        command: &KernelCommand,
        request: AttachWorkspaceLinkRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let user_id = command_caller_user_id(command);
        let config = self.config_projection.snapshot();
        let machine_id = command
            .caller
            .machine_id
            .clone()
            .unwrap_or(config.host_machine_id);
        let kernel_id = config.daemon_id;
        let repo_root = if let Some(repo_root) = request.repo_root {
            repo_root
        } else {
            let app = self.app.lock().await;
            app.sessions()
                .get_session(&request.session_id)?
                .worktree_id()
                .to_string()
        };
        let (session, link, attachment) = {
            let app = self.app.lock().await;
            let result = app.sessions_mut().attach_workspace_link(
                &request.session_id,
                &request.link_ref,
                user_id,
                machine_id,
                kernel_id,
                repo_root,
                request.branch,
                request.repo_fingerprint,
            )?;
            result
        };
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::WorkspaceLinkAttached {
            link,
            attachment,
            session,
        })
    }

    async fn execute_detach_workspace_link_request(
        &self,
        command: &KernelCommand,
        request: DetachWorkspaceLinkRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let user_id = command_caller_user_id(command);
        let repo_root = request.repo_root.as_deref().map(std::path::Path::new);
        let (session, link, detached) = {
            let app = self.app.lock().await;
            let result = app.sessions_mut().detach_workspace_link(
                &request.session_id,
                &request.link_ref,
                user_id,
                repo_root,
            )?;
            result
        };
        self.session_projection.update(session.clone());
        Ok(LocalDaemonResponse::WorkspaceLinkDetached {
            link,
            detached,
            session,
        })
    }

    async fn execute_create_pairing_invite_request(
        &self,
        request: CreatePairingInviteRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config_projection.snapshot();
        let relay_url = config
            .relay_url
            .clone()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "create pairing invite",
                message: "relay URL must be configured before creating an invite".to_string(),
            })?;
        let relay_token =
            config
                .relay_token
                .clone()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "create pairing invite",
                    message: "relay token must be configured before creating an invite".to_string(),
                })?;
        let issued_at_ms = current_unix_ms();
        let expires_at_ms =
            issued_at_ms.saturating_add(request.expires_in_ms.unwrap_or(15 * 60 * 1000));
        let invite_id = random_hex_id();
        let token = PairingInviteToken {
            version: 1,
            intent: request.intent,
            invite_id: invite_id.clone(),
            relay_url: relay_url.clone(),
            relay_token,
            target_daemon_id: config.daemon_id.clone(),
            target_daemon_alias: config.daemon_alias.clone().or(request.alias),
            issuer_machine_id: config.host_machine_id,
            issued_at_ms,
            expires_at_ms,
            terminal_type: request
                .terminal_type
                .map(|terminal_type| terminal_type.as_str().to_string()),
            pairing_code: request.terminal_type.map(|_| random_pairing_code()),
            terminal_id: None,
        };
        let invite_token = encode_pairing_invite_token(&token)?;
        Ok(LocalDaemonResponse::PairingInviteCreated {
            invite: PairingInviteRecord {
                intent: token.intent,
                invite_id,
                invite_token,
                relay_url,
                target_daemon_id: token.target_daemon_id,
                target_daemon_alias: token.target_daemon_alias,
                issued_at_ms,
                expires_at_ms,
            },
        })
    }

    async fn execute_create_terminal_pairing_link_request(
        &self,
        request: CreateTerminalPairingLinkRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let terminal_type = request.terminal_type.unwrap_or(TerminalType::Cli);
        let config = self.config_projection.snapshot();
        let relay_url = config
            .relay_url
            .clone()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "create terminal pairing link",
                message: "relay URL must be configured before creating a terminal pairing link"
                    .to_string(),
            })?;
        let issued_at_ms = current_unix_ms();
        let expires_at_ms =
            issued_at_ms.saturating_add(request.expires_in_ms.unwrap_or(15 * 60 * 1000));
        let invite_id = random_hex_id();
        let pairing_code = random_pairing_code();
        let terminal_id = format!("{}-{}", terminal_type.as_str(), random_hex_id());
        let target_daemon_id = config.daemon_id.clone();
        let target_daemon_alias = config.daemon_alias.clone().or(request.alias);
        let relay_token = if let Some(profile) = config.cloud_relay.clone().filter(|profile| {
            profile.relay_url == relay_url
                && (profile.cloud_session_token.is_some() || profile.machine_credential.is_some())
        }) {
            let pairing: CloudPairingTokenResponse = match post_cloud_json(
                profile.api_url.clone(),
                "/pairing-tokens",
                serde_json::json!({
                    "accountId": profile.account_id,
                    "createdByUserId": profile.user_id,
                    "subjectKind": "client",
                }),
            )
            .await
            {
                Ok(pairing) => pairing,
                Err(error) => {
                    self.clear_cloud_profile_if_stale(&error).await?;
                    return Err(error);
                }
            };
            if let Err(error) = post_cloud_json::<serde_json::Value>(
                profile.api_url.clone(),
                "/clients/pair",
                serde_json::json!({
                    "accountId": profile.account_id,
                    "token": pairing.token,
                    "clientId": terminal_id,
                    "userId": profile.user_id,
                    "alias": format!("{} terminal", terminal_type.as_str()),
                }),
            )
            .await
            {
                self.clear_cloud_profile_if_stale(&error).await?;
                return Err(error);
            }
            let mut allowed_targets = vec![target_daemon_id.clone()];
            if let Some(alias) = target_daemon_alias.clone() {
                if !allowed_targets.iter().any(|target| target == &alias) {
                    allowed_targets.push(alias);
                }
            }
            match issue_cloud_runtime_token(
                &profile,
                &terminal_id,
                "client",
                Some(allowed_targets),
                Some(terminal_id.clone()),
                profile
                    .machine_credential
                    .as_ref()
                    .and(profile.machine_id.clone()),
                None,
            )
            .await
            {
                Ok(issued) => issued.token,
                Err(error) => {
                    self.clear_cloud_profile_if_stale(&error).await?;
                    return Err(error);
                }
            }
        } else {
            config
                .relay_token
                .clone()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "create terminal pairing link",
                    message:
                        "relay token must be configured before creating a terminal pairing link"
                            .to_string(),
                })?
        };
        let token = PairingInviteToken {
            version: 1,
            intent: PairingInviteIntent::Client,
            invite_id: invite_id.clone(),
            relay_url: relay_url.clone(),
            relay_token,
            target_daemon_id,
            target_daemon_alias,
            issuer_machine_id: config.host_machine_id,
            issued_at_ms,
            expires_at_ms,
            terminal_type: Some(terminal_type.as_str().to_string()),
            pairing_code: Some(pairing_code.clone()),
            terminal_id: Some(terminal_id.clone()),
        };
        let pairing_link = encode_terminal_pairing_link(&token)?;
        let _ = crate::config::DaemonConfig::record_paired_terminal(
            terminal_id.clone(),
            format!("pairing-link:{invite_id}"),
            token.target_daemon_alias.clone(),
            issued_at_ms,
            terminal_type.as_str(),
        )?;
        Ok(LocalDaemonResponse::TerminalPairingLinkCreated {
            pairing: TerminalPairingLinkRecord {
                terminal_id,
                pairing_link,
                pairing_code,
                invite_id,
                relay_url,
                target_daemon_id: token.target_daemon_id,
                target_daemon_alias: token.target_daemon_alias,
                terminal_type,
                issued_at_ms,
                expires_at_ms,
            },
        })
    }

    async fn execute_join_pairing_invite_request(
        &self,
        request: JoinPairingInviteRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let token = decode_pairing_invite_token(&request.invite_token)?;
        let now_ms = current_unix_ms();
        if token.expires_at_ms <= now_ms {
            return Err(DaemonError::LocalTransport {
                operation: "join pairing invite",
                message: "pairing invite is expired".to_string(),
            });
        }
        let config = self.config_projection.snapshot();
        let subject_id = request.subject_id.unwrap_or_else(|| match token.intent {
            PairingInviteIntent::Client => token
                .terminal_id
                .clone()
                .unwrap_or_else(|| format!("client-{}", random_hex_id())),
            PairingInviteIntent::Machine => config.host_machine_id.clone(),
        });
        let public_key_thumbprint = request
            .public_key_thumbprint
            .unwrap_or_else(|| public_key_thumbprint(&config.relay_public_key));
        match token.intent {
            PairingInviteIntent::Client => {
                crate::config::DaemonConfig::record_paired_terminal(
                    subject_id.clone(),
                    public_key_thumbprint.clone(),
                    request.alias.clone(),
                    now_ms,
                    token.terminal_type.as_deref().unwrap_or("cli"),
                )?;
            }
            PairingInviteIntent::Machine => {
                crate::config::DaemonConfig::pair_remote_machine(
                    subject_id.clone(),
                    public_key_thumbprint.clone(),
                    now_ms,
                )?;
                {
                    let mut app = self.app.lock().await;
                    app.configure_relay(Some(token.relay_url.clone()), Some(token.relay_token))?;
                    app.invalidate_provider_catalog_cache();
                    self.config_projection.update(app.config().clone());
                }
                self.provider_catalog_projection.invalidate();
            }
        }
        Ok(LocalDaemonResponse::PairingInviteJoined {
            pairing: PairingJoinRecord {
                intent: token.intent,
                subject_id,
                relay_url: token.relay_url,
                target_daemon_id: token.target_daemon_id,
                alias: request.alias,
                public_key_thumbprint,
                paired_at_ms: now_ms,
            },
        })
    }

    async fn execute_join_terminal_pairing_link_request(
        &self,
        request: JoinTerminalPairingLinkRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let token = decode_pairing_invite_token(&request.pairing_link)?;
        let now_ms = current_unix_ms();
        if token.expires_at_ms <= now_ms {
            return Err(DaemonError::LocalTransport {
                operation: "join terminal pairing link",
                message: "terminal pairing link is expired".to_string(),
            });
        }
        if token.intent != PairingInviteIntent::Client {
            return Err(DaemonError::LocalTransport {
                operation: "join terminal pairing link",
                message: "pairing link is not for a terminal".to_string(),
            });
        }
        let config = self.config_projection.snapshot();
        let terminal_type = request
            .terminal_type
            .or_else(|| token.terminal_type.as_deref().map(terminal_type_from_str))
            .unwrap_or(TerminalType::Cli);
        let terminal_id = request
            .terminal_id
            .or(token.terminal_id.clone())
            .unwrap_or_else(|| format!("{}-{}", terminal_type.as_str(), random_hex_id()));
        let public_key_thumbprint = public_key_thumbprint(&config.relay_public_key);
        let client = crate::config::DaemonConfig::record_paired_terminal(
            terminal_id.clone(),
            public_key_thumbprint.clone(),
            request.alias.clone(),
            now_ms,
            terminal_type.as_str(),
        )?;
        let terminal = terminal_record(client);
        Ok(LocalDaemonResponse::TerminalPairingLinkJoined {
            terminal,
            pairing: PairingJoinRecord {
                intent: PairingInviteIntent::Client,
                subject_id: terminal_id,
                relay_url: token.relay_url,
                target_daemon_id: token.target_daemon_id,
                alias: request.alias,
                public_key_thumbprint,
                paired_at_ms: now_ms,
            },
        })
    }

    async fn execute_list_terminals_request(&self) -> Result<LocalDaemonResponse, DaemonError> {
        Ok(LocalDaemonResponse::TerminalsListed {
            terminals: paired_terminal_records(),
        })
    }

    async fn execute_list_paired_clients_request(
        &self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let clients = crate::config::DaemonConfig::client_pairing_entries()
            .into_iter()
            .map(paired_client_record)
            .collect();
        Ok(LocalDaemonResponse::PairedClientsListed { clients })
    }

    async fn execute_record_paired_client_request(
        &self,
        request: RecordPairedClientRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let paired_at_ms = request.paired_at_ms.unwrap_or_else(current_unix_ms);
        let client = crate::config::DaemonConfig::record_paired_terminal(
            request.client_id,
            request.public_key_thumbprint,
            request.alias,
            paired_at_ms,
            request.terminal_type.unwrap_or(TerminalType::Cli).as_str(),
        )?;
        Ok(LocalDaemonResponse::PairedClientRecorded {
            client: paired_client_record(client),
        })
    }

    async fn execute_revoke_paired_client_request(
        &self,
        request: RevokePairedClientRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let client = crate::config::DaemonConfig::revoke_paired_client(request.client_id)?;
        Ok(LocalDaemonResponse::PairedClientRevoked {
            client: paired_client_record(client),
        })
    }

    async fn projected_provider_catalog_response(
        &self,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if let Some(catalog) = self
            .provider_catalog_projection
            .get(PROVIDER_CATALOG_CACHE_TTL)
        {
            return Ok(LocalDaemonResponse::ProviderCatalog { catalog });
        }

        let config = self.config_projection.snapshot();
        let catalog = tokio::task::spawn_blocking(move || load_provider_catalog(config))
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "load provider catalog",
                message: error.to_string(),
            })??;
        self.provider_catalog_projection.update(catalog.clone());
        Ok(LocalDaemonResponse::ProviderCatalog { catalog })
    }

    fn projected_session_or_absence(
        &self,
        session_id: &str,
    ) -> Option<Result<crate::session::RuntimeSession, DaemonError>> {
        if let Some(session) = self.session_projection.get(session_id) {
            return Some(Ok(session));
        }
        if self.session_projection.has_warmed_list() {
            return Some(Err(DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            }));
        }
        None
    }

    async fn authorize_session_membership(
        &self,
        command: &KernelCommand,
        request: &LocalDaemonRequest,
    ) -> Result<String, DaemonError> {
        if is_implicit_local_session_caller(command) {
            return Ok(DEFAULT_LOCAL_USER_ID.to_string());
        }
        if matches!(
            request,
            LocalDaemonRequest::CreateSession(_) | LocalDaemonRequest::JoinSessionInvite(_)
        ) {
            return Ok(command_session_user_id(command)
                .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string()));
        }

        let Some(scope) = request_session_scope(request) else {
            return Ok(command_session_user_id(command)
                .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string()));
        };
        let user_id = command_session_user_id(command).ok_or_else(|| {
            DaemonError::MissingSessionCallerIdentity {
                operation: command.command_type.clone(),
            }
        })?;

        match scope {
            SessionMembershipScope::AllSessions => Ok(user_id),
            SessionMembershipScope::SessionId(session_id) => {
                self.ensure_session_member(&session_id, &user_id).await?;
                Ok(user_id)
            }
            SessionMembershipScope::SessionRef {
                session_ref,
                workspace_id,
            } => {
                let session = self
                    .resolve_session_for_membership(&session_ref, workspace_id.as_deref())
                    .await?;
                if !session.has_member(&user_id) {
                    return Err(DaemonError::SessionAccessDenied {
                        session_id: session.id().to_string(),
                        user_id,
                    });
                }
                Ok(user_id)
            }
            SessionMembershipScope::AttachmentId(attachment_id) => {
                let session_id = if let Some(session_id) = self
                    .session_projection
                    .session_id_for_attachment(&attachment_id)
                {
                    session_id
                } else {
                    let app = self.app.lock().await;
                    app.sessions()
                        .list_sessions()
                        .into_iter()
                        .find(|session| session.has_attachment(&attachment_id))
                        .map(|session| session.id().to_string())
                        .ok_or_else(|| DaemonError::AttachmentNotFound {
                            attachment_id: attachment_id.clone(),
                        })?
                };
                self.ensure_session_member(&session_id, &user_id).await?;
                Ok(user_id)
            }
        }
    }

    async fn ensure_session_member(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<(), DaemonError> {
        if let Some(session) = self.session_projection.get(session_id) {
            if session.has_member(user_id) {
                return Ok(());
            }
            return Err(DaemonError::SessionAccessDenied {
                session_id: session.id().to_string(),
                user_id: user_id.to_string(),
            });
        }
        let session = {
            let app = self.app.lock().await;
            app.sessions().get_session(session_id)?
        };
        if session.has_member(user_id) {
            Ok(())
        } else {
            Err(DaemonError::SessionAccessDenied {
                session_id: session.id().to_string(),
                user_id: user_id.to_string(),
            })
        }
    }

    async fn resolve_session_for_membership(
        &self,
        session_ref: &str,
        workspace_id: Option<&str>,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        if let Some(session) = self
            .session_projection
            .resolve_session_ref(session_ref, workspace_id)
        {
            return Ok(session);
        }
        let app = self.app.lock().await;
        app.sessions()
            .resolve_session_ref(session_ref, workspace_id)
    }

    async fn projected_session_history_response(
        &self,
        request: &GetSessionHistoryRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        if let Some(page) = self.history_projection.page(
            &request.session_id,
            request.agent_id.as_deref(),
            request.round_count,
            request.max_chars,
            request.before_entry_index,
            request.before_entry_char_offset,
        ) {
            return Some(Ok(LocalDaemonResponse::SessionHistory {
                entries: page.entries,
                next_cursor: page.next_cursor,
            }));
        }

        let session = match self.projected_session_or_absence(&request.session_id)? {
            Ok(session) => session,
            Err(error) => return Some(Err(error)),
        };
        Some(
            self.execute_session_history_request_from_session(session, request.clone())
                .await,
        )
    }

    async fn execute_session_history_request(
        &self,
        request: GetSessionHistoryRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = {
            let app = self.app.lock().await;
            app.sessions().get_session(&request.session_id)?
        };
        self.execute_session_history_request_from_session(session, request)
            .await
    }

    async fn execute_prompt_input_history_request(
        &self,
        request: GetPromptInputHistoryRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session_id = request.session_id.clone();
        let limit = request.limit.unwrap_or(5000).clamp(1, 5000);
        let after_sequence = request.after_sequence;
        let history = self.operational_history_store.clone();
        tokio::task::spawn_blocking(move || {
            let mut events = prompt_input_history_events_for_kind(
                &history,
                &session_id,
                "user_prompt",
                after_sequence,
                limit,
            )?;
            events.extend(prompt_input_history_events_for_kind(
                &history,
                &session_id,
                "prompt_input",
                after_sequence,
                limit,
            )?);
            events.sort_by_key(|event| event.sequence);
            events.truncate(limit);
            Ok(LocalDaemonResponse::PromptInputHistory {
                entries: events
                    .into_iter()
                    .filter_map(prompt_input_history_entry_from_event)
                    .collect(),
            })
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "load prompt input history",
            message: error.to_string(),
        })?
    }

    async fn execute_record_prompt_input_history_request(
        &self,
        request: RecordPromptInputHistoryRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if request.text.trim().is_empty() {
            return Ok(LocalDaemonResponse::PromptInputHistoryRecorded {
                entry: PromptInputHistoryEntry {
                    sequence: 0,
                    timestamp_ms: 0,
                    session_id: request.session_id,
                    source_attachment_id: request.attachment_id,
                    kind: request.kind,
                    text: String::new(),
                },
            });
        }
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "input_kind".to_string(),
            serde_json::Value::String(
                match request.kind {
                    PromptInputHistoryEntryKind::Prompt => "prompt",
                    PromptInputHistoryEntryKind::Command => "command",
                }
                .to_string(),
            ),
        );
        if let Some(attachment_id) = request.attachment_id.clone() {
            metadata.insert(
                "source_attachment_id".to_string(),
                serde_json::Value::String(attachment_id),
            );
        }
        let event = self.operational_history_store.append_operational_event(
            HistoryEventKind::PromptInput,
            Some(HistoryEventRole::User),
            Some(request.text),
            metadata,
            crate::history::HistoryEventTurnContext {
                session_id: Some(request.session_id),
                ..Default::default()
            },
        )?;
        let entry = prompt_input_history_entry_from_event(event).ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "record prompt input history",
                message: "recorded event could not be converted".to_string(),
            }
        })?;
        Ok(LocalDaemonResponse::PromptInputHistoryRecorded { entry })
    }

    async fn execute_query_history_request(
        &self,
        query: HistoryEventQuery,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let requested_limit = query.limit.unwrap_or(100).clamp(1, 500);
        let history = self.operational_history_store.clone();
        let archive_config = self
            .config_projection
            .snapshot()
            .user_config
            .history
            .archive;
        tokio::task::spawn_blocking(move || {
            let mut events = history.query_events(query.clone())?;
            let archive_client = HistoryArchiveClient::from_config(&archive_config)?;
            let archive_capabilities = archive_client.capabilities().ok();
            if archive_capabilities
                .as_ref()
                .map(|capabilities| capabilities.search)
                .unwrap_or(false)
            {
                let archive_response = archive_client.search_events(query.clone())?;
                merge_history_events(&mut events, archive_response.events);
            }
            events.sort_by(|left, right| {
                left.sequence
                    .cmp(&right.sequence)
                    .then_with(|| left.event_id.cmp(&right.event_id))
            });
            events.truncate(requested_limit);
            let next_sequence = if events.len() == requested_limit {
                events.last().map(|event| event.sequence)
            } else {
                None
            };
            Ok(LocalDaemonResponse::HistoryEvents {
                events,
                next_sequence,
            })
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "query history",
            message: error.to_string(),
        })?
    }

    async fn execute_terminal_output_request(
        &self,
        request: PumpTerminalOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.terminal_output_executor.execute(request).await
    }

    async fn execute_teardown_provider_processes_request(
        &self,
        request: TeardownProviderProcessesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (processes, sessions) = {
            let mut app = self.app.lock().await;
            let processes =
                app.teardown_provider_processes(request.provider.as_deref(), request.force)?;
            let session_ids = processes
                .iter()
                .flat_map(|process| process.owner_session_ids.iter())
                .cloned()
                .collect::<HashSet<_>>();
            let sessions = session_ids
                .into_iter()
                .filter_map(|session_id| {
                    crate::app::KernelSessionReadService::new(&app)
                        .session_snapshot(&session_id)
                        .ok()
                })
                .collect::<Vec<_>>();
            (processes, sessions)
        };
        for session in &sessions {
            self.agent_runtime_projection.update_session(session);
            self.session_projection.update(session.clone());
        }
        Ok(LocalDaemonResponse::ProviderProcessesTornDown { processes })
    }

    async fn execute_session_history_request_from_session(
        &self,
        session: crate::session::RuntimeSession,
        request: GetSessionHistoryRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let history = self.history_store.clone();
        let operational_history = self.operational_history_store.clone();
        let history_projection = self.history_projection.clone();
        tokio::task::spawn_blocking(move || {
            let operational_entries = operational_history
                .load_session_history_entries(session.id(), request.agent_id.as_deref())?;
            let entries = if operational_entries.is_empty()
                && !operational_history.has_session_events(session.id())?
                && !operational_history.legacy_fallback_disabled(session.id())?
            {
                let legacy_entries = history.load(&session)?;
                match request.agent_id.as_deref() {
                    Some(agent_id) => legacy_entries
                        .into_iter()
                        .filter(|entry| entry.agent_id.as_deref() == Some(agent_id))
                        .collect(),
                    None => legacy_entries,
                }
            } else {
                operational_entries
            };
            history_projection.update_entries(session.id(), entries.clone());
            let page = page_history_entries(
                entries,
                request.agent_id.as_deref(),
                request.round_count,
                request.max_chars,
                request.before_entry_index,
                request.before_entry_char_offset,
            );
            Ok(LocalDaemonResponse::SessionHistory {
                entries: page.entries,
                next_cursor: page.next_cursor,
            })
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "load session history",
            message: error.to_string(),
        })?
    }

    #[allow(dead_code)]
    pub(crate) async fn daemon_health_projection(
        &self,
        last_event_id: u64,
    ) -> DaemonHealthProjection {
        DaemonHealthProjection::new(
            last_event_id,
            self.session_runtime.queue_snapshots().await,
            self.agent_runtime.queue_snapshots().await,
            self.workflow_runtime.queue_snapshots().await,
            self.provider_runtime_lanes.queue_snapshots(),
            self.provider_runtime_lanes.health_snapshot(),
            self.capability_health.snapshot(),
            self.session_projection.health_snapshot(),
            self.agent_runtime_projection.health_snapshot(),
            self.provider_catalog_projection
                .health_snapshot(PROVIDER_CATALOG_CACHE_TTL),
            self.transport_health.snapshot(
                crate::runtime_transport::RECENT_EVENT_LIMIT,
                crate::runtime_transport::COMMAND_RESULT_CACHE_LIMIT,
                crate::runtime_transport::INBOUND_REQUEST_LIMIT,
            ),
            self.terminal_health.snapshot(),
            self.session_projection
                .workspace_coordination_snapshot(self.workspace_coordinator.active_claims()),
            self.runtime_state.managed_io_health_snapshot().await,
            self.session_projection
                .invariant_snapshot(&self.agent_runtime_projection),
        )
    }

    async fn dispatch_interactive(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if SessionActor::is_session_interactive_command(&request) {
            return self
                .session_runtime
                .dispatch_session_command(command, request)
                .await;
        }

        match request {
            LocalDaemonRequest::GrantAgentCapability(request) => {
                return self
                    .execute_grant_agent_capability_request(&command, request)
                    .await;
            }
            LocalDaemonRequest::MoveAgentToRemote(request) => {
                return self
                    .execute_move_agent_to_remote_request(&command, request)
                    .await;
            }
            LocalDaemonRequest::RevokeAgentCapability(request) => {
                return self
                    .execute_revoke_agent_capability_request(&command, request)
                    .await;
            }
            LocalDaemonRequest::SubmitPrompt(request) => {
                return self
                    .agent_runtime
                    .dispatch_prompt_submit(&command, request)
                    .await;
            }
            LocalDaemonRequest::CancelActivePrompt(request) => {
                return self
                    .agent_runtime
                    .dispatch_prompt_cancel(&command, request)
                    .await;
            }
            _ => {
                return Err(DaemonError::LocalTransport {
                    operation: "route interactive kernel command",
                    message: format!(
                        "unsupported interactive command `{}` reached the explicit interactive router",
                        command.command_type
                    ),
                });
            }
        }
    }

    async fn dispatch_normal_or_background(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::RespondToInteraction(_) => Err(DaemonError::LocalTransport {
                operation: "dispatch normal or background",
                message: "interaction responses must be dispatched through the session runtime"
                    .to_string(),
            }),
            LocalDaemonRequest::LaunchProviderRun(request) => {
                ProviderLaunchCommandExecutor::new(self.runtime_state.clone())
                    .execute(request, command_caller_user_id(&command))
                    .await
            }
            LocalDaemonRequest::ListSessions(request) => {
                self.execute_cold_list_sessions_request(request).await
            }
            LocalDaemonRequest::ResolveSession(request) => {
                self.execute_cold_resolve_session_request(request).await
            }
            LocalDaemonRequest::GetSessionState(request) => {
                self.execute_cold_get_session_state_request(request).await
            }
            LocalDaemonRequest::GetDaemonHealth(_) => Ok(LocalDaemonResponse::DaemonHealth {
                projection: self.daemon_health_projection(0).await,
            }),
            LocalDaemonRequest::GetProviderRun(request) => {
                self.execute_get_provider_run_request(request).await
            }
            LocalDaemonRequest::GetPromptInputHistory(request) => {
                self.execute_prompt_input_history_request(request).await
            }
            LocalDaemonRequest::RecordPromptInputHistory(request) => {
                self.execute_record_prompt_input_history_request(request)
                    .await
            }
            LocalDaemonRequest::GetProviderCatalog(_) => {
                self.projected_provider_catalog_response().await
            }
            LocalDaemonRequest::GetProviderCommandCatalogs(_) => {
                provider_command_catalogs_response()
            }
            LocalDaemonRequest::InstallMcpServer(request) => {
                self.execute_install_mcp_server_request(request).await
            }
            LocalDaemonRequest::UpdateMcpServer(request) => {
                self.execute_update_mcp_server_request(request).await
            }
            LocalDaemonRequest::UninstallMcpServer(request) => {
                self.execute_uninstall_mcp_server_request(request).await
            }
            LocalDaemonRequest::ImportMcpServers(request) => {
                self.execute_import_mcp_servers_request(request).await
            }
            LocalDaemonRequest::GetMcpServer(request) => {
                self.execute_get_mcp_server_request(request).await
            }
            LocalDaemonRequest::ListMcpServers(request) => {
                self.execute_list_mcp_servers_request(request).await
            }
            LocalDaemonRequest::InstallSkill(request) => {
                self.execute_install_skill_request(request).await
            }
            LocalDaemonRequest::UpdateSkill(request) => {
                self.execute_update_skill_request(request).await
            }
            LocalDaemonRequest::UninstallSkill(request) => {
                self.execute_uninstall_skill_request(request).await
            }
            LocalDaemonRequest::ImportSkills(request) => {
                self.execute_import_skills_request(request).await
            }
            LocalDaemonRequest::GetSkill(request) => self.execute_get_skill_request(request).await,
            LocalDaemonRequest::ListSkills(request) => {
                self.execute_list_skills_request(request).await
            }
            LocalDaemonRequest::RelayStatus(_) => self.projected_relay_status_response().await,
            LocalDaemonRequest::ConfigureRelay(request) => {
                self.execute_configure_relay_request(request).await
            }
            LocalDaemonRequest::CloudRelayStatus(_) => {
                self.execute_cloud_relay_status_request().await
            }
            LocalDaemonRequest::StartCloudRelayLogin(request) => {
                self.execute_start_cloud_relay_login_request(request).await
            }
            LocalDaemonRequest::PollCloudRelayLogin(request) => {
                self.execute_poll_cloud_relay_login_request(request).await
            }
            LocalDaemonRequest::LogoutCloudRelay(request) => {
                self.execute_logout_cloud_relay_request(request).await
            }
            LocalDaemonRequest::PairCloudRelayClient(request) => {
                self.execute_pair_cloud_relay_client_request(request).await
            }
            LocalDaemonRequest::PairCloudRelayMachine(request) => {
                self.execute_pair_cloud_relay_machine_request(request).await
            }
            LocalDaemonRequest::ConnectCloudRelay(request) => {
                self.execute_connect_cloud_relay_request(request).await
            }
            LocalDaemonRequest::IssueCloudRelayClientToken(request) => {
                self.execute_issue_cloud_relay_client_token_request(request)
                    .await
            }
            LocalDaemonRequest::CreateCloudSessionInvite(request) => {
                self.execute_create_cloud_session_invite_request(request)
                    .await
            }
            LocalDaemonRequest::ShowCloudSessionInvite(request) => {
                self.execute_show_cloud_session_invite_request(request)
                    .await
            }
            LocalDaemonRequest::AcceptCloudSessionInvite(request) => {
                self.execute_accept_cloud_session_invite_request(request)
                    .await
            }
            LocalDaemonRequest::RevokeCloudSessionInvite(request) => {
                self.execute_revoke_cloud_session_invite_request(request)
                    .await
            }
            LocalDaemonRequest::ListCloudSessionMembers(request) => {
                self.execute_list_cloud_session_members_request(request)
                    .await
            }
            LocalDaemonRequest::ListCloudCollaborators(request) => {
                self.execute_list_cloud_collaborators_request(request).await
            }
            LocalDaemonRequest::GetUserConfig(request) => {
                self.execute_get_user_config_request(request).await
            }
            LocalDaemonRequest::GetUserConfigSchema(request) => {
                self.execute_get_user_config_schema_request(request).await
            }
            LocalDaemonRequest::SetUserConfigValue(request) => {
                self.execute_set_user_config_value_request(request).await
            }
            LocalDaemonRequest::UnsetUserConfigValue(request) => {
                self.execute_unset_user_config_value_request(request).await
            }
            LocalDaemonRequest::SetCredentialSecret(request) => {
                self.execute_set_credential_secret_request(request).await
            }
            LocalDaemonRequest::DeleteCredentialSecret(request) => {
                self.execute_delete_credential_secret_request(request).await
            }
            LocalDaemonRequest::DeleteKernel(request) => {
                self.execute_delete_kernel_request(request).await
            }
            LocalDaemonRequest::ListRemoteMachines(_) => {
                self.projected_remote_machines_response().await
            }
            LocalDaemonRequest::ListRemoteMachineKernels(request) => {
                self.projected_remote_machine_kernels_response(request.machine_ref)
                    .await
            }
            LocalDaemonRequest::GetWaitingRoomInventory(_) => {
                self.projected_waiting_room_inventory_response().await
            }
            LocalDaemonRequest::GetWaitingRoomPublicSnapshot(_) => {
                self.projected_waiting_room_public_snapshot_response().await
            }
            LocalDaemonRequest::SearchWorkspaceDirectories(request) => {
                self.execute_search_workspace_directories_request(request)
                    .await
            }
            LocalDaemonRequest::CreateWorkspaceDirectory(request) => {
                self.execute_create_workspace_directory_request(request)
                    .await
            }
            LocalDaemonRequest::ListWorkspaceWorktrees(request) => {
                self.execute_list_workspace_worktrees_request(request).await
            }
            LocalDaemonRequest::CreateWorkspaceWorktree(request) => {
                self.execute_create_workspace_worktree_request(request)
                    .await
            }
            LocalDaemonRequest::ApproveRemoteMachine(request) => {
                self.execute_approve_remote_machine_request(request).await
            }
            LocalDaemonRequest::ForgetRemoteMachine(request) => {
                self.execute_forget_remote_machine_request(request).await
            }
            LocalDaemonRequest::RenameRemoteMachine(request) => {
                self.execute_rename_remote_machine_request(request).await
            }
            LocalDaemonRequest::ListSessionMembers(request) => {
                self.execute_list_session_members_request(request).await
            }
            LocalDaemonRequest::CreateSessionInvite(request) => {
                self.execute_create_session_invite_request(&command, request)
                    .await
            }
            LocalDaemonRequest::JoinSessionInvite(request) => {
                self.execute_join_session_invite_request(request).await
            }
            LocalDaemonRequest::RevokeSessionInvite(request) => {
                self.execute_revoke_session_invite_request(request).await
            }
            LocalDaemonRequest::CreateWorkspaceLink(request) => {
                self.execute_create_workspace_link_request(&command, request)
                    .await
            }
            LocalDaemonRequest::ListWorkspaceLinks(request) => {
                self.execute_list_workspace_links_request(request).await
            }
            LocalDaemonRequest::ShowWorkspaceLink(request) => {
                self.execute_show_workspace_link_request(request).await
            }
            LocalDaemonRequest::AttachWorkspaceLink(request) => {
                self.execute_attach_workspace_link_request(&command, request)
                    .await
            }
            LocalDaemonRequest::DetachWorkspaceLink(request) => {
                self.execute_detach_workspace_link_request(&command, request)
                    .await
            }
            LocalDaemonRequest::CreatePairingInvite(request) => {
                self.execute_create_pairing_invite_request(request).await
            }
            LocalDaemonRequest::JoinPairingInvite(request) => {
                self.execute_join_pairing_invite_request(request).await
            }
            LocalDaemonRequest::CreateTerminalPairingLink(request) => {
                self.execute_create_terminal_pairing_link_request(request)
                    .await
            }
            LocalDaemonRequest::JoinTerminalPairingLink(request) => {
                self.execute_join_terminal_pairing_link_request(request)
                    .await
            }
            LocalDaemonRequest::ListTerminals(_) => self.execute_list_terminals_request().await,
            LocalDaemonRequest::ListPairedClients(_) => {
                self.execute_list_paired_clients_request().await
            }
            LocalDaemonRequest::RecordPairedClient(request) => {
                self.execute_record_paired_client_request(request).await
            }
            LocalDaemonRequest::RevokePairedClient(request) => {
                self.execute_revoke_paired_client_request(request).await
            }
            LocalDaemonRequest::GetProviderAuthStatus(request) => {
                Self::execute_get_provider_auth_status_request(request).await
            }
            LocalDaemonRequest::StartProviderLogin(request) => {
                Self::execute_start_provider_login_request(request).await
            }
            LocalDaemonRequest::LogoutProvider(request) => {
                self.execute_logout_provider_request(request).await
            }
            LocalDaemonRequest::ListProviderProcesses(request) => {
                execute_list_provider_processes_request(&self.app, request).await
            }
            LocalDaemonRequest::TeardownProviderProcesses(request) => {
                self.execute_teardown_provider_processes_request(request)
                    .await
            }
            LocalDaemonRequest::GetSessionHistory(request) => {
                self.execute_session_history_request(request).await
            }
            LocalDaemonRequest::QueryHistory(request) => {
                self.execute_query_history_request(history_query_from_request(request))
                    .await
            }
            LocalDaemonRequest::SearchHistory(request) => {
                self.execute_query_history_request(history_query_from_search_request(request))
                    .await
            }
            LocalDaemonRequest::PumpTerminalOutput(request) => {
                self.execute_terminal_output_request(request).await
            }
            request @ (LocalDaemonRequest::RunShellCommand(_)
            | LocalDaemonRequest::ReadDirectoryTree(_)
            | LocalDaemonRequest::ReadFile(_)
            | LocalDaemonRequest::EditFile(_)
            | LocalDaemonRequest::InspectGit(_)
            | LocalDaemonRequest::CaptureScreenshot(_)
            | LocalDaemonRequest::StoreTransferredFile(_)) => {
                self.execute_capability_request(request).await
            }
            LocalDaemonRequest::SubmitPrompt(request) => {
                self.agent_runtime
                    .dispatch_prompt_submit(&command, request)
                    .await
            }
            LocalDaemonRequest::CompletePrompt(request) => {
                self.agent_runtime
                    .dispatch_prompt_complete(&command, request)
                    .await
            }
            LocalDaemonRequest::CancelActivePrompt(request) => {
                self.agent_runtime
                    .dispatch_prompt_cancel(&command, request)
                    .await
            }
            LocalDaemonRequest::ListAgents(request) => {
                self.execute_cold_list_agents_request(request).await
            }
            request @ (LocalDaemonRequest::CreateWorkflow(_)
            | LocalDaemonRequest::AliasWorkflow(_)
            | LocalDaemonRequest::ListWorkflows(_)
            | LocalDaemonRequest::ResolveWorkflow(_)
            | LocalDaemonRequest::CreateWorkflowPublication(_)
            | LocalDaemonRequest::ListWorkflowPublications(_)
            | LocalDaemonRequest::GetWorkflowPublication(_)
            | LocalDaemonRequest::DisableWorkflowPublication(_)
            | LocalDaemonRequest::CreateWorkflowPublicationPairCode(_)
            | LocalDaemonRequest::RedeemWorkflowPublicationPairCode(_)
            | LocalDaemonRequest::ListWorkflowPublicationSenders(_)
            | LocalDaemonRequest::RevokeWorkflowPublicationSender(_)
            | LocalDaemonRequest::AuthenticateWorkflowPublicationSender(_)
            | LocalDaemonRequest::CreateWorkflowEndpoint(_)
            | LocalDaemonRequest::AliasWorkflowEndpoint(_)
            | LocalDaemonRequest::BindWorkflowEndpoint(_)
            | LocalDaemonRequest::AddWorkflowNode(_)
            | LocalDaemonRequest::RemoveWorkflowNode(_)
            | LocalDaemonRequest::UpdateWorkflowNodeInstructions(_)
            | LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(_)
            | LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(_)
            | LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(_)
            | LocalDaemonRequest::SetWorkflowNodeMaxTurns(_)
            | LocalDaemonRequest::AddWorkflowEdge(_)
            | LocalDaemonRequest::RemoveWorkflowEdge(_)
            | LocalDaemonRequest::InvokeWorkflowEndpoint(_)
            | LocalDaemonRequest::ListWorkflowRuns(_)
            | LocalDaemonRequest::GetWorkflowRun(_)
            | LocalDaemonRequest::CancelWorkflowRun(_)
            | LocalDaemonRequest::ResumeWorkflowRun(_)
            | LocalDaemonRequest::CreateWorkflowWatchdog(_)
            | LocalDaemonRequest::ListWorkflowWatchdogs(_)
            | LocalDaemonRequest::SetWorkflowWatchdogEnabled(_)
            | LocalDaemonRequest::RemoveWorkflowWatchdog(_)
            | LocalDaemonRequest::SetWorkflowFlushContext(_)
            | LocalDaemonRequest::SetWorkflowRunOutputSchema(_)
            | LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(_)
            | LocalDaemonRequest::SetWorkflowLaunchPolicy(_)
            | LocalDaemonRequest::ListQueuedWorkflowLaunches(_)
            | LocalDaemonRequest::RemoveQueuedWorkflowLaunch(_)
            | LocalDaemonRequest::ClearQueuedWorkflowLaunches(_)
            | LocalDaemonRequest::ValidateWorkflowOutput(_)
            | LocalDaemonRequest::AckWorkflowTurn(_)) => {
                self.workflow_runtime
                    .dispatch_workflow_command(command, request)
                    .await
            }
            request @ (LocalDaemonRequest::CreateSession(_)
            | LocalDaemonRequest::AttachToSession(_)
            | LocalDaemonRequest::DetachFromSession(_)
            | LocalDaemonRequest::UpdateSessionConfig(_)
            | LocalDaemonRequest::AliasAgent(_)
            | LocalDaemonRequest::UpdateAgentConfig(_)
            | LocalDaemonRequest::UpdateAgentProfile(_)
            | LocalDaemonRequest::UpdateAgentSubstitutes(_)
            | LocalDaemonRequest::ResizeTerminal(_)
            | LocalDaemonRequest::EndSession(_)
            | LocalDaemonRequest::DeleteSession(_)
            | LocalDaemonRequest::AliasSession(_)
            | LocalDaemonRequest::SpawnAgent(_)
            | LocalDaemonRequest::MoveAgentToRemote(_)
            | LocalDaemonRequest::DestroyAgent(_)
            | LocalDaemonRequest::FocusAgent(_)
            | LocalDaemonRequest::CycleAgentFocus(_)
            | LocalDaemonRequest::GrantAgentCapability(_)
            | LocalDaemonRequest::RevokeAgentCapability(_)
            | LocalDaemonRequest::PollRuntimeNotices(_)) => {
                self.dispatch_interactive(command, request).await
            }
        }
    }

    async fn execute_install_mcp_server_request(
        &self,
        request: InstallMcpServerRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let registry = crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        let path = registry.install(&request.config)?;
        Ok(LocalDaemonResponse::McpServerInstalled {
            mcp: request.config,
            path,
        })
    }

    async fn execute_update_mcp_server_request(
        &self,
        request: UpdateMcpServerRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let registry = crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        let path = registry.update(&request.config)?;
        Ok(LocalDaemonResponse::McpServerUpdated {
            mcp: request.config,
            path,
        })
    }

    async fn execute_uninstall_mcp_server_request(
        &self,
        request: UninstallMcpServerRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let registry = crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        let path = registry.uninstall(&request.name)?;
        Ok(LocalDaemonResponse::McpServerUninstalled {
            name: request.name,
            path,
        })
    }

    async fn execute_import_mcp_servers_request(
        &self,
        request: ImportMcpServersRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let registry = crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        let outcome = match request.provider.as_str() {
            "codex" => crate::mcp::import_codex_mcp_servers(&registry, request.name.as_deref())?,
            "opencode" => {
                let workspace = registry_workspace_root(request.workspace_id.as_deref())?;
                crate::mcp::import_opencode_mcp_servers(
                    &registry,
                    &workspace,
                    request.name.as_deref(),
                )?
            }
            _ => {
                return Err(DaemonError::InvalidConfig {
                    field: "provider",
                    message: "only Codex and OpenCode MCP import are supported",
                });
            }
        };
        Ok(LocalDaemonResponse::McpServersImported { outcome })
    }

    async fn execute_get_mcp_server_request(
        &self,
        request: GetMcpServerRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let registry = crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        let Some(mcp) = registry.get(&request.name)? else {
            return Err(DaemonError::LocalTransport {
                operation: "mcp.get",
                message: format!("MCP `{}` was not found", request.name),
            });
        };
        Ok(LocalDaemonResponse::McpServer { mcp })
    }

    async fn execute_list_mcp_servers_request(
        &self,
        request: ListMcpServersRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let registry = crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        Ok(LocalDaemonResponse::McpServersListed {
            mcps: registry.list()?,
        })
    }

    async fn execute_install_skill_request(
        &self,
        request: InstallSkillRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workspace = registry_workspace_root(request.workspace_id.as_deref())?;
        let source_path = if request.source_path.is_absolute() {
            request.source_path
        } else {
            workspace.join(request.source_path)
        };
        let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        let (skill, path) = registry.install_from_path(&source_path)?;
        Ok(LocalDaemonResponse::SkillInstalled { skill, path })
    }

    async fn execute_update_skill_request(
        &self,
        request: UpdateSkillRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workspace = registry_workspace_root(request.workspace_id.as_deref())?;
        let source_path = if request.source_path.is_absolute() {
            request.source_path
        } else {
            workspace.join(request.source_path)
        };
        let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        let (skill, path) = registry.update_from_path(&source_path)?;
        Ok(LocalDaemonResponse::SkillUpdated { skill, path })
    }

    async fn execute_uninstall_skill_request(
        &self,
        request: UninstallSkillRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        let (skill, path) = registry.uninstall(&request.name)?;
        Ok(LocalDaemonResponse::SkillUninstalled { skill, path })
    }

    async fn execute_import_skills_request(
        &self,
        request: ImportSkillsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let workspace = registry_workspace_root(request.workspace_id.as_deref())?;
        let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        let outcome = match request.provider.as_str() {
            "codex" => {
                crate::skill::import_codex_skills(&registry, &workspace, request.name.as_deref())?
            }
            "opencode" => crate::skill::import_opencode_skills(
                &registry,
                &workspace,
                request.name.as_deref(),
            )?,
            _ => {
                return Err(DaemonError::InvalidConfig {
                    field: "provider",
                    message: "only Codex and OpenCode skill import are supported",
                });
            }
        };
        Ok(LocalDaemonResponse::SkillsImported { outcome })
    }

    async fn execute_grant_agent_capability_request(
        &self,
        command: &KernelCommand,
        request: GrantAgentCapabilityRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_caller_user_id(command);
        match request.kind {
            AgentGrantKind::Mcp => {
                self.ensure_mcp_exists(request.workspace_id.as_deref(), &request.name)?;
                let agent = self
                    .runtime_state
                    .grant_agent_mcp(&request.agent_ref, request.name, &caller_user_id)
                    .await?;
                Ok(LocalDaemonResponse::AgentCapabilityGranted { agent })
            }
            AgentGrantKind::Skill => {
                self.ensure_skill_exists(request.workspace_id.as_deref(), &request.name)?;
                let agent = self
                    .runtime_state
                    .grant_agent_skill(&request.agent_ref, request.name, &caller_user_id)
                    .await?;
                Ok(LocalDaemonResponse::AgentCapabilityGranted { agent })
            }
        }
    }

    async fn execute_move_agent_to_remote_request(
        &self,
        command: &KernelCommand,
        request: MoveAgentToRemoteRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_caller_user_id(command);
        let agent = self
            .runtime_state
            .move_agent_to_remote(
                &request.session_id,
                &request.agent_ref,
                &request.machine_ref,
                &caller_user_id,
            )
            .await?;
        Ok(LocalDaemonResponse::AgentMovedToRemote { agent })
    }

    async fn execute_revoke_agent_capability_request(
        &self,
        command: &KernelCommand,
        request: RevokeAgentCapabilityRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_caller_user_id(command);
        let agent = match request.kind {
            AgentGrantKind::Mcp => {
                self.runtime_state
                    .revoke_agent_mcp(&request.agent_ref, &request.name, &caller_user_id)
                    .await?
            }
            AgentGrantKind::Skill => {
                self.runtime_state
                    .revoke_agent_skill(&request.agent_ref, &request.name, &caller_user_id)
                    .await?
            }
        };
        Ok(LocalDaemonResponse::AgentCapabilityRevoked { agent })
    }

    fn ensure_mcp_exists(&self, workspace_id: Option<&str>, name: &str) -> Result<(), DaemonError> {
        let registry = crate::mcp::ArrobaMcpRegistry::new(mcp_registry_roots(workspace_id)?);
        if registry.get(name)?.is_none() {
            return Err(DaemonError::LocalTransport {
                operation: "agent.capability.grant",
                message: format!("MCP `{name}` is not installed"),
            });
        }
        Ok(())
    }

    fn ensure_skill_exists(
        &self,
        workspace_id: Option<&str>,
        name: &str,
    ) -> Result<(), DaemonError> {
        let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(workspace_id)?);
        if registry.get(name)?.is_none() {
            return Err(DaemonError::LocalTransport {
                operation: "agent.capability.grant",
                message: format!("skill `{name}` is not installed"),
            });
        }
        Ok(())
    }

    async fn execute_get_skill_request(
        &self,
        request: GetSkillRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        let Some(skill) = registry.get(&request.name)? else {
            return Err(DaemonError::LocalTransport {
                operation: "skill.get",
                message: format!("skill `{}` was not found", request.name),
            });
        };
        Ok(LocalDaemonResponse::Skill { skill })
    }

    async fn execute_list_skills_request(
        &self,
        request: ListSkillsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let registry = crate::skill::ArrobaSkillRegistry::new(skill_registry_roots(
            request.workspace_id.as_deref(),
        )?);
        Ok(LocalDaemonResponse::SkillsListed {
            skills: registry.list()?,
        })
    }

    async fn apply_focus_projection_refresh(
        &self,
        refresh: FocusProjectionRefresh,
        result: &Result<LocalDaemonResponse, DaemonError>,
    ) {
        if result.is_err() {
            return;
        }
        match refresh {
            FocusProjectionRefresh::None => {}
            FocusProjectionRefresh::AgentSpawn => {
                if let Ok(LocalDaemonResponse::AgentSpawned { agent }) = result {
                    self.focus_projection
                        .update(agent.session_id(), Some(agent.id()))
                        .await;
                }
            }
            FocusProjectionRefresh::SnapshotSession { session_id } => {
                let focused_agent_id =
                    if let Some(session) = self.session_projection.get(&session_id) {
                        session.focused_agent_id().map(str::to_string)
                    } else if let Ok(app) = self.app.try_lock() {
                        app.sessions()
                            .get_session(&session_id)
                            .ok()
                            .and_then(|session| session.focused_agent_id().map(str::to_string))
                    } else {
                        return;
                    };
                self.focus_projection
                    .update(&session_id, focused_agent_id.as_deref())
                    .await;
            }
        }
    }

    async fn apply_session_projection_refresh(
        &self,
        refresh: SessionProjectionRefresh,
        result: &Result<LocalDaemonResponse, DaemonError>,
    ) {
        let response = match result {
            Ok(response) => response,
            Err(_) => return,
        };

        let mut refreshed_session_ids = Vec::new();
        for session in response_sessions(response) {
            refreshed_session_ids.push(session.id().to_string());
            if should_update_agent_runtime_projection_from_response(response) {
                self.agent_runtime_projection.update_session(&session);
            }
            self.session_projection.update(session);
        }
        if let LocalDaemonResponse::SessionsListed { sessions } = response {
            for session in sessions {
                self.agent_runtime_projection.update_session(session);
            }
            self.session_projection.update_list(sessions.clone());
        }
        for session_id in response_removed_session_ids(response) {
            self.agent_runtime_projection.remove_session(session_id);
            self.session_projection.remove(session_id);
            self.history_projection.remove(session_id);
            refreshed_session_ids.push(session_id.to_string());
        }

        let mut snapshot_session_ids = refresh.session_ids(response);
        snapshot_session_ids.sort();
        snapshot_session_ids.dedup();
        match refresh {
            SessionProjectionRefresh::None => {}
            SessionProjectionRefresh::SnapshotAgentResponse => {
                for session_id in snapshot_session_ids {
                    if let Some(session) = self.session_projection.get(&session_id) {
                        refreshed_session_ids.push(session.id().to_string());
                        self.agent_runtime_projection.update_session(&session);
                    }
                }
            }
        }

        if !matches!(refresh, SessionProjectionRefresh::None) || !refreshed_session_ids.is_empty() {
            self.provider_process_projection.invalidate();
        }

        refreshed_session_ids.sort();
        refreshed_session_ids.dedup();
        for session_id in refreshed_session_ids {
            self.clear_provider_launch_pending_if_settled(&session_id)
                .await;
        }
    }

    async fn apply_provider_launch_projection_state(
        &self,
        result: &Result<LocalDaemonResponse, DaemonError>,
    ) {
        if let Ok(LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run }) = result {
            self.pending_provider_launch_sessions
                .lock()
                .await
                .insert(provider_run.session_id().to_string());
        }
    }

    async fn apply_provider_run_projection_refresh(
        &self,
        result: &Result<LocalDaemonResponse, DaemonError>,
    ) {
        match result {
            Ok(LocalDaemonResponse::ProviderRun { provider_run })
            | Ok(LocalDaemonResponse::ProviderRunLaunched { provider_run })
            | Ok(LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run }) => {
                self.provider_run_projection.update(provider_run.clone());
                self.provider_process_projection.invalidate();
            }
            _ => {}
        }
    }

    async fn apply_agent_lane_cleanup(&self, result: &Result<LocalDaemonResponse, DaemonError>) {
        let Ok(response) = result else {
            return;
        };
        match response {
            LocalDaemonResponse::AgentDestroyed { agent } => {
                self.agent_runtime.remove_agent_lane(agent.id()).await;
            }
            LocalDaemonResponse::SessionDeleted { session }
            | LocalDaemonResponse::SessionEnded { session } => {
                self.agent_runtime.remove_session_state(session.id());
                self.agent_runtime
                    .remove_agent_lanes(session.agents().iter().map(|agent| agent.id()))
                    .await;
                self.workflow_runtime
                    .remove_session_lane(session.id())
                    .await;
            }
            LocalDaemonResponse::KernelDeleted {
                deleted_sessions, ..
            } => {
                for session in deleted_sessions {
                    self.agent_runtime.remove_session_state(session.id());
                    self.agent_runtime
                        .remove_agent_lanes(session.agents().iter().map(|agent| agent.id()))
                        .await;
                    self.workflow_runtime
                        .remove_session_lane(session.id())
                        .await;
                }
            }
            _ => {}
        }
    }

    async fn has_unsettled_pending_provider_launch(&self, session_id: &str) -> bool {
        if !self
            .pending_provider_launch_sessions
            .lock()
            .await
            .contains(session_id)
        {
            return false;
        }
        if let Some(is_starting) =
            self.provider_launch_is_still_starting_from_projection(session_id)
        {
            if !is_starting {
                self.pending_provider_launch_sessions
                    .lock()
                    .await
                    .remove(session_id);
            }
            return is_starting;
        }
        true
    }

    async fn clear_provider_launch_pending_if_settled(&self, session_id: &str) {
        if !self
            .pending_provider_launch_sessions
            .lock()
            .await
            .contains(session_id)
        {
            return;
        }
        if let Some(is_starting) =
            self.provider_launch_is_still_starting_from_projection(session_id)
        {
            if !is_starting {
                self.pending_provider_launch_sessions
                    .lock()
                    .await
                    .remove(session_id);
            }
            return;
        }
        let Ok(app) = self.app.try_lock() else {
            return;
        };
        let is_still_starting = app
            .sessions()
            .get_session(session_id)
            .ok()
            .and_then(|session| session.active_provider_run_id().map(str::to_string))
            .and_then(|provider_run_id| app.providers().get_run(&provider_run_id).ok())
            .is_some_and(|run| run.state() == ProviderRunState::Starting);
        if !is_still_starting {
            self.pending_provider_launch_sessions
                .lock()
                .await
                .remove(session_id);
        }
    }

    fn provider_launch_is_still_starting_from_projection(&self, session_id: &str) -> Option<bool> {
        let session = self.session_projection.get(session_id)?;
        let Some(provider_run_id) = session.active_provider_run_id() else {
            return Some(false);
        };
        let run = self.provider_run_projection.get(provider_run_id)?;
        Some(run.state() == ProviderRunState::Starting)
    }
}

fn waiting_room_inventory_version(
    sessions: &[WaitingRoomPublicSessionSummary],
    relay_status: &RelayStatus,
    remote_machines: &[crate::local::RemoteMachineRecord],
    remote_kernels: &[RelayKernelPresence],
    terminals: &[TerminalRecord],
    launch_target: Option<&WaitingRoomLaunchTarget>,
) -> Result<String, DaemonError> {
    let payload = serde_json::to_vec(&serde_json::json!({
        "sessions": sessions,
        "relay_status": relay_status,
        "remote_machines": remote_machines,
        "remote_kernels": remote_kernels,
        "terminals": terminals,
        "launch_target": launch_target,
    }))
    .map_err(|error| DaemonError::LocalTransport {
        operation: "serialize waiting room inventory snapshot",
        message: error.to_string(),
    })?;
    Ok(URL_SAFE_NO_PAD.encode(Sha256::digest(payload)))
}

fn waiting_room_session_summaries(
    sessions: Vec<crate::session::RuntimeSession>,
) -> Vec<WaitingRoomPublicSessionSummary> {
    let mut workspace_labels: HashMap<String, Option<String>> = HashMap::new();
    let mut worktree_labels: HashMap<(String, String), Option<String>> = HashMap::new();
    sessions
        .into_iter()
        .map(|session| {
            let workspace_id = session.workspace_id().to_string();
            let worktree_id = session.worktree_id().to_string();
            let workspace_label = workspace_labels
                .entry(workspace_id.clone())
                .or_insert_with(|| workspace_display_label(&workspace_id))
                .clone();
            let worktree_label = worktree_labels
                .entry((workspace_id.clone(), worktree_id.clone()))
                .or_insert_with(|| {
                    let branch = detect_git_branch(&worktree_id).ok();
                    worktree_display_label(&worktree_id, &workspace_id, branch.as_deref())
                })
                .clone();
            WaitingRoomPublicSessionSummary {
                id: session.id().to_string(),
                alias: session.alias().map(ToOwned::to_owned),
                workspace_id: workspace_id.clone(),
                worktree_id: worktree_id.clone(),
                workspace_label: workspace_label.clone(),
                directory: Some(workspace_id),
                worktree_label,
                created_at_ms: session.created_at_ms(),
                last_used_at_ms: session.last_used_at_ms(),
                status: session.status(),
                connected_cli_count: session.attachment_ids().len(),
                activity: waiting_room_session_activity_summary(&session),
                agents: waiting_room_public_agent_summaries(
                    &session,
                    workspace_label.clone(),
                    &mut worktree_labels,
                ),
                workflows: waiting_room_public_workflow_summaries(&session),
            }
        })
        .collect()
}

fn waiting_room_public_agent_summaries(
    session: &crate::session::RuntimeSession,
    workspace_label: Option<String>,
    worktree_labels: &mut HashMap<(String, String), Option<String>>,
) -> Vec<WaitingRoomPublicAgentSummary> {
    let workspace_id = session.workspace_id().to_string();
    let mut agents = session
        .agents()
        .iter()
        .map(|agent| {
            let worktree_id = agent
                .worktree_id()
                .unwrap_or_else(|| session.worktree_id())
                .to_string();
            let worktree_label = worktree_labels
                .entry((workspace_id.clone(), worktree_id.clone()))
                .or_insert_with(|| {
                    let branch = detect_git_branch(&worktree_id).ok();
                    worktree_display_label(&worktree_id, &workspace_id, branch.as_deref())
                })
                .clone();
            WaitingRoomPublicAgentSummary {
                id: agent.id().to_string(),
                agent_ref: agent.agent_ref().to_string(),
                alias: agent.alias().map(ToOwned::to_owned),
                created_at_ms: agent.created_at_ms(),
                provider: agent.primary_provider().to_string(),
                model: agent.primary_model().map(ToOwned::to_owned),
                variant: agent.primary_effort().map(ToOwned::to_owned),
                permission: waiting_room_agent_permission(session, agent),
                workspace_id: workspace_id.clone(),
                worktree_id,
                workspace_label: workspace_label.clone(),
                directory: Some(workspace_id.clone()),
                worktree_label,
                activity: waiting_room_agent_activity_summary(session, agent),
            }
        })
        .collect::<Vec<_>>();
    agents.sort_by(|left, right| {
        left.created_at_ms
            .cmp(&right.created_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    agents
}

fn waiting_room_agent_permission(
    session: &crate::session::RuntimeSession,
    agent: &crate::agent::AgentInstance,
) -> Option<String> {
    agent
        .permission_level_override()
        .or(session.agent_defaults().permission_level)
        .or_else(|| {
            session
                .config_state()
                .values()
                .get("agents.permissions")
                .and_then(|value| crate::provider::AgentPermissionLevel::parse(value.as_str()))
        })
        .map(|permission| permission.as_str().to_string())
}

fn waiting_room_agent_activity_summary(
    session: &crate::session::RuntimeSession,
    agent: &crate::agent::AgentInstance,
) -> WaitingRoomPublicItemActivitySummary {
    let active_prompt_count = usize::from(session.active_prompt_for_agent(agent.id()).is_some());
    let queued_prompt_count = session
        .queued_prompts_for_agent(agent.id())
        .map(|queued| queued.len())
        .unwrap_or(0);
    let error = agent.state() == AgentState::Error;
    WaitingRoomPublicItemActivitySummary {
        working: agent.state() == AgentState::Working
            || agent.is_processing()
            || active_prompt_count > 0,
        active_prompt_count,
        queued_prompt_count,
        error,
    }
}

fn waiting_room_public_workflow_summaries(
    session: &crate::session::RuntimeSession,
) -> Vec<WaitingRoomPublicWorkflowSummary> {
    let mut workflows = session
        .workflows()
        .iter()
        .map(|workflow| WaitingRoomPublicWorkflowSummary {
            id: workflow.id().to_string(),
            alias: workflow.alias().map(ToOwned::to_owned),
            created_at_ms: workflow.created_at_ms(),
            activity: waiting_room_workflow_activity_summary(session, workflow.id()),
            nodes: workflow
                .nodes()
                .iter()
                .map(|node| WaitingRoomPublicWorkflowNodeSummary {
                    id: node.id().to_string(),
                    agent_id: node.agent_id().to_string(),
                    label: node.public_label().to_string(),
                })
                .collect(),
            edges: workflow
                .edges()
                .iter()
                .map(|edge| WaitingRoomPublicWorkflowEdgeSummary {
                    id: edge.id().to_string(),
                    from_node_id: edge.from_node_id().to_string(),
                    to_node_id: edge.to_node_id().to_string(),
                })
                .collect(),
            endpoints: workflow
                .endpoints()
                .iter()
                .map(|endpoint| WaitingRoomPublicWorkflowEndpointSummary {
                    id: endpoint.id().to_string(),
                    alias: endpoint.alias().map(ToOwned::to_owned),
                    entry_node_id: endpoint.entry_node_id().to_string(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    workflows.sort_by(|left, right| {
        left.created_at_ms
            .cmp(&right.created_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    workflows
}

fn waiting_room_workflow_activity_summary(
    session: &crate::session::RuntimeSession,
    workflow_id: &str,
) -> WaitingRoomPublicItemActivitySummary {
    let working = session.workflow_runs().iter().any(|run| {
        run.workflow_id() == workflow_id
            && matches!(
                run.status(),
                crate::session::WorkflowRunStatus::Created
                    | crate::session::WorkflowRunStatus::Running
                    | crate::session::WorkflowRunStatus::Waiting
                    | crate::session::WorkflowRunStatus::Completing
            )
    });
    let error = session.workflow_runs().iter().any(|run| {
        run.workflow_id() == workflow_id
            && matches!(run.status(), crate::session::WorkflowRunStatus::Failed)
    });
    WaitingRoomPublicItemActivitySummary {
        working,
        active_prompt_count: 0,
        queued_prompt_count: 0,
        error,
    }
}

fn waiting_room_session_activity_summary(
    session: &crate::session::RuntimeSession,
) -> WaitingRoomSessionActivitySummary {
    let active_prompt_agent_ids: HashSet<&str> = session
        .prompt_states()
        .iter()
        .filter(|(_, state)| state.active_prompt().is_some())
        .map(|(agent_id, _)| agent_id.as_str())
        .collect();
    let active_prompt_count = if active_prompt_agent_ids.is_empty() && session.has_active_prompt() {
        1
    } else {
        active_prompt_agent_ids.len()
    };
    let queued_prompt_count = if session.prompt_states().is_empty() {
        session.queued_prompts().len()
    } else {
        session
            .prompt_states()
            .values()
            .map(|state| state.queued_prompts().len())
            .sum()
    };
    let mut working_agent_count = session
        .agents()
        .iter()
        .filter(|agent| {
            agent.state() == AgentState::Working
                || agent.is_processing()
                || active_prompt_agent_ids.contains(agent.id())
        })
        .count();
    if working_agent_count == 0 && active_prompt_count > 0 {
        working_agent_count = 1;
    }
    WaitingRoomSessionActivitySummary {
        agent_count: session.agents().len(),
        working_agent_count,
        active_prompt_count,
        queued_prompt_count,
        error_agent_count: session
            .agents()
            .iter()
            .filter(|agent| agent.state() == AgentState::Error)
            .count(),
    }
}

fn infer_waiting_room_launch_target() -> Option<WaitingRoomLaunchTarget> {
    let cwd = std::env::current_dir().ok()?;
    let cwd_string = cwd.display().to_string();
    let worktree = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&cwd)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| cwd_string.clone());
    let workspace = std::process::Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(&cwd)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|common_dir| {
            if let Some(stripped) = common_dir.strip_suffix("/.git") {
                stripped.to_string()
            } else {
                worktree.clone()
            }
        })
        .unwrap_or_else(|| cwd_string.clone());
    let branch = detect_git_branch(&worktree).ok();
    Some(WaitingRoomLaunchTarget {
        workspace_label: workspace_display_label(&workspace),
        directory: Some(workspace.clone()),
        worktree_label: worktree_display_label(&worktree, &workspace, branch.as_deref()),
        workspace_id: workspace,
        worktree_id: worktree,
    })
}

fn search_workspace_directories(query: &str, limit: usize) -> Result<Vec<String>, DaemonError> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();
    let roots = workspace_search_roots();
    let launch_target = infer_waiting_room_launch_target();
    let trimmed_query = query.trim();
    let normalized_query = trimmed_query.to_lowercase();

    if let Some(target) = launch_target {
        push_matching_path(
            &mut results,
            &mut seen,
            target.workspace_id,
            &normalized_query,
            limit,
        );
        push_matching_path(
            &mut results,
            &mut seen,
            target.worktree_id,
            &normalized_query,
            limit,
        );
    }

    if normalized_query.is_empty() {
        for root in &roots {
            push_unique_path(&mut results, &mut seen, root.display().to_string());
            if results.len() >= limit {
                break;
            }
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        push_unique_path(&mut results, &mut seen, path.display().to_string());
                        if results.len() >= limit {
                            break;
                        }
                    }
                }
            }
            if results.len() >= limit {
                break;
            }
        }
        results.truncate(limit);
        return Ok(results);
    }

    if looks_like_path_query(trimmed_query) {
        append_directory_completion(&mut results, &mut seen, trimmed_query, limit)?;
        results.truncate(limit);
        return Ok(results);
    }

    for root in roots {
        append_matching_directory_children(
            &mut results,
            &mut seen,
            &root,
            &normalized_query,
            limit,
        )?;
    }
    results.truncate(limit);
    Ok(results)
}

fn workspace_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    for candidate in [
        std::env::current_dir().ok(),
        std::env::var_os("HOME").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    {
        let path = candidate;
        if seen.insert(path.clone()) {
            roots.push(path);
        }
    }
    roots
}

fn push_unique_path(results: &mut Vec<String>, seen: &mut HashSet<String>, value: String) {
    if value.trim().is_empty() {
        return;
    }
    if seen.insert(value.clone()) {
        results.push(value);
    }
}

fn push_matching_path(
    results: &mut Vec<String>,
    seen: &mut HashSet<String>,
    value: String,
    normalized_query: &str,
    limit: usize,
) {
    if results.len() >= limit {
        return;
    }
    if normalized_query.is_empty() || value.to_lowercase().contains(normalized_query) {
        push_unique_path(results, seen, value);
    }
}

fn looks_like_path_query(query: &str) -> bool {
    query.starts_with('/') || query.starts_with("~/") || query == "~" || query.contains('/')
}

fn append_directory_completion(
    results: &mut Vec<String>,
    seen: &mut HashSet<String>,
    query: &str,
    limit: usize,
) -> Result<(), DaemonError> {
    let expanded = expand_workspace_query_path(query);
    if query == "~" {
        if expanded.is_dir() {
            push_unique_path(results, seen, expanded.display().to_string());
            append_matching_directory_children(results, seen, &expanded, "", limit)?;
        }
        return Ok(());
    }
    if query.ends_with('/') {
        return append_matching_directory_children(results, seen, &expanded, "", limit);
    }

    if expanded.is_dir() {
        push_unique_path(results, seen, expanded.display().to_string());
    }
    let prefix = expanded
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_lowercase();
    let parent = expanded
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    append_matching_directory_children(results, seen, &parent, &prefix, limit)
}

fn append_matching_directory_children(
    results: &mut Vec<String>,
    seen: &mut HashSet<String>,
    parent: &Path,
    normalized_query: &str,
    limit: usize,
) -> Result<(), DaemonError> {
    if results.len() >= limit || !parent.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(parent).map_err(|error| DaemonError::LocalTransport {
        operation: "search workspace directories",
        message: error.to_string(),
    })?;
    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("")
            .to_lowercase();
        if normalized_query.is_empty() || name.contains(normalized_query) {
            matches.push(path);
        }
    }
    matches.sort_by(|left, right| {
        directory_match_rank(left, normalized_query)
            .cmp(&directory_match_rank(right, normalized_query))
            .then_with(|| directory_sort_name(left).cmp(&directory_sort_name(right)))
    });
    for path in matches {
        push_unique_path(results, seen, path.display().to_string());
        if results.len() >= limit {
            break;
        }
    }
    Ok(())
}

fn directory_match_rank(path: &Path, normalized_query: &str) -> (u8, u8) {
    let name = directory_sort_name(path);
    let query = normalized_query.trim();
    let exact_rank = if !query.is_empty() && name == query {
        0
    } else if query.is_empty() || name.starts_with(query) {
        1
    } else {
        2
    };
    let hidden_rank = if query.starts_with('.') || !name.starts_with('.') {
        0
    } else {
        1
    };
    (exact_rank, hidden_rank)
}

fn directory_sort_name(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_lowercase()
}

fn expand_workspace_query_path(query: &str) -> PathBuf {
    if query == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(query));
    }
    if let Some(rest) = query.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(query)
}

fn create_workspace_directory(path: &str) -> Result<String, DaemonError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "create workspace directory",
            message: "workspace path is required".to_string(),
        });
    }
    let expanded = expand_workspace_query_path(trimmed);
    let directory = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "create workspace directory",
                message: error.to_string(),
            })?
            .join(expanded)
    };
    if directory.exists() && !directory.is_dir() {
        return Err(DaemonError::LocalTransport {
            operation: "create workspace directory",
            message: format!("{} exists and is not a directory", directory.display()),
        });
    }
    std::fs::create_dir_all(&directory).map_err(|error| DaemonError::LocalTransport {
        operation: "create workspace directory",
        message: error.to_string(),
    })?;
    Ok(directory.display().to_string())
}

fn list_workspace_worktrees(
    workspace_id: &str,
    current_worktree: Option<&str>,
) -> Result<Vec<WorkspaceWorktreeRecord>, DaemonError> {
    let workspace_path = PathBuf::from(workspace_id);
    let output = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&workspace_path)
        .output();
    let Ok(output) = output else {
        return Ok(vec![WorkspaceWorktreeRecord {
            path: workspace_id.to_string(),
            branch: None,
            label: worktree_display_label(workspace_id, workspace_id, None),
            current: true,
        }]);
    };
    if !output.status.success() {
        let branch = detect_git_branch(workspace_id).ok();
        return Ok(vec![WorkspaceWorktreeRecord {
            path: workspace_id.to_string(),
            label: worktree_display_label(workspace_id, workspace_id, branch.as_deref()),
            branch,
            current: true,
        }]);
    }
    let current_worktree_path = current_worktree.unwrap_or(workspace_id);
    let mut worktrees = parse_git_worktree_list(String::from_utf8_lossy(&output.stdout).as_ref())
        .into_iter()
        .map(|(path, branch)| WorkspaceWorktreeRecord {
            current: same_fs_path(&path, current_worktree_path),
            label: worktree_display_label(&path, workspace_id, branch.as_deref()),
            branch,
            path,
        })
        .collect::<Vec<_>>();
    if worktrees.is_empty() {
        let branch = detect_git_branch(workspace_id).ok();
        worktrees.push(WorkspaceWorktreeRecord {
            path: workspace_id.to_string(),
            label: worktree_display_label(workspace_id, workspace_id, branch.as_deref()),
            branch,
            current: true,
        });
    }
    Ok(worktrees)
}

fn parse_git_worktree_list(stdout: &str) -> Vec<(String, Option<String>)> {
    let mut entries = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_branch: Option<String> = None;
    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            if let Some(path) = current_path.take() {
                entries.push((path, current_branch.take()));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            if let Some(path) = current_path.replace(rest.trim().to_string()) {
                entries.push((path, current_branch.take()));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("branch ") {
            current_branch = Some(rest.trim().trim_start_matches("refs/heads/").to_string());
        }
    }
    if let Some(path) = current_path.take() {
        entries.push((path, current_branch.take()));
    }
    entries
}

fn detect_git_branch(path: &str) -> Result<String, DaemonError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "detect git branch",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "detect git branch",
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn workspace_display_label(workspace_path: &str) -> Option<String> {
    git_command_output(workspace_path, &["remote", "get-url", "origin"])
        .as_deref()
        .and_then(repo_label_from_remote_url)
        .or_else(|| {
            Path::new(workspace_path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
}

fn repo_label_from_remote_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches(".git");
    let candidate = if let Some(rest) = trimmed.strip_prefix("git@") {
        rest.split_once(':').map(|(_, path)| path.to_string())
    } else if let Some((_, path)) = trimmed.split_once("://") {
        let mut parts = path.split('/').collect::<Vec<_>>();
        if parts.len() >= 3 {
            Some(parts.split_off(parts.len() - 2).join("/"))
        } else {
            None
        }
    } else {
        None
    }?;
    let parts = candidate
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    Some(format!(
        "{}/{}",
        parts[parts.len() - 2],
        parts[parts.len() - 1]
    ))
}

fn worktree_display_label(
    path: &str,
    workspace_path: &str,
    branch: Option<&str>,
) -> Option<String> {
    let branch = branch
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| *value != "HEAD")
        .unwrap_or("detached");
    if same_fs_path(path, workspace_path) {
        return Some(branch.to_string());
    }
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|value| !value.is_empty())?;
    Some(format!("{name} / {branch}"))
}

fn git_command_output(path: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn create_waiting_room_worktree(
    workspace_path: &str,
    requested_path: Option<&str>,
    requested_branch: Option<&str>,
    requested_base_ref: Option<&str>,
) -> Result<String, DaemonError> {
    let repo_root = resolve_repo_root(workspace_path)?;
    let base_ref = requested_base_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or(resolve_preferred_base_ref(&repo_root)?);
    let description = std::env::var("ARROBA_WAITING_ROOM_WORKTREE_DESCRIPTION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "{}-session",
                repo_root
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or("workspace")
            )
        });
    let branch_base = format!(
        "arroba/{}-{}",
        slugify_segment(&description),
        timestamp_slug(),
    );
    let branch = match requested_branch
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => value.to_string(),
        None => resolve_available_branch_name(&repo_root, &branch_base)?,
    };
    let parent = repo_root.parent().unwrap_or(&repo_root);
    let directory_base = format!(
        "{}-{}",
        repo_root
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("workspace"),
        slugify_segment(&branch.replace('/', "-"))
    );
    let directory = requested_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| resolve_requested_worktree_directory(parent, value))
        .unwrap_or_else(|| resolve_available_worktree_directory(parent, &directory_base));
    run_git(
        &repo_root,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            directory.to_str().unwrap_or(""),
            &base_ref,
        ],
    )?;
    Ok(directory.display().to_string())
}

fn resolve_repo_root(workspace_path: &str) -> Result<PathBuf, DaemonError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(workspace_path)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "resolve repo root",
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "resolve repo root",
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

fn resolve_preferred_base_ref(repo_root: &Path) -> Result<String, DaemonError> {
    for candidate in ["main", "master"] {
        if git_ref_exists(repo_root, &format!("refs/heads/{candidate}"))? {
            return Ok(candidate.to_string());
        }
    }
    let branch = detect_git_branch(repo_root.to_string_lossy().as_ref())?;
    Ok(if branch == "HEAD" || branch.is_empty() {
        "HEAD".to_string()
    } else {
        branch
    })
}

fn resolve_available_branch_name(repo_root: &Path, base_name: &str) -> Result<String, DaemonError> {
    let mut attempt = base_name.to_string();
    let mut index = 1;
    while git_ref_exists(repo_root, &format!("refs/heads/{attempt}"))? {
        attempt = format!("{base_name}-{index}");
        index += 1;
    }
    Ok(attempt)
}

fn resolve_available_worktree_directory(parent: &Path, base_name: &str) -> PathBuf {
    let mut attempt = parent.join(base_name);
    let mut index = 1;
    while attempt.exists() {
        attempt = parent.join(format!("{base_name}-{index}"));
        index += 1;
    }
    attempt
}

fn resolve_requested_worktree_directory(parent: &Path, value: &str) -> PathBuf {
    let expanded = expand_workspace_query_path(value);
    if expanded.is_absolute() {
        expanded
    } else {
        parent.join(expanded)
    }
}

fn git_ref_exists(repo_root: &Path, reference: &str) -> Result<bool, DaemonError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", reference])
        .current_dir(repo_root)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "check git ref",
            message: error.to_string(),
        })?;
    Ok(output.status.success())
}

fn run_git(repo_root: &Path, args: &[&str]) -> Result<(), DaemonError> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "run git command",
            message: error.to_string(),
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(DaemonError::LocalTransport {
            operation: "run git command",
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

fn slugify_segment(value: &str) -> String {
    let slug = value
        .trim()
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    slug.trim_matches('-')
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn timestamp_slug() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now.to_string()
}

fn same_fs_path(left: &str, right: &str) -> bool {
    std::fs::canonicalize(left).ok() == std::fs::canonicalize(right).ok()
        || Path::new(left) == Path::new(right)
}

fn paired_client_record(client: crate::config::PersistedClientPairing) -> PairedClientRecord {
    let terminal_type = terminal_type_from_str(&client.terminal_type);
    PairedClientRecord {
        client_id: client.client_id,
        alias: client.alias,
        terminal_type: Some(terminal_type),
        public_key_thumbprint: client.public_key_thumbprint,
        paired_at_ms: client.paired_at_ms,
        revoked: client.revoked,
    }
}

fn paired_terminal_records() -> Vec<TerminalRecord> {
    crate::config::DaemonConfig::client_pairing_entries()
        .into_iter()
        .map(terminal_record)
        .collect()
}

fn terminal_record(client: crate::config::PersistedClientPairing) -> TerminalRecord {
    TerminalRecord {
        terminal_id: client.client_id,
        terminal_type: terminal_type_from_str(&client.terminal_type),
        alias: client.alias,
        paired_at_ms: client.paired_at_ms,
        revoked: client.revoked,
    }
}

fn terminal_type_from_str(value: &str) -> TerminalType {
    match value.trim().to_ascii_lowercase().as_str() {
        "web" | "web_terminal" | "web-terminal" => TerminalType::Web,
        "ios" | "ios_terminal" | "ios-terminal" => TerminalType::Ios,
        "android" | "android_terminal" | "android-terminal" => TerminalType::Android,
        _ => TerminalType::Cli,
    }
}

fn command_caller_user_id(command: &KernelCommand) -> String {
    command
        .caller
        .user_id
        .clone()
        .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string())
}

fn ensure_provider_run_visible_to_user(
    provider_run: &crate::provider::RuntimeProviderRun,
    caller_user_id: &str,
) -> Result<(), DaemonError> {
    if provider_run.owned_by(caller_user_id) {
        Ok(())
    } else {
        Err(DaemonError::OwnershipAccessDenied {
            user_id: caller_user_id.to_string(),
            owner_user_id: provider_run.owner_user_id().to_string(),
            resource: format!("provider run `{}`", provider_run.id()),
            operation: "read provider run",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionInviteToken {
    version: u8,
    session_id: String,
    invite_id: String,
    created_by_user_id: String,
    issued_at_ms: u64,
    #[serde(default)]
    expires_at_ms: Option<u64>,
    #[serde(default)]
    max_uses: Option<u32>,
}

fn encode_session_invite_token(token: &SessionInviteToken) -> Result<String, DaemonError> {
    let payload = serde_json::to_vec(token).map_err(|error| DaemonError::LocalTransport {
        operation: "encode session invite",
        message: error.to_string(),
    })?;
    Ok(format!(
        "arroba-session-invite-v1.{}",
        URL_SAFE_NO_PAD.encode(payload)
    ))
}

fn decode_session_invite_token(token: &str) -> Result<SessionInviteToken, DaemonError> {
    let payload = token
        .trim()
        .strip_prefix("arroba-session-invite-v1.")
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "decode session invite",
            message: "session invite token has an unsupported format".to_string(),
        })?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "decode session invite",
            message: error.to_string(),
        })?;
    let decoded = serde_json::from_slice::<SessionInviteToken>(&bytes).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "decode session invite",
            message: error.to_string(),
        }
    })?;
    if decoded.version != 1 {
        return Err(DaemonError::LocalTransport {
            operation: "decode session invite",
            message: format!("unsupported session invite version {}", decoded.version),
        });
    }
    Ok(decoded)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PairingInviteToken {
    version: u8,
    intent: PairingInviteIntent,
    invite_id: String,
    relay_url: String,
    relay_token: String,
    target_daemon_id: String,
    #[serde(default)]
    target_daemon_alias: Option<String>,
    issuer_machine_id: String,
    issued_at_ms: u64,
    expires_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pairing_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_id: Option<String>,
}

fn encode_pairing_invite_token(token: &PairingInviteToken) -> Result<String, DaemonError> {
    encode_pairing_invite_token_with_prefix("arroba-invite-v1", token)
}

fn encode_terminal_pairing_link(token: &PairingInviteToken) -> Result<String, DaemonError> {
    encode_pairing_invite_token_with_prefix("arroba-terminal-pair-v1", token)
}

fn encode_pairing_invite_token_with_prefix(
    prefix: &str,
    token: &PairingInviteToken,
) -> Result<String, DaemonError> {
    let payload = serde_json::to_vec(token).map_err(|error| DaemonError::LocalTransport {
        operation: "encode pairing invite",
        message: error.to_string(),
    })?;
    Ok(format!("{prefix}.{}", URL_SAFE_NO_PAD.encode(payload)))
}

fn decode_pairing_invite_token(token: &str) -> Result<PairingInviteToken, DaemonError> {
    let trimmed = token.trim();
    let payload = trimmed
        .strip_prefix("arroba-invite-v1.")
        .or_else(|| trimmed.strip_prefix("arroba-terminal-pair-v1."))
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "decode pairing invite",
            message: "pairing invite token has an unsupported format".to_string(),
        })?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "decode pairing invite",
            message: error.to_string(),
        })?;
    let decoded = serde_json::from_slice::<PairingInviteToken>(&bytes).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "decode pairing invite",
            message: error.to_string(),
        }
    })?;
    if decoded.version != 1 {
        return Err(DaemonError::LocalTransport {
            operation: "decode pairing invite",
            message: format!("unsupported pairing invite version {}", decoded.version),
        });
    }
    Ok(decoded)
}

fn random_hex_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn random_pairing_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    let value: String = bytes
        .iter()
        .map(|byte| ALPHABET[(*byte as usize) % ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &value[..4], &value[4..])
}

fn public_key_thumbprint(public_key: &str) -> String {
    let digest = Sha256::digest(public_key.as_bytes());
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn projected_workflow_id(
    session: &crate::session::RuntimeSession,
    workflow_ref: Option<&str>,
) -> Result<Option<String>, DaemonError> {
    workflow_ref
        .map(|reference| projected_resolve_workflow(session, reference))
        .transpose()
        .map(|workflow| workflow.map(|workflow| workflow.id().to_string()))
}

fn projected_resolve_workflow(
    session: &crate::session::RuntimeSession,
    workflow_ref: &str,
) -> Result<crate::session::WorkflowDefinition, DaemonError> {
    let normalized_ref = workflow_ref.trim().to_lowercase();
    if let Some(workflow) = session
        .workflows()
        .iter()
        .find(|workflow| workflow.id() == normalized_ref)
    {
        return Ok(workflow.clone());
    }
    if let Some(workflow) = session
        .workflows()
        .iter()
        .find(|workflow| workflow.alias() == Some(normalized_ref.as_str()))
    {
        return Ok(workflow.clone());
    }
    let id_matches = session
        .workflows()
        .iter()
        .filter(|workflow| workflow.id().starts_with(&normalized_ref))
        .cloned()
        .collect::<Vec<_>>();
    if id_matches.len() == 1 {
        return Ok(id_matches[0].clone());
    }
    let alias_matches = session
        .workflows()
        .iter()
        .filter(|workflow| {
            workflow
                .alias()
                .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
        })
        .cloned()
        .collect::<Vec<_>>();
    if alias_matches.len() == 1 {
        return Ok(alias_matches[0].clone());
    }
    Err(DaemonError::WorkflowNotFound {
        session_id: session.id().to_string(),
        workflow_id: workflow_ref.to_string(),
    })
}

fn projected_resolve_workflow_run(
    session: &crate::session::RuntimeSession,
    workflow_run_ref: &str,
) -> Result<crate::session::WorkflowRun, DaemonError> {
    let normalized_ref = workflow_run_ref.trim().to_lowercase();
    if let Some(workflow_run) = session
        .workflow_runs()
        .iter()
        .find(|workflow_run| workflow_run.id() == normalized_ref)
    {
        return Ok(workflow_run.clone());
    }
    let id_matches = session
        .workflow_runs()
        .iter()
        .filter(|workflow_run| workflow_run.id().starts_with(&normalized_ref))
        .cloned()
        .collect::<Vec<_>>();
    if id_matches.len() == 1 {
        return Ok(id_matches[0].clone());
    }
    Err(DaemonError::WorkflowRunNotFound {
        session_id: session.id().to_string(),
        workflow_run_id: workflow_run_ref.to_string(),
    })
}

fn router_projection_stores(
    app: &Arc<Mutex<DaemonApp>>,
) -> (
    SessionHistoryStore,
    OperationalHistoryStore,
    SessionStateProjectionStore,
    SessionHistoryProjectionStore,
    ProviderCatalogProjectionStore,
    ProviderRunProjectionStore,
    ProviderProcessProjectionStore,
    RemoteRelayInventoryProjectionStore,
    AgentRuntimeProjectionStore,
    DaemonConfigProjectionStore,
    crate::session::SessionStateStore,
    crate::agent::AgentServiceStore,
    crate::attachment::AttachmentServiceStore,
    crate::provider::ProviderProcessServiceStore,
    crate::app::ProviderProcessTrackingStore,
    crate::app::PromptActivityStore,
    std::time::Duration,
    crate::app::PromptWorkspaceClaimStore,
    crate::app::provider_output::StructuredOutputRecordStore,
    crate::durable_state::DurableKernelStateStore,
    Arc<RwLock<RelayClientState>>,
    TerminalStreamHealthStore,
    TerminalStreamStore,
    WorkspaceCoordinator,
    PromptStateOwner,
    PromptIdAllocator,
) {
    let started = std::time::Instant::now();
    let app = loop {
        if let Ok(app) = app.try_lock() {
            break app;
        }
        if started.elapsed() >= Duration::from_secs(5) {
            panic!("CommandRouter could not acquire the app lock during bootstrap");
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    (
        app.history_store(),
        app.operational_history_store(),
        app.session_state_projection_store(),
        app.session_history_projection_store(),
        app.provider_catalog_projection_store(),
        app.provider_run_projection_store(),
        app.provider_process_projection_store(),
        app.remote_relay_inventory_projection_store(),
        app.agent_runtime_projection_store(),
        app.config_projection_store(),
        app.session_state_store(),
        app.agents().clone(),
        app.attachments().clone(),
        app.providers().clone(),
        app.provider_process_tracking_store(),
        app.prompt_activity_store(),
        app.prompt_idle_timeout(),
        app.prompt_workspace_claim_store(),
        app.structured_output_record_store(),
        app.durable_state_store(),
        app.relay_client_state(),
        app.terminal_health_store(),
        app.terminal_stream_store(),
        app.workspace_coordinator(),
        app.prompt_state_owner(),
        app.prompt_id_allocator(),
    )
}

#[derive(Debug)]
enum FocusProjectionRefresh {
    None,
    AgentSpawn,
    SnapshotSession { session_id: String },
}

fn mcp_registry_roots(workspace_id: Option<&str>) -> Result<Vec<std::path::PathBuf>, DaemonError> {
    let workspace = registry_workspace_root(workspace_id)?;
    let mut roots = vec![crate::mcp::ArrobaMcpRegistry::project_root(&workspace)];
    if let Some(root) = crate::mcp::ArrobaMcpRegistry::user_root() {
        roots.push(root);
    }
    Ok(roots)
}

fn skill_registry_roots(
    workspace_id: Option<&str>,
) -> Result<Vec<std::path::PathBuf>, DaemonError> {
    let workspace = registry_workspace_root(workspace_id)?;
    let mut roots = vec![crate::skill::ArrobaSkillRegistry::project_root(&workspace)];
    if let Some(root) = crate::skill::ArrobaSkillRegistry::user_root() {
        roots.push(root);
    }
    Ok(roots)
}

fn registry_workspace_root(workspace_id: Option<&str>) -> Result<std::path::PathBuf, DaemonError> {
    match workspace_id {
        Some(value) if !value.trim().is_empty() => Ok(std::path::PathBuf::from(value)),
        _ => std::env::current_dir().map_err(|error| DaemonError::LocalTransport {
            operation: "registry.roots",
            message: format!("failed to resolve current directory: {error}"),
        }),
    }
}

#[derive(Debug, Clone)]
enum SessionMembershipScope {
    AllSessions,
    SessionId(String),
    SessionRef {
        session_ref: String,
        workspace_id: Option<String>,
    },
    AttachmentId(String),
}

fn command_session_user_id(command: &KernelCommand) -> Option<String> {
    match command.caller.caller_kind {
        KernelCallerKind::LocalClient => command
            .caller
            .user_id
            .clone()
            .or_else(|| Some(DEFAULT_LOCAL_USER_ID.to_string())),
        KernelCallerKind::RemoteClient
        | KernelCallerKind::RemoteKernel
        | KernelCallerKind::HostedService => command.caller.user_id.clone(),
    }
}

fn is_implicit_local_session_caller(command: &KernelCommand) -> bool {
    matches!(command.caller.caller_kind, KernelCallerKind::LocalClient)
        && command.caller.user_id.is_none()
}

fn request_session_scope(request: &LocalDaemonRequest) -> Option<SessionMembershipScope> {
    match request {
        LocalDaemonRequest::ListSessions(_) => Some(SessionMembershipScope::AllSessions),
        LocalDaemonRequest::ResolveSession(request) => Some(SessionMembershipScope::SessionRef {
            session_ref: request.session_ref.clone(),
            workspace_id: request.workspace_id.clone(),
        }),
        LocalDaemonRequest::DeleteSession(request) => Some(SessionMembershipScope::SessionRef {
            session_ref: request.session_ref.clone(),
            workspace_id: request.workspace_id.clone(),
        }),
        LocalDaemonRequest::DetachFromSession(request) => Some(
            SessionMembershipScope::AttachmentId(request.attachment_id.clone()),
        ),
        LocalDaemonRequest::QueryHistory(request) => request
            .session_id
            .as_ref()
            .map(|session_id| SessionMembershipScope::SessionId(session_id.clone())),
        LocalDaemonRequest::SearchHistory(request) => request
            .session_id
            .as_ref()
            .map(|session_id| SessionMembershipScope::SessionId(session_id.clone())),
        LocalDaemonRequest::AttachToSession(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::LaunchProviderRun(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ListSessionMembers(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CreateSessionInvite(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RevokeSessionInvite(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CreateWorkspaceLink(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkspaceLinks(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ShowWorkspaceLink(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::AttachWorkspaceLink(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::DetachWorkspaceLink(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SubmitPrompt(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CompletePrompt(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CancelActivePrompt(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::UpdateSessionConfig(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::UpdateAgentConfig(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::UpdateAgentProfile(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::AliasAgent(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::UpdateAgentSubstitutes(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::GetSessionState(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::AliasSession(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::GetSessionHistory(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::GetPromptInputHistory(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RecordPromptInputHistory(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::PollRuntimeNotices(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ResizeTerminal(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::PumpTerminalOutput(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::EndSession(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::RunShellCommand(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ReadDirectoryTree(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ReadFile(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::EditFile(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::InspectGit(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CaptureScreenshot(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::StoreTransferredFile(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SpawnAgent(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::MoveAgentToRemote(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::DestroyAgent(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::FocusAgent(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CycleAgentFocus(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ListAgents(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CreateWorkflow(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::AliasWorkflow(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ListWorkflows(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ResolveWorkflow(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CreateWorkflowPublication(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowPublications(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::GetWorkflowPublication(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::DisableWorkflowPublication(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CreateWorkflowPublicationPairCode(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RedeemWorkflowPublicationPairCode(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowPublicationSenders(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RevokeWorkflowPublicationSender(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AuthenticateWorkflowPublicationSender(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::CreateWorkflowEndpoint(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AliasWorkflowEndpoint(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::BindWorkflowEndpoint(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AddWorkflowNode(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::RemoveWorkflowNode(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AddWorkflowEdge(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ValidateWorkflowOutput(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::AckWorkflowTurn(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::RemoveWorkflowEdge(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::InvokeWorkflowEndpoint(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowRuns(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::GetWorkflowRun(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CancelWorkflowRun(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::ResumeWorkflowRun(request) => Some(SessionMembershipScope::SessionId(
            request.session_id.clone(),
        )),
        LocalDaemonRequest::CreateWorkflowWatchdog(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListWorkflowWatchdogs(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RemoveWorkflowWatchdog(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowFlushContext(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::SetWorkflowLaunchPolicy(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::RemoveQueuedWorkflowLaunch(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        LocalDaemonRequest::ClearQueuedWorkflowLaunches(request) => Some(
            SessionMembershipScope::SessionId(request.session_id.clone()),
        ),
        _ => None,
    }
}

fn focus_projection_refresh(request: &LocalDaemonRequest) -> FocusProjectionRefresh {
    match request {
        LocalDaemonRequest::SpawnAgent(_) => FocusProjectionRefresh::AgentSpawn,
        LocalDaemonRequest::AliasAgent(request) => FocusProjectionRefresh::SnapshotSession {
            session_id: request.session_id.clone(),
        },
        LocalDaemonRequest::UpdateAgentConfig(request) => FocusProjectionRefresh::SnapshotSession {
            session_id: request.session_id.clone(),
        },
        LocalDaemonRequest::UpdateAgentProfile(request) => {
            FocusProjectionRefresh::SnapshotSession {
                session_id: request.session_id.clone(),
            }
        }
        LocalDaemonRequest::UpdateAgentSubstitutes(request) => {
            FocusProjectionRefresh::SnapshotSession {
                session_id: request.session_id.clone(),
            }
        }
        LocalDaemonRequest::DestroyAgent(request) => FocusProjectionRefresh::SnapshotSession {
            session_id: request.session_id.clone(),
        },
        _ => FocusProjectionRefresh::None,
    }
}

#[derive(Debug)]
enum SessionProjectionRefresh {
    None,
    SnapshotAgentResponse,
}

impl SessionProjectionRefresh {
    fn session_ids(&self, response: &LocalDaemonResponse) -> Vec<String> {
        match self {
            SessionProjectionRefresh::None => Vec::new(),
            SessionProjectionRefresh::SnapshotAgentResponse => match response {
                LocalDaemonResponse::AgentSpawned { agent }
                | LocalDaemonResponse::AgentAliased { agent, .. }
                | LocalDaemonResponse::AgentConfigUpdated { agent, .. }
                | LocalDaemonResponse::AgentProfileUpdated { agent, .. }
                | LocalDaemonResponse::AgentDestroyed { agent }
                | LocalDaemonResponse::AgentFocused { agent } => {
                    vec![agent.session_id().to_string()]
                }
                LocalDaemonResponse::AgentFocusCycled { agent: Some(agent) } => {
                    vec![agent.session_id().to_string()]
                }
                _ => Vec::new(),
            },
        }
    }
}

fn prompt_input_history_entry_from_event(
    event: crate::history::HistoryEvent,
) -> Option<PromptInputHistoryEntry> {
    let session_id = event.session_id.clone()?;
    let kind = match event.kind {
        HistoryEventKind::UserPrompt => PromptInputHistoryEntryKind::Prompt,
        HistoryEventKind::PromptInput => match event
            .metadata
            .get("input_kind")
            .and_then(|value| value.as_str())
        {
            Some("command") => PromptInputHistoryEntryKind::Command,
            _ => PromptInputHistoryEntryKind::Prompt,
        },
        _ => return None,
    };
    Some(PromptInputHistoryEntry {
        sequence: event.sequence,
        timestamp_ms: event.timestamp_ms,
        session_id,
        source_attachment_id: event
            .metadata
            .get("source_attachment_id")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        kind,
        text: event.content.unwrap_or_default(),
    })
}

fn prompt_input_history_events_for_kind(
    history: &OperationalHistoryStore,
    session_id: &str,
    kind: &str,
    after_sequence: Option<u64>,
    limit: usize,
) -> Result<Vec<crate::history::HistoryEvent>, DaemonError> {
    let mut events = Vec::new();
    let mut cursor = after_sequence;
    while events.len() < limit {
        let batch = history.query_events(HistoryEventQuery {
            session_id: Some(session_id.to_string()),
            kind: Some(kind.to_string()),
            after_sequence: cursor,
            limit: Some((limit - events.len()).min(500)),
            ..HistoryEventQuery::default()
        })?;
        let Some(last_sequence) = batch.last().map(|event| event.sequence) else {
            break;
        };
        let batch_len = batch.len();
        events.extend(batch);
        cursor = Some(last_sequence);
        if batch_len < 500 {
            break;
        }
    }
    Ok(events)
}

fn history_query_from_request(request: QueryHistoryRequest) -> HistoryEventQuery {
    HistoryEventQuery {
        session_id: request.session_id,
        agent_id: request.agent_id,
        provider: request.provider,
        model: request.model,
        workflow_id: request.workflow_id,
        machine_id: request.machine_id,
        repo_root: request.repo_root,
        worktree_path: request.worktree_path,
        kind: request.kind,
        text: request.text,
        after_sequence: request.after_sequence,
        limit: request.limit,
    }
}

fn history_query_from_search_request(request: SearchHistoryRequest) -> HistoryEventQuery {
    HistoryEventQuery {
        session_id: request.session_id,
        agent_id: request.agent_id,
        provider: request.provider,
        model: request.model,
        workflow_id: request.workflow_id,
        machine_id: request.machine_id,
        repo_root: request.repo_root,
        worktree_path: request.worktree_path,
        kind: request.kind,
        text: Some(request.query),
        after_sequence: request.after_sequence,
        limit: request.limit,
    }
}

fn merge_history_events(
    events: &mut Vec<crate::history::HistoryEvent>,
    archive_events: Vec<crate::history::HistoryEvent>,
) {
    let mut seen = events
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<HashSet<_>>();
    for event in archive_events {
        if seen.insert(event.event_id.clone()) {
            events.push(event);
        }
    }
}

fn session_projection_refresh(request: &LocalDaemonRequest) -> SessionProjectionRefresh {
    match request {
        LocalDaemonRequest::AttachToSession(_)
        | LocalDaemonRequest::DetachFromSession(_)
        | LocalDaemonRequest::FocusAgent(_)
        | LocalDaemonRequest::CycleAgentFocus(_) => SessionProjectionRefresh::None,
        LocalDaemonRequest::SpawnAgent(_)
        | LocalDaemonRequest::AliasAgent(_)
        | LocalDaemonRequest::UpdateAgentConfig(_)
        | LocalDaemonRequest::UpdateAgentProfile(_)
        | LocalDaemonRequest::UpdateAgentSubstitutes(_)
        | LocalDaemonRequest::DestroyAgent(_) => SessionProjectionRefresh::SnapshotAgentResponse,
        LocalDaemonRequest::CompletePrompt(_) | LocalDaemonRequest::CancelActivePrompt(_) => {
            SessionProjectionRefresh::None
        }
        LocalDaemonRequest::PumpTerminalOutput(_) => SessionProjectionRefresh::None,
        LocalDaemonRequest::PollRuntimeNotices(_) | LocalDaemonRequest::ResizeTerminal(_) => {
            SessionProjectionRefresh::None
        }
        _ => SessionProjectionRefresh::None,
    }
}

fn response_sessions(response: &LocalDaemonResponse) -> Vec<crate::session::RuntimeSession> {
    match response {
        LocalDaemonResponse::SessionCreated { session, .. }
        | LocalDaemonResponse::SessionResolved { session }
        | LocalDaemonResponse::SessionState { session }
        | LocalDaemonResponse::InteractionResponded { session, .. }
        | LocalDaemonResponse::SessionConfigUpdated { session, .. }
        | LocalDaemonResponse::AgentAliased { session, .. }
        | LocalDaemonResponse::AgentConfigUpdated { session, .. }
        | LocalDaemonResponse::AgentProfileUpdated { session, .. }
        | LocalDaemonResponse::SessionEnded { session }
        | LocalDaemonResponse::SessionAliased { session }
        | LocalDaemonResponse::WorkspaceLinkCreated { session, .. }
        | LocalDaemonResponse::WorkspaceLinkAttached { session, .. }
        | LocalDaemonResponse::WorkspaceLinkDetached { session, .. }
        | LocalDaemonResponse::WorkflowCreated { session, .. }
        | LocalDaemonResponse::WorkflowAliased { session, .. }
        | LocalDaemonResponse::WorkflowEndpointCreated { session, .. }
        | LocalDaemonResponse::WorkflowEndpointAliased { session, .. }
        | LocalDaemonResponse::WorkflowEndpointBound { session, .. }
        | LocalDaemonResponse::WorkflowNodeAdded { session, .. }
        | LocalDaemonResponse::WorkflowNodeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowNodeInstructionsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowEdgeAdded { session, .. }
        | LocalDaemonResponse::WorkflowEdgeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowRunInvoked { session, .. }
        | LocalDaemonResponse::WorkflowRunQueued { session, .. }
        | LocalDaemonResponse::WorkflowRunCancelled { session, .. }
        | LocalDaemonResponse::WorkflowRunResumed { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogCreated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogUpdated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogRemoved { session, .. }
        | LocalDaemonResponse::WorkflowFlushContextUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session, .. }
        | LocalDaemonResponse::QueuedWorkflowLaunchRemoved { session, .. }
        | LocalDaemonResponse::QueuedWorkflowLaunchesCleared { session, .. }
        | LocalDaemonResponse::WorkflowTurnAcknowledged { session, .. } => vec![session.clone()],
        _ => Vec::new(),
    }
}

fn should_update_agent_runtime_projection_from_response(response: &LocalDaemonResponse) -> bool {
    !matches!(response, LocalDaemonResponse::PromptSubmitted { .. })
}

fn response_removed_session_ids(response: &LocalDaemonResponse) -> Vec<&str> {
    match response {
        LocalDaemonResponse::SessionDeleted { session } => vec![session.id()],
        LocalDaemonResponse::KernelDeleted {
            deleted_sessions, ..
        } => deleted_sessions
            .iter()
            .map(|session| session.id())
            .collect(),
        _ => Vec::new(),
    }
}

async fn execute_list_provider_processes_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: ListProviderProcessesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (processes, delay_ms) = {
        let app = app.lock().await;
        (
            app.list_provider_processes(request.provider.as_deref())?,
            app.config().provider_process_list_delay_ms,
        )
    };
    if delay_ms > 0 {
        sleep(Duration::from_millis(delay_ms)).await;
    }
    Ok(LocalDaemonResponse::ProviderProcessesListed { processes })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudDeviceStartResponse {
    device_code: String,
    user_code: String,
    verification_url: String,
    expires_at: String,
    interval_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudDevicePollResponse {
    status: String,
    interval_seconds: Option<u64>,
    expires_at: Option<String>,
    profile: Option<CloudDeviceProfileResponse>,
    cloud_session_token: Option<String>,
    cloud_session_expires_at: Option<String>,
    machine_credential: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudDeviceProfileResponse {
    email: String,
    account_id: String,
    user_id: String,
    account_slug: String,
    realm_id: String,
    relay_url: String,
    issuer_id: String,
    client_id: Option<String>,
    client_alias: Option<String>,
    machine_id: Option<String>,
    machine_alias: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudPairingTokenResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudRuntimeTokenResponse {
    token: String,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionInviteResponse {
    invite_id: String,
    invite_token: String,
    session_id: String,
    account_id: String,
    created_by_user_id: String,
    expires_at: Option<String>,
    max_uses: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionInviteDetailsResponse {
    invite_id: String,
    session_id: String,
    account_id: String,
    created_by_user_id: String,
    display_name: Option<String>,
    expires_at: Option<String>,
    max_uses: Option<u32>,
    used_count: u32,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionInviteAcceptanceResponse {
    session_id: String,
    account_id: String,
    user_id: String,
    invited_by_user_id: String,
    joined_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionInviteRevokedResponse {
    invite_id: String,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionMembersResponse {
    session_id: String,
    members: Vec<CloudSessionMemberResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudSessionMemberResponse {
    user_id: String,
    email: String,
    display_name: Option<String>,
    invited_by_user_id: Option<String>,
    joined_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudCollaboratorsResponse {
    collaborators: Vec<CloudCollaboratorResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudCollaboratorResponse {
    user_id: String,
    email: String,
    display_name: Option<String>,
    last_collaborated_at: String,
    shared_session_count: u32,
}

fn cloud_session_invite_from_response(response: CloudSessionInviteResponse) -> CloudSessionInvite {
    CloudSessionInvite {
        invite_id: response.invite_id,
        invite_token: response.invite_token,
        session_id: response.session_id,
        account_id: response.account_id,
        created_by_user_id: response.created_by_user_id,
        expires_at: response.expires_at,
        max_uses: response.max_uses,
    }
}

fn cloud_session_invite_details_from_response(
    response: CloudSessionInviteDetailsResponse,
) -> CloudSessionInviteDetails {
    CloudSessionInviteDetails {
        invite_id: response.invite_id,
        session_id: response.session_id,
        account_id: response.account_id,
        created_by_user_id: response.created_by_user_id,
        display_name: response.display_name,
        expires_at: response.expires_at,
        max_uses: response.max_uses,
        used_count: response.used_count,
        status: response.status,
    }
}

fn cloud_session_invite_acceptance_from_response(
    response: CloudSessionInviteAcceptanceResponse,
) -> CloudSessionInviteAcceptance {
    CloudSessionInviteAcceptance {
        session_id: response.session_id,
        account_id: response.account_id,
        user_id: response.user_id,
        invited_by_user_id: response.invited_by_user_id,
        joined_at: response.joined_at,
    }
}

fn cloud_session_member_from_response(response: CloudSessionMemberResponse) -> CloudSessionMember {
    CloudSessionMember {
        user_id: response.user_id,
        email: response.email,
        display_name: response.display_name,
        invited_by_user_id: response.invited_by_user_id,
        joined_at: response.joined_at,
    }
}

fn cloud_collaborator_from_response(response: CloudCollaboratorResponse) -> CloudCollaborator {
    CloudCollaborator {
        user_id: response.user_id,
        email: response.email,
        display_name: response.display_name,
        last_collaborated_at: response.last_collaborated_at,
        shared_session_count: response.shared_session_count,
    }
}

async fn issue_cloud_runtime_token(
    profile: &PersistedCloudRelayProfile,
    subject: &str,
    subject_kind: &str,
    allowed_targets: Option<Vec<String>>,
    client_id: Option<String>,
    machine_id: Option<String>,
    session_id: Option<String>,
) -> Result<CloudRuntimeTokenResponse, DaemonError> {
    let mut body = serde_json::Map::new();
    if let Some(machine_credential) = profile.machine_credential.clone() {
        body.insert(
            "machineCredential".to_string(),
            serde_json::Value::String(machine_credential),
        );
    } else if let Some(session_token) = profile.cloud_session_token.clone() {
        body.insert(
            "sessionToken".to_string(),
            serde_json::Value::String(session_token),
        );
    }
    body.insert(
        "accountId".to_string(),
        serde_json::Value::String(profile.account_id.clone()),
    );
    body.insert(
        "subject".to_string(),
        serde_json::Value::String(subject.to_string()),
    );
    body.insert(
        "subjectKind".to_string(),
        serde_json::Value::String(subject_kind.to_string()),
    );
    body.insert(
        "realmId".to_string(),
        serde_json::Value::String(profile.realm_id.clone()),
    );
    body.insert(
        "ttlMs".to_string(),
        serde_json::Value::Number(serde_json::Number::from(CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS)),
    );
    body.insert(
        "userId".to_string(),
        serde_json::Value::String(profile.user_id.clone()),
    );
    if let Some(allowed_targets) = allowed_targets {
        body.insert(
            "allowedTargets".to_string(),
            serde_json::to_value(allowed_targets).map_err(|error| DaemonError::LocalTransport {
                operation: "encode cloud relay token request",
                message: error.to_string(),
            })?,
        );
    }
    if let Some(client_id) = client_id {
        body.insert("clientId".to_string(), serde_json::Value::String(client_id));
    }
    if let Some(machine_id) = machine_id {
        body.insert(
            "machineId".to_string(),
            serde_json::Value::String(machine_id),
        );
    }
    if let Some(session_id) = session_id {
        body.insert(
            "sessionId".to_string(),
            serde_json::Value::String(session_id),
        );
    }
    post_cloud_json(
        profile.api_url.clone(),
        "/relay/token",
        serde_json::Value::Object(body),
    )
    .await
}

fn cloud_profile_from_persisted(profile: &PersistedCloudRelayProfile) -> CloudRelayProfile {
    CloudRelayProfile {
        api_url: profile.api_url.clone(),
        email: profile.email.clone(),
        account_id: profile.account_id.clone(),
        user_id: profile.user_id.clone(),
        account_slug: profile.account_slug.clone(),
        realm_id: profile.realm_id.clone(),
        relay_url: profile.relay_url.clone(),
        issuer_id: profile.issuer_id.clone(),
        client_id: profile.client_id.clone(),
        client_alias: profile.client_alias.clone(),
        machine_id: profile.machine_id.clone(),
        machine_alias: profile.machine_alias.clone(),
        machine_credential: profile.machine_credential.clone(),
        cloud_session_token: profile.cloud_session_token.clone(),
        cloud_session_expires_at_ms: profile.cloud_session_expires_at_ms,
        token_expires_at_ms: profile.token_expires_at_ms,
    }
}

fn normalize_cloud_api_url(api_url: &str) -> Result<String, DaemonError> {
    let normalized = api_url.trim().trim_end_matches('/').to_string();
    if normalized.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "normalize cloud relay api url",
            message: "api_url must not be empty".to_string(),
        });
    }
    Ok(normalized)
}

async fn post_cloud_json<T>(
    api_url: String,
    path: &'static str,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || post_cloud_json_blocking(api_url, path, body))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "post cloud relay json",
            message: error.to_string(),
        })?
}

async fn post_cloud_json_dynamic<T>(
    api_url: String,
    path: String,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || post_cloud_json_blocking(api_url, &path, body))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "post cloud relay json",
            message: error.to_string(),
        })?
}

async fn get_cloud_json<T>(api_url: String, path: String) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || get_cloud_json_blocking(api_url, &path))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "get cloud relay json",
            message: error.to_string(),
        })?
}

fn post_cloud_json_blocking<T>(
    api_url: String,
    path: &str,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let response = ureq::post(&url)
        .set("content-type", "application/json")
        .send_string(&body.to_string())
        .map_err(|error| cloud_transport_error(error))?;
    let payload = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read cloud relay response",
            message: error.to_string(),
        })?;
    serde_json::from_str::<T>(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "decode cloud relay response",
        message: error.to_string(),
    })
}

fn get_cloud_json_blocking<T>(api_url: String, path: &str) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let response = ureq::get(&url)
        .call()
        .map_err(|error| cloud_transport_error(error))?;
    let payload = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read cloud relay response",
            message: error.to_string(),
        })?;
    serde_json::from_str::<T>(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "decode cloud relay response",
        message: error.to_string(),
    })
}

fn cloud_url_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

fn cloud_transport_error(error: ureq::Error) -> DaemonError {
    let message = match error {
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            if body.is_empty() {
                format!("cloud relay request failed with {status}")
            } else if let Some(code) = cloud_api_error_code(&body) {
                format!("cloud relay request failed with {status}: cloud_api_code={code}: {body}")
            } else {
                format!("cloud relay request failed with {status}: {body}")
            }
        }
        ureq::Error::Transport(error) => error.to_string(),
    };
    DaemonError::LocalTransport {
        operation: "cloud relay request",
        message,
    }
}

fn cloud_api_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|payload| {
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(|code| code.as_str())
                .map(str::to_string)
        })
}

fn is_stale_cloud_link_error(error: &DaemonError) -> bool {
    let message = match error {
        DaemonError::LocalTransport { message, .. } => message.as_str(),
        _ => return false,
    };
    [
        "cloud_api_code=session_invalid",
        "cloud_api_code=identity_revoked",
        "cloud_api_code=realm_not_found",
        "cloud_api_code=account_deleted",
        "cloud_api_code=user_deleted",
        "\"code\":\"session_invalid\"",
        "\"code\":\"identity_revoked\"",
        "\"code\":\"realm_not_found\"",
        "\"code\":\"account_deleted\"",
        "\"code\":\"user_deleted\"",
        "invalid_session",
        "cloud relay request failed with 401",
        "cloud relay request failed with 403",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tokio::time::{Duration, timeout};

    use crate::agent::CreateAgentRequest;
    use crate::attachment::ClientCapabilityLevel;
    use crate::local::{
        AddWorkflowEdgeRequest, AddWorkflowNodeRequest, AliasSessionRequest,
        AttachToSessionRequest, CancelActivePromptRequest, CompletePromptRequest,
        CreateWorkflowEndpointRequest, CreateWorkflowRequest, CycleAgentFocusRequest,
        DeleteKernelRequest, DeleteSessionRequest, DestroyAgentRequest, DetachFromSessionRequest,
        EndSessionRequest, FocusAgentRequest, GetDaemonHealthRequest, GetProviderAuthStatusRequest,
        GetProviderCatalogRequest, GetProviderCommandCatalogsRequest, GetProviderRunRequest,
        GetSessionHistoryRequest, GetSessionStateRequest, InvokeWorkflowEndpointRequest,
        LaunchProviderRunRequest, ListAgentsRequest, ListProviderProcessesRequest,
        ListSessionsRequest, ListWorkflowRunsRequest, ListWorkflowWatchdogsRequest,
        ListWorkflowsRequest, LocalDaemonRequest, LocalDaemonResponse, PollRuntimeNoticesRequest,
        PumpTerminalOutputRequest, QueryHistoryRequest, RelayStatusRequest,
        RemoveWorkflowEdgeRequest, ResizeTerminalRequest, ResolveSessionRequest,
        ResolveWorkflowRequest, RunShellCapabilityRequest, SpawnAgentRequest, SubmitPromptRequest,
        TeardownProviderProcessesRequest, UpdateSessionConfigRequest,
    };
    use crate::provider::{
        LaunchProviderRequest, OpenCodeProviderCatalog, OpenCodeProviderInfo, RuntimeProviderRun,
    };
    use crate::runtime::command::{
        KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource,
    };
    use crate::runtime::router::CommandRouter;
    use crate::session::{
        CreateSessionRequest, DEFAULT_LOCAL_USER_ID, PromptStatus, PromptSubmissionOutcome,
        SessionStatus,
    };
    use crate::{DaemonApp, DaemonConfig, DaemonError};

    #[test]
    fn workspace_directory_completion_keeps_sibling_prefix_matches_for_existing_path() {
        let root = unique_test_dir("workspace-directory-completion-siblings");
        create_test_dir(root.join("arroba"));
        create_test_dir(root.join("arroba-cloud"));
        create_test_dir(root.join("arroba-feature"));
        create_test_dir(root.join(".arroba"));
        create_test_dir(root.join("bar-arroba"));

        let results =
            super::search_workspace_directories(&root.join("arroba").display().to_string(), 20)
                .expect("workspace directory search should succeed");
        remove_test_dir(&root);

        let exact = root.join("arroba").display().to_string();
        let cloud = root.join("arroba-cloud").display().to_string();
        let feature = root.join("arroba-feature").display().to_string();
        let hidden = root.join(".arroba").display().to_string();
        let contains = root.join("bar-arroba").display().to_string();

        assert!(results.contains(&exact), "missing exact match: {results:?}");
        assert!(
            results.contains(&cloud),
            "missing prefix sibling: {results:?}"
        );
        assert!(
            results.contains(&feature),
            "missing prefix sibling: {results:?}"
        );
        assert!(
            results.contains(&hidden),
            "missing hidden contains match: {results:?}"
        );
        assert!(
            results.contains(&contains),
            "missing contains match: {results:?}"
        );

        let exact_index = result_index(&results, &exact);
        assert!(exact_index < result_index(&results, &cloud));
        assert!(exact_index < result_index(&results, &feature));
        assert!(result_index(&results, &cloud) < result_index(&results, &hidden));
        assert!(result_index(&results, &feature) < result_index(&results, &hidden));
        assert!(result_index(&results, &contains) < result_index(&results, &hidden));
    }

    #[test]
    fn workspace_directory_completion_lists_children_only_after_trailing_separator() {
        let root = unique_test_dir("workspace-directory-completion-children");
        create_test_dir(root.join("arroba").join("child"));
        create_test_dir(root.join("arroba-cloud"));

        let query = format!("{}/", root.join("arroba").display());
        let results = super::search_workspace_directories(&query, 20)
            .expect("workspace directory search should succeed");
        remove_test_dir(&root);

        assert!(
            results.contains(&root.join("arroba").join("child").display().to_string()),
            "missing child directory: {results:?}",
        );
        assert!(
            !results.contains(&root.join("arroba-cloud").display().to_string()),
            "trailing slash should not include siblings: {results:?}",
        );
    }

    #[test]
    fn workspace_directory_completion_prioritizes_hidden_dirs_when_query_starts_hidden() {
        let root = unique_test_dir("workspace-directory-completion-hidden");
        create_test_dir(root.join(".arroba"));
        create_test_dir(root.join(".arroba-cache"));
        create_test_dir(root.join("my-.arroba"));

        let results =
            super::search_workspace_directories(&root.join(".arroba").display().to_string(), 20)
                .expect("workspace directory search should succeed");
        remove_test_dir(&root);

        let exact = root.join(".arroba").display().to_string();
        let hidden_prefix = root.join(".arroba-cache").display().to_string();
        let contains = root.join("my-.arroba").display().to_string();
        assert!(result_index(&results, &exact) < result_index(&results, &hidden_prefix));
        assert!(result_index(&results, &hidden_prefix) < result_index(&results, &contains));
    }

    fn unique_test_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("arroba-{label}-{}-{nanos}", std::process::id()))
    }

    fn create_test_dir(path: PathBuf) {
        fs::create_dir_all(path).expect("test directory should be created");
    }

    fn remove_test_dir(path: &PathBuf) {
        let _ = fs::remove_dir_all(path);
    }

    fn result_index(results: &[String], value: &str) -> usize {
        results
            .iter()
            .position(|result| result == value)
            .unwrap_or_else(|| panic!("missing {value} in {results:?}"))
    }

    fn spawn_test_agent(
        app: &mut DaemonApp,
        session_id: &str,
        alias: &str,
        provider: &str,
    ) -> crate::agent::AgentInstance {
        crate::app::KernelSessionService::new(app)
            .spawn_agent(CreateAgentRequest::new(session_id, provider).with_alias(alias))
            .expect("agent should spawn")
    }

    fn launch_test_provider(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
        adapter_key: &str,
        provider: &str,
        model: &str,
    ) -> RuntimeProviderRun {
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(session_id, adapter_key, provider, "default", model)
                    .with_agent_id(agent_id),
            )
            .expect("provider run should launch");
        app.update_provider_run_projection(provider_run.clone());
        provider_run
    }

    fn remote_command_for_request(
        request: &LocalDaemonRequest,
        user_id: Option<&str>,
    ) -> KernelCommand {
        KernelCommand::from_local_request_with_caller(
            "remote-command",
            KernelCommandSource::RelayClient,
            KernelCaller {
                caller_id: "client-remote".to_string(),
                caller_kind: KernelCallerKind::RemoteClient,
                user_id: user_id.map(str::to_string),
                client_id: Some("client-remote".to_string()),
                machine_id: None,
                realm_id: Some("realm-1".to_string()),
                public_key_thumbprint: Some("thumbprint-remote".to_string()),
            },
            None,
            None,
            request,
        )
    }

    fn focus_test_agent(app: &mut DaemonApp, session_id: &str, agent_id: &str) {
        crate::app::KernelSessionService::new(app)
            .focus_agent(session_id, agent_id)
            .expect("focus should succeed");
    }

    #[tokio::test]
    async fn pending_provider_launch_cleanup_does_not_wait_for_app_lock_when_projection_is_cold() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        router
            .pending_provider_launch_sessions
            .lock()
            .await
            .insert("cold-session".to_string());

        let app_guard = app.lock().await;
        let cleanup_router = router.clone();
        let cleanup_task = tokio::spawn(async move {
            cleanup_router
                .clear_provider_launch_pending_if_settled("cold-session")
                .await;
        });

        timeout(Duration::from_millis(100), cleanup_task)
            .await
            .expect("cold pending launch cleanup should not wait for the app lock")
            .expect("cleanup task should join");
        drop(app_guard);

        assert!(
            router
                .pending_provider_launch_sessions
                .lock()
                .await
                .contains("cold-session"),
            "cold cleanup should leave the guard for a later projection-backed refresh"
        );
    }

    #[tokio::test]
    async fn routes_interactive_commands_through_bounded_lane() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let request = LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "cli-1".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        });
        let command = KernelCommand::from_local_request("cmd-1", None, None, &request);

        let response = router
            .dispatch(command, request)
            .await
            .expect("command should run");

        assert!(matches!(
            response,
            crate::local::LocalDaemonResponse::SessionAttached { .. }
        ));
    }

    #[tokio::test]
    async fn runtime_agent_skill_grant_survives_kernel_restart() {
        let config = DaemonConfig::for_tests();
        let (session_id, agent_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
            let granted = router
                .runtime_state
                .grant_agent_skill(agent.id(), "review".to_string(), DEFAULT_LOCAL_USER_ID)
                .await
                .expect("skill grant should persist");
            assert!(granted.skill_grants().contains(&"review".to_string()));
            (session.id().to_string(), agent.id().to_string())
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_agent = app
            .agents
            .get_agent(&agent_id)
            .expect("agent should restore");
        assert_eq!(restored_agent.session_id(), session_id);
        assert!(
            restored_agent
                .skill_grants()
                .contains(&"review".to_string())
        );
    }

    #[tokio::test]
    async fn runtime_agent_capability_grants_accept_agent_id_or_public_ref() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let agent_id = agent.id().to_string();
        let agent_ref = agent.agent_ref().to_string();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);

        for (agent_ref, skill_name) in [(agent_id, "by-id"), (agent_ref, "by-ref")] {
            let agent = router
                .runtime_state
                .grant_agent_skill(&agent_ref, skill_name.to_string(), DEFAULT_LOCAL_USER_ID)
                .await
                .expect("grant should succeed");
            assert_eq!(agent.session_id(), session.id());
            assert!(agent.skill_grants().contains(&skill_name.to_string()));
        }
    }

    #[tokio::test]
    async fn workflow_definition_survives_kernel_restart() {
        let config = DaemonConfig::for_tests();
        let (session_id, agent_id, workflow_id) = {
            let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
            let (created, _) = router
                .runtime_state
                .execute_workflow_request(
                    LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                        session_id: session.id().to_string(),
                        alias: Some("review".to_string()),
                    }),
                    DEFAULT_LOCAL_USER_ID.to_string(),
                )
                .await;
            let workflow_id = match created.expect("workflow should create") {
                LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow.id().to_string(),
                other => panic!("unexpected response: {other:?}"),
            };
            let (added, _) = router
                .runtime_state
                .execute_workflow_request(
                    LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
                        session_id: session.id().to_string(),
                        workflow_ref: workflow_id.clone(),
                        agent_id: agent.id().to_string(),
                        expected_workflow_revision: None,
                    }),
                    DEFAULT_LOCAL_USER_ID.to_string(),
                )
                .await;
            added.expect("workflow node should add");
            (
                session.id().to_string(),
                agent.id().to_string(),
                workflow_id,
            )
        };

        let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
        let restored_session = app
            .sessions()
            .get_session(&session_id)
            .expect("session should restore");
        let workflow = restored_session
            .workflows()
            .iter()
            .find(|workflow| workflow.id() == workflow_id)
            .expect("workflow should restore");
        assert_eq!(workflow.alias(), Some("review"));
        assert_eq!(workflow.nodes().len(), 1);
        assert_eq!(workflow.nodes()[0].agent_id(), agent_id);
    }

    #[tokio::test]
    async fn runtime_end_and_delete_session_survive_kernel_restart() {
        let end_config = DaemonConfig::for_tests();
        let ended_session_id = {
            let mut app = DaemonApp::bootstrap(end_config.clone()).expect("daemon should boot");
            let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
            router
                .runtime_state
                .end_session(session.id())
                .await
                .expect("session should end");
            session.id().to_string()
        };
        let app = DaemonApp::bootstrap(end_config).expect("daemon should reboot");
        let restored = app
            .sessions()
            .get_session(&ended_session_id)
            .expect("ended session should restore");
        assert_eq!(restored.status(), SessionStatus::Ended);
        assert!(app.agents.get_session_agents(&ended_session_id).is_empty());

        let delete_config = DaemonConfig::for_tests();
        let deleted_session_id = {
            let mut app = DaemonApp::bootstrap(delete_config.clone()).expect("daemon should boot");
            let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
            router
                .runtime_state
                .delete_session_ref(session.id(), None)
                .await
                .expect("session should delete");
            session.id().to_string()
        };
        let app = DaemonApp::bootstrap(delete_config).expect("daemon should reboot");
        assert!(app.sessions().get_session(&deleted_session_id).is_err());
        assert!(
            app.agents
                .get_session_agents(&deleted_session_id)
                .is_empty()
        );
    }

    #[tokio::test]
    async fn rejects_session_commands_when_bounded_lane_is_full() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_and_session_capacity(Arc::clone(&app), 1, 1);
        let app_guard = app.lock().await;

        let first_request = attach_request(&session_id, "cli-1");
        let first_result_rx = router
            .session_runtime
            .enqueue_for_test(&session_id, "cmd-1", "session.attach", first_request)
            .await
            .expect("first command should enter the session lane");

        let mut first_command_is_running = false;
        for _ in 0..50 {
            if router
                .session_runtime
                .lane_capacity(&session_id)
                .await
                .is_some_and(|capacity| capacity == 1)
            {
                first_command_is_running = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            first_command_is_running,
            "first session command should be running before filling the queue"
        );

        let queued_request = attach_request(&session_id, "queued-cli");
        let queued_result_rx = router
            .session_runtime
            .enqueue_for_test(&session_id, "cmd-queued", "session.attach", queued_request)
            .await
            .expect("queued command should fill the session lane");

        let mut session_lane_is_full = false;
        for _ in 0..50 {
            if router
                .session_runtime
                .lane_capacity(&session_id)
                .await
                .is_some_and(|capacity| capacity == 0)
            {
                session_lane_is_full = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            session_lane_is_full,
            "session command queue should be full before overflow dispatch"
        );

        let third_request = attach_request(&session_id, "cli-overflow");
        let third_command =
            KernelCommand::from_local_request("cmd-overflow", None, None, &third_request);
        let error = router
            .dispatch(third_command, third_request)
            .await
            .expect_err("overflow session command should be rejected while lane is full");
        assert!(
            error
                .to_string()
                .contains("session command lane overloaded")
        );

        drop(app_guard);
        let _ = first_result_rx.await.expect("first result should resolve");
        let _ = queued_result_rx
            .await
            .expect("queued result should resolve");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_submit_does_not_wait_behind_slow_history_load() {
        let mut config = DaemonConfig::for_tests();
        config.session_history_read_delay_ms = 120;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-slow-history",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.history_store()
            .append(
                &session,
                &crate::history::SessionHistoryEntry::user_prompt(
                    &session_id,
                    attachment.id(),
                    &agent_id,
                    "slow history entry",
                ),
            )
            .expect("legacy-only history should append");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-history-prompt-state",
            None,
            None,
            &state_request,
        );
        router
            .dispatch(state_command, state_request)
            .await
            .expect("state read should warm session projection");

        let history_request = LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            round_count: Some(10),
            max_chars: None,
            before_entry_index: None,
            before_entry_char_offset: None,
        });
        let history_command = KernelCommand::from_local_request(
            "cmd-history-slow-background",
            None,
            None,
            &history_request,
        );
        let history_router = router.clone();
        let history_task = tokio::spawn(async move {
            history_router
                .dispatch(history_command, history_request)
                .await
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            !history_task.is_finished(),
            "test setup should keep history loading in the background"
        );

        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "submit while history is slow".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-during-history",
            None,
            None,
            &prompt_request,
        );
        let prompt_response = timeout(
            Duration::from_millis(75),
            router.dispatch(prompt_command, prompt_request),
        )
        .await
        .expect("prompt submit should not wait behind slow history")
        .expect("prompt submit should succeed");
        assert!(matches!(
            prompt_response,
            LocalDaemonResponse::PromptSubmitted { .. }
        ));

        let _ = history_task
            .await
            .expect("history task should join")
            .expect("history should eventually resolve");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn focus_resize_and_cancel_do_not_wait_behind_slow_provider_catalog() {
        let mut config = DaemonConfig::for_tests();
        config.provider_catalog_read_delay_ms = 120;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-slow-catalog",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "prompt to cancel while catalog is slow".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-catalog-prompt", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt should start before catalog drill");

        let catalog_request = LocalDaemonRequest::GetProviderCatalog(GetProviderCatalogRequest);
        let catalog_command =
            KernelCommand::from_local_request("cmd-slow-catalog", None, None, &catalog_request);
        let catalog_router = router.clone();
        let catalog_task = tokio::spawn(async move {
            catalog_router
                .dispatch(catalog_command, catalog_request)
                .await
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            !catalog_task.is_finished(),
            "test setup should keep provider catalog discovery in the background"
        );

        let focus_request = focus_request(&session_id, &agent_id);
        let focus_command = KernelCommand::from_local_request(
            "cmd-focus-during-catalog",
            None,
            None,
            &focus_request,
        );
        let focus_response = timeout(
            Duration::from_millis(75),
            router.dispatch(focus_command, focus_request),
        )
        .await
        .expect("focus should not wait behind slow catalog")
        .expect("focus should succeed");
        assert!(matches!(
            focus_response,
            LocalDaemonResponse::AgentFocused { .. }
        ));

        let resize_request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
            session_id: session_id.clone(),
            cols: 120,
            rows: 40,
        });
        let resize_command = KernelCommand::from_local_request(
            "cmd-resize-during-catalog",
            None,
            None,
            &resize_request,
        );
        let resize_response = timeout(
            Duration::from_millis(75),
            router.dispatch(resize_command, resize_request),
        )
        .await
        .expect("resize should not wait behind slow catalog")
        .expect("resize should succeed");
        assert!(matches!(
            resize_response,
            LocalDaemonResponse::TerminalResized { .. }
        ));

        let cancel_request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
        });
        let cancel_command = KernelCommand::from_local_request(
            "cmd-cancel-during-catalog",
            None,
            None,
            &cancel_request,
        );
        let cancel_response = timeout(
            Duration::from_millis(75),
            router.dispatch(cancel_command, cancel_request),
        )
        .await
        .expect("cancel should not wait behind slow catalog")
        .expect("cancel should succeed");
        assert!(matches!(
            cancel_response,
            LocalDaemonResponse::PromptCancelled { .. }
        ));

        let _ = catalog_task.await.expect("catalog task should join");
    }

    #[tokio::test]
    async fn session_runtime_publishes_attach_and_focus_projection_without_router_snapshot() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let second_agent = spawn_test_agent(&mut app, &session_id, "reviewer", "claude-code");
        assert_ne!(first_agent.id(), second_agent.id());

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let attach_request = attach_request(&session_id, "cli-session-projection");
        let attach_command = KernelCommand::from_local_request(
            "cmd-session-projection-attach",
            None,
            None,
            &attach_request,
        );
        let attachment_id = match router
            .dispatch(attach_command, attach_request)
            .await
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
            _ => panic!("unexpected attach response"),
        };

        let focus_request = focus_request(&session_id, second_agent.id());
        let focus_command = KernelCommand::from_local_request(
            "cmd-session-projection-focus",
            None,
            None,
            &focus_request,
        );
        router
            .dispatch(focus_command, focus_request)
            .await
            .expect("focus should succeed");

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-session-projection-state",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "session state should come from the SessionRuntime-published projection without taking the app lock"
        );
        drop(app_guard);

        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState { session } => {
                assert!(session.has_attachment(&attachment_id));
                assert_eq!(session.focused_agent_id(), Some(second_agent.id()));
            }
            _ => panic!("unexpected session state response"),
        }
    }

    #[tokio::test]
    async fn agent_lifecycle_refresh_uses_published_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let spawn_request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("projected-agent".to_string()),
            provider: Some("claude-code".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        });
        let spawn_command = KernelCommand::from_local_request(
            "cmd-agent-lifecycle-spawn",
            None,
            None,
            &spawn_request,
        );
        let spawned_agent_id = match router
            .dispatch(spawn_command, spawn_request)
            .await
            .expect("spawn should succeed")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
            _ => panic!("unexpected spawn response"),
        };
        assert!(
            router.session_runtime.has_lane(&session_id).await,
            "agent lifecycle should run through the session runtime lane"
        );

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-agent-lifecycle-spawn-state",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });
        let state_response = timeout(Duration::from_millis(100), state_task)
            .await
            .expect("spawn-projected state should not wait for the app lock")
            .expect("state task should join")
            .expect("state should resolve");
        drop(app_guard);
        match state_response {
            LocalDaemonResponse::SessionState { session } => {
                assert!(
                    session
                        .agents()
                        .iter()
                        .any(|agent| agent.id() == spawned_agent_id)
                );
            }
            _ => panic!("unexpected state response"),
        }

        let destroy_request = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
            session_id: session_id.clone(),
            agent_id: spawned_agent_id.clone(),
        });
        let destroy_command = KernelCommand::from_local_request(
            "cmd-agent-lifecycle-destroy",
            None,
            None,
            &destroy_request,
        );
        router
            .dispatch(destroy_command, destroy_request)
            .await
            .expect("destroy should succeed");
        assert!(
            router.session_runtime.has_lane(&session_id).await,
            "destroying an agent should not bypass the session runtime lane"
        );

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-agent-lifecycle-destroy-state",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });
        let state_response = timeout(Duration::from_millis(100), state_task)
            .await
            .expect("destroy-projected state should not wait for the app lock")
            .expect("state task should join")
            .expect("state should resolve");
        drop(app_guard);
        match state_response {
            LocalDaemonResponse::SessionState { session } => {
                assert!(
                    !session
                        .agents()
                        .iter()
                        .any(|agent| agent.id() == spawned_agent_id)
                );
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn end_session_uses_session_lane_and_removes_lane_registration() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let attach_request = attach_request(&session_id, "cli-1");
        let attach_command =
            KernelCommand::from_local_request("cmd-attach", None, None, &attach_request);
        router
            .dispatch(attach_command, attach_request)
            .await
            .expect("attach should create a session lane");
        assert!(router.session_runtime.has_lane(&session_id).await);

        let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session_id.clone(),
        });
        let end_command = KernelCommand::from_local_request("cmd-end", None, None, &end_request);
        let response = router
            .dispatch(end_command, end_request)
            .await
            .expect("end session should run through the session lane");

        assert!(matches!(
            response,
            crate::local::LocalDaemonResponse::SessionEnded { .. }
        ));
        assert!(
            !router.session_runtime.has_lane(&session_id).await,
            "ending a session should remove its mailbox registration"
        );
    }

    #[tokio::test]
    async fn delete_session_uses_owned_runtime_state_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let create_request = LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-delete-projection", "worktree")
                .with_alias("doomed"),
        );
        let create_command = KernelCommand::from_local_request(
            "cmd-delete-projection-create",
            None,
            None,
            &create_request,
        );
        let session_id = match router
            .dispatch(create_command, create_request)
            .await
            .expect("create should warm session projection")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session.id().to_string(),
            _ => panic!("unexpected create response"),
        };

        let app_guard = app.lock().await;
        let delete_request = LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: "doomed".to_string(),
            workspace_id: Some("workspace-delete-projection".to_string()),
        });
        let delete_command =
            KernelCommand::from_local_request("cmd-delete-projection", None, None, &delete_request);
        let delete_router = router.clone();
        let delete_task =
            tokio::spawn(
                async move { delete_router.dispatch(delete_command, delete_request).await },
            );

        let delete_response = timeout(Duration::from_millis(100), delete_task)
            .await
            .expect("owned delete should not wait for the app lock")
            .expect("delete task should join")
            .expect("delete should succeed");
        drop(app_guard);
        assert!(matches!(
            delete_response,
            LocalDaemonResponse::SessionDeleted { .. }
        ));
        assert!(
            !router.session_runtime.has_lane(&session_id).await,
            "deleting a session should remove its mailbox registration"
        );
    }

    #[tokio::test]
    async fn missing_delete_session_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-list-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let delete_request = LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: "missing-session".to_string(),
            workspace_id: None,
        });
        let delete_command =
            KernelCommand::from_local_request("cmd-delete-missing", None, None, &delete_request);
        let delete_router = router.clone();
        let delete_task =
            tokio::spawn(
                async move { delete_router.dispatch(delete_command, delete_request).await },
            );

        let error = timeout(Duration::from_millis(100), delete_task)
            .await
            .expect("missing delete should not wait for the app lock")
            .expect("delete task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_detach_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-list-warm-detach", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let detach_request = LocalDaemonRequest::DetachFromSession(DetachFromSessionRequest {
            attachment_id: "missing-attachment".to_string(),
        });
        let detach_command =
            KernelCommand::from_local_request("cmd-detach-missing", None, None, &detach_request);
        let detach_router = router.clone();
        let detach_task =
            tokio::spawn(
                async move { detach_router.dispatch(detach_command, detach_request).await },
            );

        let error = timeout(Duration::from_millis(100), detach_task)
            .await
            .expect("missing detach should not wait for the app lock")
            .expect("detach task should join")
            .expect_err("missing attachment should fail");
        drop(app_guard);

        match error {
            DaemonError::AttachmentNotFound { attachment_id } => {
                assert_eq!(attachment_id, "missing-attachment");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_attach_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-attach-missing-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let attach_request = attach_request("missing-session", "cli-missing-session");
        let attach_command =
            KernelCommand::from_local_request("cmd-attach-missing", None, None, &attach_request);
        let attach_router = router.clone();
        let attach_task =
            tokio::spawn(
                async move { attach_router.dispatch(attach_command, attach_request).await },
            );

        let error = timeout(Duration::from_millis(100), attach_task)
            .await
            .expect("missing attach should not wait for the app lock")
            .expect("attach task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_alias_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-alias-missing-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let alias_request = LocalDaemonRequest::AliasSession(AliasSessionRequest {
            session_id: "missing-session".to_string(),
            alias: "review".to_string(),
        });
        let alias_command =
            KernelCommand::from_local_request("cmd-alias-missing", None, None, &alias_request);
        let alias_router = router.clone();
        let alias_task =
            tokio::spawn(async move { alias_router.dispatch(alias_command, alias_request).await });

        let error = timeout(Duration::from_millis(100), alias_task)
            .await
            .expect("missing alias should not wait for the app lock")
            .expect("alias task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_end_session_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-end-missing-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: "missing-session".to_string(),
        });
        let end_command =
            KernelCommand::from_local_request("cmd-end-missing", None, None, &end_request);
        let end_router = router.clone();
        let end_task =
            tokio::spawn(async move { end_router.dispatch(end_command, end_request).await });

        let error = timeout(Duration::from_millis(100), end_task)
            .await
            .expect("missing end should not wait for the app lock")
            .expect("end task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn invalid_focus_uses_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-focus-invalid-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let focus_request = focus_request(&session_id, "missing-agent");
        let focus_command =
            KernelCommand::from_local_request("cmd-focus-invalid", None, None, &focus_request);
        let focus_router = router.clone();
        let focus_task =
            tokio::spawn(async move { focus_router.dispatch(focus_command, focus_request).await });

        let error = timeout(Duration::from_millis(100), focus_task)
            .await
            .expect("invalid focus should not wait for the app lock")
            .expect("focus task should join")
            .expect_err("missing agent should fail");
        drop(app_guard);

        match error {
            DaemonError::AgentNotInSession {
                session_id: error_session_id,
                agent_id,
            } => {
                assert_eq!(error_session_id, session_id);
                assert_eq!(agent_id, "missing-agent");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_cycle_focus_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-cycle-missing-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("list should warm the session projection");

        let app_guard = app.lock().await;
        let cycle_request = LocalDaemonRequest::CycleAgentFocus(CycleAgentFocusRequest {
            session_id: "missing-session".to_string(),
        });
        let cycle_command =
            KernelCommand::from_local_request("cmd-cycle-missing", None, None, &cycle_request);
        let cycle_router = router.clone();
        let cycle_task =
            tokio::spawn(async move { cycle_router.dispatch(cycle_command, cycle_request).await });

        let error = timeout(Duration::from_millis(100), cycle_task)
            .await
            .expect("missing cycle focus should not wait for the app lock")
            .expect("cycle task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn daemon_health_projection_reports_session_and_agent_mailboxes() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let focus_request = focus_request(&session_id, &agent_id);
        let focus_command =
            KernelCommand::from_local_request("cmd-focus", None, None, &focus_request);
        router
            .dispatch(focus_command, focus_request)
            .await
            .expect("focus should create a session lane");

        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "hello from health projection test".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt should create an agent lane");

        let workflow_request = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session_id.clone(),
            alias: Some("health-workflow".to_string()),
        });
        let workflow_command =
            KernelCommand::from_local_request("cmd-workflow", None, None, &workflow_request);
        router
            .dispatch(workflow_command, workflow_request)
            .await
            .expect("workflow command should create a workflow lane");

        let shell_request = LocalDaemonRequest::RunShellCommand(RunShellCapabilityRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            command: "/bin/true".to_string(),
            args: Vec::new(),
            working_directory: None,
            timeout_ms: Some(1_000),
        });
        let shell_command =
            KernelCommand::from_local_request("cmd-capability", None, None, &shell_request);
        router
            .dispatch(shell_command, shell_request)
            .await
            .expect_err(
                "capability command should report executor failure for missing test worktree",
            );

        let health_request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let health_command =
            KernelCommand::from_local_request("cmd-health", None, None, &health_request);
        let health_response = router
            .dispatch(health_command, health_request)
            .await
            .expect("health projection should be returned");
        let projection = match health_response {
            LocalDaemonResponse::DaemonHealth { projection } => projection,
            _ => panic!("unexpected health response"),
        };
        assert!(
            projection
                .session_command_lanes
                .iter()
                .any(|lane| lane.lane_id == session_id && lane.queue_limit == 128)
        );
        assert!(
            projection
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == agent_id && lane.queue_limit == 128)
        );
        assert!(
            projection
                .workflow_command_lanes
                .iter()
                .any(|lane| lane.lane_id == session_id && lane.queue_limit == 128)
        );
        assert_eq!(projection.session_projection.projected_sessions, 1);
        assert_eq!(projection.session_projection.active_prompts, 1);
        assert_eq!(projection.session_projection.queued_prompts, 0);
        assert_eq!(projection.agent_runtime_projection.projected_agents, 1);
        assert_eq!(projection.agent_runtime_projection.active_prompts, 1);
        assert_eq!(projection.agent_runtime_projection.queued_prompts, 0);
        assert_eq!(projection.capability_executor.max_concurrent_jobs, 64);
        assert_eq!(projection.capability_executor.available_permits, 64);
        assert_eq!(projection.capability_executor.submitted_jobs, 1);
        assert_eq!(projection.capability_executor.completed_jobs, 0);
        assert_eq!(projection.capability_executor.failed_jobs, 1);
        assert_eq!(projection.capability_executor.rejected_jobs, 0);
        assert!(!projection.provider_catalog.cached);
    }

    #[tokio::test]
    async fn daemon_health_reads_terminal_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let app_guard = app.lock().await;
        let health_request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let health_command =
            KernelCommand::from_local_request("cmd-health-no-lock", None, None, &health_request);
        let health_router = router.clone();
        let health_task =
            tokio::spawn(
                async move { health_router.dispatch(health_command, health_request).await },
            );

        let response = timeout(Duration::from_millis(100), health_task)
            .await
            .expect("daemon health should not wait for the app lock")
            .expect("health task should join")
            .expect("health should resolve");
        drop(app_guard);

        match response {
            LocalDaemonResponse::DaemonHealth { projection } => {
                assert_eq!(projection.terminal_stream.pending_output_records, 0);
            }
            _ => panic!("unexpected health response"),
        }
    }

    #[tokio::test]
    async fn relay_status_uses_config_projection_without_app_lock() {
        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some("ws://127.0.0.1:9".to_string());
        config.relay_token = Some("secret".to_string());
        config.host_machine_id = "machine-projected".to_string();
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let app_guard = app.lock().await;
        let relay_request = LocalDaemonRequest::RelayStatus(RelayStatusRequest);
        let relay_command = KernelCommand::from_local_request(
            "cmd-relay-status-projection",
            None,
            None,
            &relay_request,
        );
        let relay_router = router.clone();
        let relay_task =
            tokio::spawn(async move { relay_router.dispatch(relay_command, relay_request).await });

        let response = timeout(Duration::from_millis(100), relay_task)
            .await
            .expect("relay status should not wait for the app lock")
            .expect("relay task should join")
            .expect("relay status should resolve");
        drop(app_guard);

        match response {
            LocalDaemonResponse::RelayStatus { status } => {
                assert!(status.configured);
                assert_eq!(status.relay_url.as_deref(), Some("ws://127.0.0.1:9"));
                assert!(status.relay_token_configured);
                assert_eq!(status.machine_id, "machine-projected");
            }
            _ => panic!("unexpected relay response"),
        }
    }

    #[tokio::test]
    async fn provider_command_catalogs_do_not_wait_for_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let app_guard = app.lock().await;
        let catalog_request =
            LocalDaemonRequest::GetProviderCommandCatalogs(GetProviderCommandCatalogsRequest);
        let catalog_command = KernelCommand::from_local_request(
            "cmd-provider-command-catalog-projection",
            None,
            None,
            &catalog_request,
        );
        let catalog_router = router.clone();
        let catalog_task = tokio::spawn(async move {
            catalog_router
                .dispatch(catalog_command, catalog_request)
                .await
        });

        let response = timeout(Duration::from_millis(100), catalog_task)
            .await
            .expect("provider command catalogs should not wait for the app lock")
            .expect("catalog task should join")
            .expect("provider command catalogs should resolve");
        drop(app_guard);

        match response {
            LocalDaemonResponse::ProviderCommandCatalogs { catalogs } => {
                assert!(!catalogs.is_empty());
            }
            _ => panic!("unexpected provider command catalog response"),
        }
    }

    #[tokio::test]
    async fn provider_auth_status_does_not_use_generic_app_fallback() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let app_guard = app.lock().await;
        let auth_request =
            LocalDaemonRequest::GetProviderAuthStatus(GetProviderAuthStatusRequest {
                provider: "unsupported-provider".to_string(),
            });
        let auth_command = KernelCommand::from_local_request(
            "cmd-provider-auth-no-fallback",
            None,
            None,
            &auth_request,
        );
        let auth_router = router.clone();
        let auth_task =
            tokio::spawn(async move { auth_router.dispatch(auth_command, auth_request).await });

        let error = timeout(Duration::from_millis(100), auth_task)
            .await
            .expect("provider auth status should not wait for the app lock")
            .expect("auth task should join")
            .expect_err("unsupported provider should be rejected");
        drop(app_guard);

        match error {
            DaemonError::LocalTransport { operation, message } => {
                assert_eq!(operation, "get_provider_auth_status");
                assert!(message.contains("unsupported-provider"));
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn agent_and_workflow_lanes_are_removed_when_session_ends() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-agent-lane-cleanup",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "create agent lane".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-agent-lane-create", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt should create an agent lane");
        assert!(
            router
                .daemon_health_projection(0)
                .await
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == agent_id)
        );
        let workflow_request = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session_id.clone(),
            alias: Some("cleanup-workflow".to_string()),
        });
        let workflow_command = KernelCommand::from_local_request(
            "cmd-workflow-lane-create",
            None,
            None,
            &workflow_request,
        );
        router
            .dispatch(workflow_command, workflow_request)
            .await
            .expect("workflow command should create a workflow lane");
        assert!(router.workflow_runtime.has_lane(&session_id).await);

        let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session_id.clone(),
        });
        let end_command =
            KernelCommand::from_local_request("cmd-agent-lane-end", None, None, &end_request);
        router
            .dispatch(end_command, end_request)
            .await
            .expect("ending session should clean up agent lane");

        assert!(
            !router
                .daemon_health_projection(0)
                .await
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == agent_id)
        );
        assert!(!router.workflow_runtime.has_lane(&session_id).await);
    }

    #[tokio::test]
    async fn agent_lane_is_removed_when_agent_is_destroyed() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-agent-destroy-lane-cleanup",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "create agent lane".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-agent-destroy-lane-create",
            None,
            None,
            &prompt_request,
        );
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt should create an agent lane");
        assert!(
            router
                .daemon_health_projection(0)
                .await
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == agent_id)
        );

        let destroy_request = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
            session_id,
            agent_id: agent_id.clone(),
        });
        let destroy_command = KernelCommand::from_local_request(
            "cmd-agent-destroy-lane-cleanup",
            None,
            None,
            &destroy_request,
        );
        router
            .dispatch(destroy_command, destroy_request)
            .await
            .expect("destroying agent should clean up agent lane");

        assert!(
            !router
                .daemon_health_projection(0)
                .await
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == agent_id)
        );
    }

    #[tokio::test]
    async fn prompt_submit_uses_agent_lane_without_generic_interactive_lane() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;

        let first_request = focus_request(&session_id, &agent_id);
        let first_command =
            KernelCommand::from_local_request("cmd-focus-1", None, None, &first_request);
        let first_router = router.clone();
        let first_task =
            tokio::spawn(async move { first_router.dispatch(first_command, first_request).await });

        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "hello from agent lane".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt", None, None, &prompt_request);
        let prompt_router = router.clone();
        let prompt_task =
            tokio::spawn(
                async move { prompt_router.dispatch(prompt_command, prompt_request).await },
            );

        let prompt_response = timeout(Duration::from_millis(100), prompt_task)
            .await
            .expect("owned prompt submit should not wait for the app lock")
            .expect("prompt task should join")
            .expect("prompt should submit");
        drop(app_guard);
        let _ = first_task.await.expect("first focus should join");
        match prompt_response {
            crate::local::LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), agent_id);
                }
                _ => panic!("expected prompt to start"),
            },
            _ => panic!("unexpected prompt response"),
        }
    }

    #[tokio::test]
    async fn prompt_submit_uses_session_focus_projection_without_app_lock_for_routing() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let focused_agent = spawn_test_agent(&mut app, &session_id, "focused", "claude-code");
        launch_test_provider(
            &mut app,
            &session_id,
            focused_agent.id(),
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let focus_request = focus_request(&session_id, focused_agent.id());
        let focus_command =
            KernelCommand::from_local_request("cmd-focus-projection", None, None, &focus_request);
        router
            .dispatch(focus_command, focus_request)
            .await
            .expect("focus should populate the projection");

        let app_guard = app.lock().await;
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: None,
            prompt: "hello through projected focus".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt-projection", None, None, &prompt_request);
        let prompt_router = router.clone();
        let prompt_task =
            tokio::spawn(
                async move { prompt_router.dispatch(prompt_command, prompt_request).await },
            );

        let prompt_response = timeout(Duration::from_millis(100), prompt_task)
            .await
            .expect("owned prompt submit should not wait for the app lock")
            .expect("prompt task should join")
            .expect("prompt should submit");
        drop(app_guard);
        match prompt_response {
            LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), focused_agent.id());
                }
                _ => panic!("expected prompt to start"),
            },
            _ => panic!("unexpected prompt response"),
        }
    }

    #[tokio::test]
    async fn prompt_submit_uses_warmed_session_projection_without_app_lock_for_focus_fallback() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-session-projection-focus",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-focus-fallback-warm",
            None,
            None,
            &state_request,
        );
        router
            .dispatch(state_command, state_request)
            .await
            .expect("state read should warm the session projection");

        let app_guard = app.lock().await;
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: None,
            prompt: "hello through warmed session projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-session-projection-focus",
            None,
            None,
            &prompt_request,
        );
        let prompt_router = router.clone();
        let prompt_task =
            tokio::spawn(
                async move { prompt_router.dispatch(prompt_command, prompt_request).await },
            );

        let prompt_response = timeout(Duration::from_millis(100), prompt_task)
            .await
            .expect("owned prompt submit should not wait for the app lock")
            .expect("prompt task should join")
            .expect("prompt should submit");
        drop(app_guard);
        match prompt_response {
            LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), agent_id);
                }
                _ => panic!("expected prompt to start"),
            },
            _ => panic!("unexpected prompt response"),
        }
    }

    #[tokio::test]
    async fn agent_spawn_refreshes_focus_projection_for_followup_prompt_routing() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let spawn_request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("spawned".to_string()),
            provider: Some("claude-code".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        });
        let spawn_command =
            KernelCommand::from_local_request("cmd-spawn-projection", None, None, &spawn_request);
        let spawned_agent = match router
            .dispatch(spawn_command, spawn_request)
            .await
            .expect("spawn should succeed")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected spawn response"),
        };

        {
            let mut app = app.lock().await;
            launch_test_provider(
                &mut app,
                &session_id,
                spawned_agent.id(),
                "dev-stub",
                "claude-code",
                "sonnet",
            );
        }

        let app_guard = app.lock().await;
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: None,
            prompt: "hello after spawn".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-after-spawn",
            None,
            None,
            &prompt_request,
        );
        let prompt_router = router.clone();
        let prompt_task =
            tokio::spawn(
                async move { prompt_router.dispatch(prompt_command, prompt_request).await },
            );

        let mut spawned_agent_lane_created = false;
        for _ in 0..50 {
            let projection = router.daemon_health_projection(0).await;
            if projection
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == spawned_agent.id())
            {
                spawned_agent_lane_created = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            spawned_agent_lane_created,
            "spawn should refresh focused-agent projection before followup prompt routing"
        );

        drop(app_guard);
        let prompt_response = prompt_task
            .await
            .expect("prompt task should join")
            .expect("prompt should submit");
        match prompt_response {
            LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), spawned_agent.id());
                }
                _ => panic!("expected prompt to start"),
            },
            _ => panic!("unexpected prompt response"),
        }
    }

    #[tokio::test]
    async fn get_session_state_uses_projection_after_prompt_submit_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "warm session projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt-state", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm the session projection");

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command =
            KernelCommand::from_local_request("cmd-state-projection", None, None, &state_request);
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "warm GetSessionState should be served from the session projection without app lock access"
        );

        drop(app_guard);
        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState { session } => {
                assert!(session.active_prompt_for_agent(&agent_id).is_some());
                assert_eq!(session.agents().len(), 1);
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn update_session_config_uses_session_runtime_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-config-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let update_request = LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
            requires_idle: false,
        });
        let update_command =
            KernelCommand::from_local_request("cmd-session-config", None, None, &update_request);
        let update_response = router
            .dispatch(update_command, update_request)
            .await
            .expect("session config update should succeed");
        match update_response {
            LocalDaemonResponse::SessionConfigUpdated { config, session } => {
                assert_eq!(config.version(), 1);
                assert_eq!(session.config_state().version(), 1);
                assert_eq!(
                    session.config_state().values().get("theme"),
                    Some(&"compact".to_string())
                );
            }
            _ => panic!("unexpected config response"),
        }

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-session-config-state",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "session config update should publish a session projection for lock-free state reads"
        );

        drop(app_guard);
        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState { session } => {
                assert_eq!(session.config_state().version(), 1);
                assert_eq!(
                    session.config_state().values().get("theme"),
                    Some(&"compact".to_string())
                );
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn alias_session_uses_session_runtime_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let alias_request = LocalDaemonRequest::AliasSession(AliasSessionRequest {
            session_id: session_id.clone(),
            alias: "review entry".to_string(),
        });
        let alias_command =
            KernelCommand::from_local_request("cmd-session-alias", None, None, &alias_request);
        let alias_response = router
            .dispatch(alias_command, alias_request)
            .await
            .expect("session alias should succeed");
        match alias_response {
            LocalDaemonResponse::SessionAliased { session } => {
                assert_eq!(session.alias(), Some("review_entry"));
            }
            _ => panic!("unexpected alias response"),
        }

        let app_guard = app.lock().await;
        let resolve_request = LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "review_entry".to_string(),
            workspace_id: Some("workspace".to_string()),
        });
        let resolve_command = KernelCommand::from_local_request(
            "cmd-session-alias-resolve",
            None,
            None,
            &resolve_request,
        );
        let resolve_router = router.clone();
        let resolve_task = tokio::spawn(async move {
            resolve_router
                .dispatch(resolve_command, resolve_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(
            resolve_task.is_finished(),
            "session alias should publish a projection that resolves without app lock access"
        );

        drop(app_guard);
        let resolve_response = resolve_task
            .await
            .expect("resolve task should join")
            .expect("resolve should succeed");
        match resolve_response {
            LocalDaemonResponse::SessionResolved { session } => {
                assert_eq!(session.id(), session_id);
                assert_eq!(session.alias(), Some("review_entry"));
            }
            _ => panic!("unexpected resolve response"),
        }
    }

    #[tokio::test]
    async fn poll_runtime_notices_routes_through_session_runtime() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let source = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-notice-source",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("source attachment should attach");
        let recipient = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-notice-recipient",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("recipient attachment should attach");
        app.record_notice(
            &session_id,
            None,
            vec![recipient.id().to_string()],
            format!(
                "Attachment `{}` updated configuration for session `{}`.",
                source.id(),
                session_id
            ),
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command = KernelCommand::from_local_request(
            "cmd-runtime-notices-warm",
            None,
            None,
            &list_request,
        );
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm session projection");

        let app_guard = app.lock().await;
        let poll_request = LocalDaemonRequest::PollRuntimeNotices(PollRuntimeNoticesRequest {
            session_id: session_id.clone(),
            attachment_id: recipient.id().to_string(),
        });
        let poll_command =
            KernelCommand::from_local_request("cmd-runtime-notices", None, None, &poll_request);
        let poll_router = router.clone();
        let poll_task =
            tokio::spawn(async move { poll_router.dispatch(poll_command, poll_request).await });
        let poll_response = timeout(Duration::from_millis(100), poll_task)
            .await
            .expect("notice poll should not wait for the app lock")
            .expect("poll task should join")
            .expect("notice poll should succeed");
        drop(app_guard);

        assert!(
            router.session_runtime.has_lane(&session_id).await,
            "notice polling should be admitted through the per-session runtime lane"
        );
        match poll_response {
            LocalDaemonResponse::RuntimeNotices { notices } => {
                assert_eq!(notices.len(), 1);
                assert_eq!(notices[0].session_id, session_id);
            }
            _ => panic!("unexpected notice response"),
        }
    }

    #[tokio::test]
    async fn resize_without_active_run_uses_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command = KernelCommand::from_local_request(
            "cmd-resize-no-active-warm",
            None,
            None,
            &list_request,
        );
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm session projection");

        let app_guard = app.lock().await;
        let resize_request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
            session_id: session_id.clone(),
            cols: 120,
            rows: 40,
        });
        let resize_command = KernelCommand::from_local_request(
            "cmd-resize-no-active-projection",
            None,
            None,
            &resize_request,
        );
        let resize_router = router.clone();
        let resize_task =
            tokio::spawn(
                async move { resize_router.dispatch(resize_command, resize_request).await },
            );

        let error = timeout(Duration::from_millis(100), resize_task)
            .await
            .expect("resize absence should not wait for the app lock")
            .expect("resize task should join")
            .expect_err("resize without active provider run should fail");
        drop(app_guard);

        match error {
            DaemonError::NoActiveProviderRun {
                session_id: error_session_id,
            } => assert_eq!(error_session_id, session_id),
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn get_session_state_projection_tracks_prompt_completion_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-complete-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "complete projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-complete-state",
            None,
            None,
            &prompt_request,
        );
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm active prompt projection");
        let prompt_projection = router
            .agent_runtime_projection
            .get(&agent_id)
            .expect("agent runtime projection should track prompt state after submit");
        assert!(prompt_projection.active_prompt.is_some());
        assert_eq!(prompt_projection.queued_prompt_count, 0);

        let complete_request = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session_id.clone(),
        });
        let complete_command = KernelCommand::from_local_request(
            "cmd-complete-state-projection",
            None,
            None,
            &complete_request,
        );
        router
            .dispatch(complete_command, complete_request)
            .await
            .expect("prompt completion should publish session projection through agent runtime");
        let prompt_projection = router
            .agent_runtime_projection
            .get(&agent_id)
            .expect("agent runtime projection should retain prompt state after complete");
        assert!(prompt_projection.active_prompt.is_none());
        assert_eq!(prompt_projection.queued_prompt_count, 0);

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-state-complete-projection",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "completed prompt state should be served from projection without app lock access"
        );
        drop(app_guard);

        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState { session } => {
                assert!(session.active_prompt_for_agent(&agent_id).is_none());
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn session_snapshot_refresh_tracks_agent_runtime_projection() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-prompt-shadow-refresh",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "shadow refresh".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-shadow-submit", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm agent runtime projection");
        assert!(
            router
                .agent_runtime_projection
                .get(&agent_id)
                .and_then(|projection| projection.active_prompt)
                .is_some()
        );

        {
            let app = app.lock().await;
            app.sessions_mut()
                .complete_active_prompt_only(&session_id, &agent_id)
                .expect("compatibility state should be externally settled");
        }
        assert!(
            router
                .agent_runtime_projection
                .get(&agent_id)
                .and_then(|projection| projection.active_prompt)
                .is_some(),
            "prompt projection should stay stale until a session snapshot is observed"
        );

        let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
        });
        let pump_command =
            KernelCommand::from_local_request("cmd-shadow-refresh", None, None, &pump_request);
        router
            .dispatch(pump_command, pump_request)
            .await
            .expect("snapshot-producing pump should refresh projections");

        let prompt_projection = router
            .agent_runtime_projection
            .get(&agent_id)
            .expect("agent prompt projection should remain registered");
        assert!(prompt_projection.active_prompt.is_none());
        assert_eq!(prompt_projection.queued_prompt_count, 0);
    }

    #[tokio::test]
    async fn prompt_complete_uses_agent_runtime_projection_when_session_projection_is_stale() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let default_agent_id = default_agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-complete-owner-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let spawned_agent = spawn_test_agent(&mut app, &session_id, "worker", "claude-code");
        let spawned_agent_id = spawned_agent.id().to_string();
        launch_test_provider(
            &mut app,
            &session_id,
            &spawned_agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );
        focus_test_agent(&mut app, &session_id, &default_agent_id);
        let idle_session_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("idle session snapshot should be available");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(spawned_agent_id.clone()),
            prompt: "complete owner projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-complete-owner",
            None,
            None,
            &prompt_request,
        );
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm active prompt projection");
        router.session_projection.update(idle_session_snapshot);

        let app_guard = app.lock().await;
        let complete_request = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session_id.clone(),
        });
        let complete_command = KernelCommand::from_local_request(
            "cmd-complete-owner-projection",
            None,
            None,
            &complete_request,
        );
        let complete_router = router.clone();
        let complete_task = tokio::spawn(async move {
            complete_router
                .dispatch(complete_command, complete_request)
                .await
        });

        let mut spawned_agent_lane_created = false;
        for _ in 0..50 {
            let projection = router.daemon_health_projection(0).await;
            if projection
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == spawned_agent_id)
            {
                spawned_agent_lane_created = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            spawned_agent_lane_created,
            "prompt complete should resolve the active prompt owner from the agent-runtime projection before touching the app lock"
        );
        assert!(
            !complete_task.is_finished(),
            "agent worker should still wait on the deliberately held app lock"
        );

        drop(app_guard);
        let complete_response = complete_task
            .await
            .expect("complete task should join")
            .expect("prompt should complete");
        match complete_response {
            LocalDaemonResponse::PromptCompleted { .. } => {}
            _ => panic!("unexpected complete response"),
        }
    }

    #[tokio::test]
    async fn get_session_state_projection_tracks_prompt_cancellation_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-cancel-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "cancel projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-cancel-state",
            None,
            None,
            &prompt_request,
        );
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm active prompt projection");
        assert!(
            router
                .agent_runtime_projection
                .get(&agent_id)
                .and_then(|projection| projection.active_prompt)
                .is_some()
        );

        let cancel_request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
        });
        let cancel_command = KernelCommand::from_local_request(
            "cmd-cancel-state-projection",
            None,
            None,
            &cancel_request,
        );
        router
            .dispatch(cancel_command, cancel_request)
            .await
            .expect("prompt cancellation should publish session projection");
        let prompt_projection = router
            .agent_runtime_projection
            .get(&agent_id)
            .expect("agent runtime projection should retain prompt state after cancel");
        assert_eq!(
            prompt_projection
                .active_prompt
                .as_ref()
                .map(|prompt| prompt.status()),
            Some(PromptStatus::Cancelling)
        );

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-state-cancel-projection",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "cancelled prompt state should be served from projection without app lock access"
        );
        drop(app_guard);

        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState { session } => {
                let active_prompt = session
                    .active_prompt_for_agent(&agent_id)
                    .expect("prompt should still be settling");
                assert_eq!(active_prompt.status(), PromptStatus::Cancelling);
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn prompt_cancel_uses_agent_runtime_projection_when_session_projection_is_stale() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let default_agent_id = default_agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-cancel-owner-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let spawned_agent = spawn_test_agent(&mut app, &session_id, "worker", "claude-code");
        let spawned_agent_id = spawned_agent.id().to_string();
        launch_test_provider(
            &mut app,
            &session_id,
            &spawned_agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );
        focus_test_agent(&mut app, &session_id, &default_agent_id);
        let idle_session_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("idle session snapshot should be available");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(spawned_agent_id.clone()),
            prompt: "cancel owner projection".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command = KernelCommand::from_local_request(
            "cmd-prompt-cancel-owner",
            None,
            None,
            &prompt_request,
        );
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt submit should warm active prompt projection");
        router.session_projection.update(idle_session_snapshot);

        let app_guard = app.lock().await;
        let cancel_request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
        });
        let cancel_command = KernelCommand::from_local_request(
            "cmd-cancel-owner-projection",
            None,
            None,
            &cancel_request,
        );
        let cancel_router = router.clone();
        let cancel_task =
            tokio::spawn(
                async move { cancel_router.dispatch(cancel_command, cancel_request).await },
            );

        let mut spawned_agent_lane_created = false;
        for _ in 0..50 {
            let projection = router.daemon_health_projection(0).await;
            if projection
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == spawned_agent_id)
            {
                spawned_agent_lane_created = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            spawned_agent_lane_created,
            "prompt cancel should resolve the active prompt owner from the agent-runtime projection before touching the app lock"
        );
        assert!(
            !cancel_task.is_finished(),
            "agent worker should still wait on the deliberately held app lock"
        );

        drop(app_guard);
        let cancel_response = cancel_task
            .await
            .expect("cancel task should join")
            .expect("prompt should cancel");
        match cancel_response {
            LocalDaemonResponse::PromptCancelled { cancellation } => {
                assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
            }
            _ => panic!("unexpected cancel response"),
        }
    }

    #[tokio::test]
    async fn session_history_load_uses_warmed_session_projection_without_app_lock() {
        let mut config = DaemonConfig::for_tests();
        config.session_history_read_delay_ms = 25;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-history-load",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.append_user_prompt_history(
            &session_id,
            attachment.id(),
            &agent_id,
            "history from disk",
            &[],
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command =
            KernelCommand::from_local_request("cmd-history-state-warm", None, None, &state_request);
        router
            .dispatch(state_command, state_request)
            .await
            .expect("state read should warm session projection");

        let app_guard = app.lock().await;
        let history_request = LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            round_count: Some(10),
            max_chars: None,
            before_entry_index: None,
            before_entry_char_offset: None,
        });
        let history_command = KernelCommand::from_local_request(
            "cmd-history-without-app-lock",
            None,
            None,
            &history_request,
        );
        let history_router = router.clone();
        let history_task = tokio::spawn(async move {
            history_router
                .dispatch(history_command, history_request)
                .await
        });

        let history_response = timeout(Duration::from_millis(250), history_task)
            .await
            .expect("history load should finish while app lock is held")
            .expect("history task should join")
            .expect("history should resolve");
        drop(app_guard);

        match history_response {
            LocalDaemonResponse::SessionHistory { entries, .. } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].entry.text.trim_end(), "history from disk");
            }
            _ => panic!("unexpected history response"),
        }
    }

    #[tokio::test]
    async fn query_history_reads_operational_events() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-history-query",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.append_user_prompt_history(
            &session_id,
            attachment.id(),
            &agent_id,
            "find this history event",
            &[],
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let history_request = LocalDaemonRequest::QueryHistory(QueryHistoryRequest {
            session_id: Some(session_id.clone()),
            agent_id: Some(agent_id.clone()),
            text: Some("history event".to_string()),
            limit: Some(5),
            ..QueryHistoryRequest::default()
        });
        let history_command =
            KernelCommand::from_local_request("cmd-history-query", None, None, &history_request);

        let response = router
            .dispatch(history_command, history_request)
            .await
            .expect("history query should resolve");

        match response {
            LocalDaemonResponse::HistoryEvents {
                events,
                next_sequence,
            } => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].session_id.as_deref(), Some(session_id.as_str()));
                assert_eq!(events[0].agent_id.as_deref(), Some(agent_id.as_str()));
                assert_eq!(
                    events[0].content.as_deref().map(str::trim_end),
                    Some("find this history event")
                );
                assert!(next_sequence.is_none());
            }
            _ => panic!("unexpected history query response"),
        }
    }

    #[tokio::test]
    async fn warmed_session_history_projection_tracks_appends_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-history-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.append_user_prompt_history(&session_id, attachment.id(), &agent_id, "first", &[]);

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let history_request = LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            round_count: Some(10),
            max_chars: None,
            before_entry_index: None,
            before_entry_char_offset: None,
        });
        let history_command =
            KernelCommand::from_local_request("cmd-history-warm", None, None, &history_request);
        router
            .dispatch(history_command, history_request)
            .await
            .expect("initial history read should warm projection");

        {
            let app = app.lock().await;
            app.append_user_prompt_history(&session_id, attachment.id(), &agent_id, "second", &[]);
        }

        let app_guard = app.lock().await;
        let projected_history_request =
            LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                round_count: Some(10),
                max_chars: None,
                before_entry_index: None,
                before_entry_char_offset: None,
            });
        let projected_history_command = KernelCommand::from_local_request(
            "cmd-history-projection",
            None,
            None,
            &projected_history_request,
        );
        let history_router = router.clone();
        let history_task = tokio::spawn(async move {
            history_router
                .dispatch(projected_history_command, projected_history_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(
            history_task.is_finished(),
            "warmed GetSessionHistory should be served from the history projection without app lock access"
        );
        drop(app_guard);

        let history_response = history_task
            .await
            .expect("history task should join")
            .expect("history should resolve");
        match history_response {
            LocalDaemonResponse::SessionHistory { entries, .. } => {
                let texts = entries
                    .into_iter()
                    .map(|entry| entry.entry.text.trim_end().to_string())
                    .collect::<Vec<_>>();
                assert_eq!(texts, vec!["first".to_string(), "second".to_string()]);
            }
            _ => panic!("unexpected history response"),
        }
    }

    #[tokio::test]
    async fn get_provider_run_uses_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        });
        let launch_command =
            KernelCommand::from_local_request("cmd-provider-launch", None, None, &launch_request);
        let provider_run_id = match router
            .dispatch(launch_command, launch_request)
            .await
            .expect("provider launch should be accepted")
        {
            LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => {
                provider_run.id().to_string()
            }
            _ => panic!("unexpected launch response"),
        };

        let app_guard = app.lock().await;
        let provider_request = LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
            provider_run_id: provider_run_id.clone(),
        });
        let provider_command = KernelCommand::from_local_request(
            "cmd-provider-projection",
            None,
            None,
            &provider_request,
        );
        let provider_router = router.clone();
        let provider_task = tokio::spawn(async move {
            provider_router
                .dispatch(provider_command, provider_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(
            provider_task.is_finished(),
            "warmed GetProviderRun should be served from the provider-run projection without app lock access"
        );
        drop(app_guard);

        let provider_response = provider_task
            .await
            .expect("provider task should join")
            .expect("provider run should resolve");
        match provider_response {
            LocalDaemonResponse::ProviderRun { provider_run } => {
                assert_eq!(provider_run.id(), provider_run_id);
            }
            _ => panic!("unexpected provider response"),
        }
    }

    #[tokio::test]
    async fn get_provider_run_does_not_bypass_opencode_selection_sync_path() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let provider_run = RuntimeProviderRun::from_control_capability_inference(
            "projected-opencode-run",
            session.id().to_string(),
            Some(agent.id().to_string()),
            "opencode".to_string(),
        );
        app.update_provider_run_projection(provider_run.clone());

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;
        let provider_request = LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
            provider_run_id: provider_run.id().to_string(),
        });
        let provider_command = KernelCommand::from_local_request(
            "cmd-opencode-provider-run-refresh",
            None,
            None,
            &provider_request,
        );
        let provider_router = router.clone();
        let provider_task = tokio::spawn(async move {
            provider_router
                .dispatch(provider_command, provider_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(
            !provider_task.is_finished(),
            "warmed opencode GetProviderRun must not bypass the refresh/sync handler"
        );
        drop(app_guard);
        let _ = provider_task
            .await
            .expect("provider task should join after app lock is released");
    }

    #[tokio::test]
    async fn provider_run_projection_tracks_async_launch_completion() {
        let mut config = DaemonConfig::for_tests();
        config.provider_runtime_init_delay_ms = 25;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        });
        let launch_command = KernelCommand::from_local_request(
            "cmd-provider-launch-async",
            None,
            None,
            &launch_request,
        );
        let provider_run_id = match router
            .dispatch(launch_command, launch_request)
            .await
            .expect("provider launch should be accepted")
        {
            LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => {
                assert_eq!(
                    provider_run.state(),
                    crate::provider::ProviderRunState::Starting
                );
                provider_run.id().to_string()
            }
            _ => panic!("unexpected launch response"),
        };

        let mut running_seen = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let provider_request = LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
                provider_run_id: provider_run_id.clone(),
            });
            let provider_command = KernelCommand::from_local_request(
                "cmd-provider-running-poll",
                None,
                None,
                &provider_request,
            );
            let response = router
                .dispatch(provider_command, provider_request)
                .await
                .expect("provider run should resolve");
            if let LocalDaemonResponse::ProviderRun { provider_run } = response {
                if provider_run.state() == crate::provider::ProviderRunState::Running {
                    running_seen = true;
                    break;
                }
            }
        }
        assert!(
            running_seen,
            "provider projection should observe async launch completion"
        );

        let app_guard = app.lock().await;
        let provider_request = LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
            provider_run_id: provider_run_id.clone(),
        });
        let provider_command = KernelCommand::from_local_request(
            "cmd-provider-running-projection",
            None,
            None,
            &provider_request,
        );
        let provider_router = router.clone();
        let provider_task = tokio::spawn(async move {
            provider_router
                .dispatch(provider_command, provider_request)
                .await
        });
        tokio::task::yield_now().await;
        assert!(provider_task.is_finished());
        drop(app_guard);

        let provider_response = provider_task
            .await
            .expect("provider task should join")
            .expect("provider run should resolve");
        match provider_response {
            LocalDaemonResponse::ProviderRun { provider_run } => {
                assert_eq!(
                    provider_run.state(),
                    crate::provider::ProviderRunState::Running
                );
            }
            _ => panic!("unexpected provider response"),
        }

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-provider-running-session-projection",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });
        let state_response = timeout(Duration::from_millis(100), state_task)
            .await
            .expect("async launch completion should publish session projection without app lock")
            .expect("state task should join")
            .expect("state should resolve");
        drop(app_guard);

        match state_response {
            LocalDaemonResponse::SessionState { session } => {
                assert_eq!(
                    session.active_provider_run_id(),
                    Some(provider_run_id.as_str())
                );
            }
            _ => panic!("unexpected session state response"),
        }
    }

    #[tokio::test]
    async fn settled_provider_launch_pending_state_uses_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (mut session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let mut provider_run = RuntimeProviderRun::from_control_capability_inference(
            "projected-run",
            session_id.clone(),
            Some(agent_id),
            "dev-stub".to_string(),
        );
        provider_run.mark_running();
        session.set_active_provider_run(Some(provider_run.id().to_string()));
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        router.session_projection.update(session);
        router.provider_run_projection.update(provider_run);
        router
            .pending_provider_launch_sessions
            .lock()
            .await
            .insert(session_id.clone());

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-settled-launch-state-projection",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        let response = timeout(Duration::from_millis(100), state_task)
            .await
            .expect("settled provider launch state should not wait for the app lock")
            .expect("state task should join")
            .expect("state should resolve");
        drop(app_guard);

        match response {
            LocalDaemonResponse::SessionState { session } => {
                assert_eq!(session.id(), session_id);
                assert_eq!(session.active_provider_run_id(), Some("projected-run"));
            }
            _ => panic!("unexpected state response"),
        }
        assert!(
            !router
                .pending_provider_launch_sessions
                .lock()
                .await
                .contains(&session_id),
            "projection-settled launch should clear pending launch guard"
        );
    }

    #[tokio::test]
    async fn list_provider_processes_uses_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        });
        let launch_command = KernelCommand::from_local_request(
            "cmd-process-provider-launch",
            None,
            None,
            &launch_request,
        );
        let provider_run_id = match router
            .dispatch(launch_command, launch_request)
            .await
            .expect("provider launch should be accepted")
        {
            LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => {
                provider_run.id().to_string()
            }
            _ => panic!("unexpected launch response"),
        };

        let list_request =
            LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest {
                provider: None,
            });
        let list_command =
            KernelCommand::from_local_request("cmd-process-list-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial provider process list should warm projection");

        let app_guard = app.lock().await;
        let projected_list_request =
            LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest {
                provider: None,
            });
        let projected_list_command = KernelCommand::from_local_request(
            "cmd-process-list-projection",
            None,
            None,
            &projected_list_request,
        );
        let list_router = router.clone();
        let list_task = tokio::spawn(async move {
            list_router
                .dispatch(projected_list_command, projected_list_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(
            list_task.is_finished(),
            "warmed ListProviderProcesses should be served from projection without app lock access"
        );
        drop(app_guard);

        let list_response = list_task
            .await
            .expect("process list task should join")
            .expect("process list should resolve");
        match list_response {
            LocalDaemonResponse::ProviderProcessesListed { processes } => {
                assert_eq!(processes.len(), 1);
                assert_eq!(processes[0].owner_provider_run_ids, vec![provider_run_id]);
            }
            _ => panic!("unexpected provider process list response"),
        }
    }

    #[tokio::test]
    async fn provider_process_projection_stores_canonical_unfiltered_snapshot() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        for (idx, provider, model) in [(1, "claude-code", "sonnet"), (2, "codex", "gpt-5.4")] {
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new(
                    format!("workspace-{idx}"),
                    format!("worktree-{idx}"),
                ))
                .expect("session should be created");
            launch_test_provider(
                &mut app,
                session.id(),
                agent.id(),
                "dev-stub",
                provider,
                model,
            );
        }

        let filtered = app
            .list_provider_processes(Some("claude-code"))
            .expect("filtered process list should warm projection");
        assert_eq!(filtered.len(), 1);

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;
        let list_request =
            LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest {
                provider: None,
            });
        let list_command = KernelCommand::from_local_request(
            "cmd-process-canonical-projection",
            None,
            None,
            &list_request,
        );
        let list_router = router.clone();
        let list_task =
            tokio::spawn(async move { list_router.dispatch(list_command, list_request).await });

        tokio::task::yield_now().await;
        assert!(list_task.is_finished());
        drop(app_guard);

        let list_response = list_task
            .await
            .expect("list task should join")
            .expect("list should resolve");
        match list_response {
            LocalDaemonResponse::ProviderProcessesListed { processes } => {
                assert_eq!(processes.len(), 2);
            }
            _ => panic!("unexpected provider process list response"),
        }
    }

    #[tokio::test]
    async fn provider_process_projection_updates_after_teardown() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );
        app.list_provider_processes(None)
            .expect("process list should warm projection");
        app.teardown_provider_processes(None, false)
            .expect("teardown should update projection");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;
        let list_request =
            LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest {
                provider: None,
            });
        let list_command = KernelCommand::from_local_request(
            "cmd-process-post-teardown-projection",
            None,
            None,
            &list_request,
        );
        let list_router = router.clone();
        let list_task =
            tokio::spawn(async move { list_router.dispatch(list_command, list_request).await });

        tokio::task::yield_now().await;
        assert!(list_task.is_finished());
        drop(app_guard);

        let list_response = list_task
            .await
            .expect("list task should join")
            .expect("list should resolve");
        match list_response {
            LocalDaemonResponse::ProviderProcessesListed { processes } => {
                assert!(processes.is_empty());
            }
            _ => panic!("unexpected provider process list response"),
        }
    }

    #[tokio::test]
    async fn teardown_provider_processes_refreshes_session_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        });
        let launch_command = KernelCommand::from_local_request(
            "cmd-teardown-refresh-launch",
            None,
            None,
            &launch_request,
        );
        router
            .dispatch(launch_command, launch_request)
            .await
            .expect("provider launch should be accepted");

        let teardown_request =
            LocalDaemonRequest::TeardownProviderProcesses(TeardownProviderProcessesRequest {
                provider: None,
                force: false,
            });
        let teardown_command = KernelCommand::from_local_request(
            "cmd-teardown-refresh",
            None,
            None,
            &teardown_request,
        );
        let teardown_response = router
            .dispatch(teardown_command, teardown_request)
            .await
            .expect("safe process teardown should succeed");
        match teardown_response {
            LocalDaemonResponse::ProviderProcessesTornDown { processes } => {
                assert_eq!(processes.len(), 1);
            }
            _ => panic!("unexpected teardown response"),
        }

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-teardown-refresh-state",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        let state_response = timeout(Duration::from_millis(100), state_task)
            .await
            .expect("post-teardown session state should not wait for the app lock")
            .expect("state task should join")
            .expect("state should resolve");
        drop(app_guard);

        match state_response {
            LocalDaemonResponse::SessionState { session } => {
                assert_eq!(session.id(), session_id);
                assert_eq!(session.active_provider_run_id(), None);
            }
            _ => panic!("unexpected session state response"),
        }
    }

    #[tokio::test]
    async fn get_provider_catalog_uses_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        app.cache_provider_catalog(OpenCodeProviderCatalog {
            all: vec![OpenCodeProviderInfo {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                remote_machine_aliases: Vec::new(),
                models: Default::default(),
            }],
            default: Default::default(),
            connected: vec!["codex".to_string()],
        });
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let app_guard = app.lock().await;
        let catalog_request = LocalDaemonRequest::GetProviderCatalog(GetProviderCatalogRequest);
        let catalog_command = KernelCommand::from_local_request(
            "cmd-provider-catalog-projection",
            None,
            None,
            &catalog_request,
        );
        let catalog_router = router.clone();
        let catalog_task = tokio::spawn(async move {
            catalog_router
                .dispatch(catalog_command, catalog_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(
            catalog_task.is_finished(),
            "warmed GetProviderCatalog should be served from projection without app lock access"
        );
        drop(app_guard);

        let catalog_response = catalog_task
            .await
            .expect("catalog task should join")
            .expect("catalog should resolve");
        match catalog_response {
            LocalDaemonResponse::ProviderCatalog { catalog } => {
                assert_eq!(catalog.connected, vec!["codex"]);
            }
            _ => panic!("unexpected provider catalog response"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn relay_configure_invalidates_provider_catalog_projection() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        app.cache_provider_catalog(OpenCodeProviderCatalog {
            all: vec![OpenCodeProviderInfo {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                remote_machine_aliases: Vec::new(),
                models: Default::default(),
            }],
            default: Default::default(),
            connected: vec!["codex".to_string()],
        });
        app.configure_relay(None, None)
            .expect("relay configure should invalidate provider catalog projection");
        app.invalidate_provider_catalog_projection();

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;
        let catalog_request = LocalDaemonRequest::GetProviderCatalog(GetProviderCatalogRequest);
        let catalog_command = KernelCommand::from_local_request(
            "cmd-provider-catalog-invalidated",
            None,
            None,
            &catalog_request,
        );
        let catalog_router = router.clone();
        let catalog_task = tokio::spawn(async move {
            catalog_router
                .dispatch(catalog_command, catalog_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(
            !catalog_task.is_finished(),
            "relay configuration should invalidate warmed provider catalog projection"
        );
        drop(app_guard);
        let _ = catalog_task
            .await
            .expect("catalog task should join after app lock is released");
    }

    #[tokio::test]
    async fn list_sessions_uses_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-list-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm the projection");

        let app_guard = app.lock().await;
        let projected_list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let projected_list_command = KernelCommand::from_local_request(
            "cmd-list-projection",
            None,
            None,
            &projected_list_request,
        );
        let list_router = router.clone();
        let list_task = tokio::spawn(async move {
            list_router
                .dispatch(projected_list_command, projected_list_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(
            list_task.is_finished(),
            "warmed ListSessions should be served from the session list projection without app lock access"
        );

        drop(app_guard);
        let list_response = list_task
            .await
            .expect("list task should join")
            .expect("list should resolve");
        match list_response {
            LocalDaemonResponse::SessionsListed { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].id(), session_id);
            }
            _ => panic!("unexpected list response"),
        }
    }

    #[tokio::test]
    async fn get_session_state_uses_list_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-list-state-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should hydrate per-session projection entries");

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-list-state-projection",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        tokio::task::yield_now().await;
        assert!(
            state_task.is_finished(),
            "ListSessions warm-up should hydrate GetSessionState projection entries without app lock access"
        );

        drop(app_guard);
        let state_response = state_task
            .await
            .expect("state task should join")
            .expect("state should resolve");
        match state_response {
            LocalDaemonResponse::SessionState { session } => {
                assert_eq!(session.id(), session_id);
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn missing_session_state_uses_list_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command = KernelCommand::from_local_request(
            "cmd-list-missing-state-warm",
            None,
            None,
            &list_request,
        );
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm empty session projection");

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: "missing-session".to_string(),
        });
        let state_command = KernelCommand::from_local_request(
            "cmd-missing-state-projection",
            None,
            None,
            &state_request,
        );
        let state_router = router.clone();
        let state_task =
            tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

        let error = timeout(Duration::from_millis(100), state_task)
            .await
            .expect("missing state should not wait for the app lock")
            .expect("state task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn resolve_session_uses_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let session_prefix = session_id[..8].to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-resolve-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm visible session projection entries");

        let app_guard = app.lock().await;
        let resolve_request = LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: session_prefix,
            workspace_id: Some("workspace".to_string()),
        });
        let resolve_command = KernelCommand::from_local_request(
            "cmd-resolve-projection",
            None,
            None,
            &resolve_request,
        );
        let resolve_router = router.clone();
        let resolve_task = tokio::spawn(async move {
            resolve_router
                .dispatch(resolve_command, resolve_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(
            resolve_task.is_finished(),
            "warmed ResolveSession should return from session projection without app lock access"
        );

        drop(app_guard);
        let resolve_response = resolve_task
            .await
            .expect("resolve task should join")
            .expect("resolve should succeed");
        match resolve_response {
            LocalDaemonResponse::SessionResolved { session } => {
                assert_eq!(session.id(), session_id);
            }
            _ => panic!("unexpected resolve response"),
        }
    }

    #[tokio::test]
    async fn missing_resolve_session_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command = KernelCommand::from_local_request(
            "cmd-resolve-missing-warm",
            None,
            None,
            &list_request,
        );
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm empty session projection");

        let app_guard = app.lock().await;
        let resolve_request = LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
            session_ref: "missing-session".to_string(),
            workspace_id: None,
        });
        let resolve_command = KernelCommand::from_local_request(
            "cmd-resolve-missing-projection",
            None,
            None,
            &resolve_request,
        );
        let resolve_router = router.clone();
        let resolve_task = tokio::spawn(async move {
            resolve_router
                .dispatch(resolve_command, resolve_request)
                .await
        });

        let error = timeout(Duration::from_millis(100), resolve_task)
            .await
            .expect("missing resolve should not wait for the app lock")
            .expect("resolve task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_session_inspection_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command = KernelCommand::from_local_request(
            "cmd-inspection-missing-warm",
            None,
            None,
            &list_request,
        );
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm empty session projection");

        let app_guard = app.lock().await;
        let inspection_request = LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: "missing-session".to_string(),
        });
        let inspection_command = KernelCommand::from_local_request(
            "cmd-inspection-missing-projection",
            None,
            None,
            &inspection_request,
        );
        let inspection_router = router.clone();
        let inspection_task = tokio::spawn(async move {
            inspection_router
                .dispatch(inspection_command, inspection_request)
                .await
        });

        let error = timeout(Duration::from_millis(100), inspection_task)
            .await
            .expect("missing inspection should not wait for the app lock")
            .expect("inspection task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_session_history_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command = KernelCommand::from_local_request(
            "cmd-history-missing-warm",
            None,
            None,
            &list_request,
        );
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm empty session projection");

        let app_guard = app.lock().await;
        let history_request = LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
            session_id: "missing-session".to_string(),
            agent_id: None,
            round_count: Some(10),
            max_chars: None,
            before_entry_index: None,
            before_entry_char_offset: None,
        });
        let history_command = KernelCommand::from_local_request(
            "cmd-history-missing-projection",
            None,
            None,
            &history_request,
        );
        let history_router = router.clone();
        let history_task = tokio::spawn(async move {
            history_router
                .dispatch(history_command, history_request)
                .await
        });

        let error = timeout(Duration::from_millis(100), history_task)
            .await
            .expect("missing history should not wait for the app lock")
            .expect("history task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_terminal_output_session_uses_warmed_projection_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-pump-missing-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm empty session projection");

        let app_guard = app.lock().await;
        let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
            session_id: "missing-session".to_string(),
            attachment_id: "missing-attachment".to_string(),
        });
        let pump_command = KernelCommand::from_local_request(
            "cmd-pump-missing-projection",
            None,
            None,
            &pump_request,
        );
        let pump_router = router.clone();
        let pump_task =
            tokio::spawn(async move { pump_router.dispatch(pump_command, pump_request).await });

        let error = timeout(Duration::from_millis(100), pump_task)
            .await
            .expect("missing terminal output session should not wait for the app lock")
            .expect("pump task should join")
            .expect_err("missing session should fail");
        drop(app_guard);

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn missing_terminal_output_attachment_uses_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-pump-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command = KernelCommand::from_local_request(
            "cmd-pump-attachment-warm",
            None,
            None,
            &list_request,
        );
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm session projection");

        let app_guard = app.lock().await;
        let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
            session_id: session_id.clone(),
            attachment_id: "missing-attachment".to_string(),
        });
        let pump_command = KernelCommand::from_local_request(
            "cmd-pump-attachment-projection",
            None,
            None,
            &pump_request,
        );
        let pump_router = router.clone();
        let pump_task =
            tokio::spawn(async move { pump_router.dispatch(pump_command, pump_request).await });

        let error = timeout(Duration::from_millis(100), pump_task)
            .await
            .expect("missing terminal output attachment should not wait for the app lock")
            .expect("pump task should join")
            .expect_err("missing attachment should fail");
        drop(app_guard);

        match error {
            DaemonError::AttachmentNotInSession {
                session_id: error_session_id,
                attachment_id,
            } => {
                assert_eq!(error_session_id, session_id);
                assert_eq!(attachment_id, "missing-attachment");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn terminal_output_without_active_run_drains_store_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-pump-buffered",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.fan_out_output(
            &session_id,
            "provider-run-buffered",
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            vec![attachment.id().to_string()],
            b"buffered output",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-pump-drain-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm session projection");

        let app_guard = app.lock().await;
        let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
        });
        let pump_command = KernelCommand::from_local_request(
            "cmd-pump-drain-projection",
            None,
            None,
            &pump_request,
        );
        let pump_router = router.clone();
        let pump_task =
            tokio::spawn(async move { pump_router.dispatch(pump_command, pump_request).await });

        let pump_response = timeout(Duration::from_millis(100), pump_task)
            .await
            .expect("buffered terminal output drain should not wait for the app lock")
            .expect("pump task should join")
            .expect("pump should succeed");
        drop(app_guard);

        match pump_response {
            LocalDaemonResponse::TerminalOutput { records } => {
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].session_id, session_id);
                assert_eq!(records[0].bytes, b"buffered output".to_vec());
            }
            _ => panic!("unexpected pump response"),
        }
    }

    #[tokio::test]
    async fn terminal_output_with_active_run_enters_provider_runtime_lane() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-pump-active",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let provider_run_id = launch_test_provider(
            &mut app,
            &session_id,
            agent.id(),
            "dev-stub",
            "claude-code",
            "sonnet",
        )
        .id()
        .to_string();

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-pump-active-warm", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm active provider projection");

        let permit = router
            .provider_runtime_lanes
            .acquire(&provider_run_id)
            .await;
        let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
        });
        let pump_command =
            KernelCommand::from_local_request("cmd-pump-active-lane", None, None, &pump_request);
        let pump_router = router.clone();
        let pump_task =
            tokio::spawn(async move { pump_router.dispatch(pump_command, pump_request).await });

        tokio::task::yield_now().await;
        assert!(
            !pump_task.is_finished(),
            "active terminal output pumping should wait behind the provider-run runtime lane"
        );

        drop(permit);
        let pump_response = pump_task
            .await
            .expect("pump task should join")
            .expect("pump should succeed");
        match pump_response {
            LocalDaemonResponse::TerminalOutput { records } => {
                assert!(records.is_empty());
            }
            _ => panic!("unexpected pump response"),
        }
    }

    #[tokio::test]
    async fn terminal_output_with_projected_inactive_run_drains_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-pump-parked",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let mut projected_session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("session snapshot should be available");
        let mut provider_run = RuntimeProviderRun::from_control_capability_inference(
            "provider-run-parked",
            session_id.clone(),
            Some(agent.id().to_string()),
            "dev-stub".to_string(),
        );
        provider_run.mark_parked();
        projected_session.set_active_provider_run(Some(provider_run.id().to_string()));
        app.fan_out_output(
            &session_id,
            provider_run.id(),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            None,
            vec![attachment.id().to_string()],
            b"parked buffered output",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        router.session_projection.update(projected_session);
        router.provider_run_projection.update(provider_run.clone());

        let app_guard = app.lock().await;
        let permit = router
            .provider_runtime_lanes
            .acquire(provider_run.id())
            .await;
        let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
        });
        let pump_command = KernelCommand::from_local_request(
            "cmd-pump-parked-projection",
            None,
            None,
            &pump_request,
        );
        let pump_router = router.clone();
        let pump_task =
            tokio::spawn(async move { pump_router.dispatch(pump_command, pump_request).await });

        let pump_response = timeout(Duration::from_millis(100), pump_task)
            .await
            .expect("inactive run drain should not wait for app lock or provider lane")
            .expect("pump task should join")
            .expect("pump should succeed");
        drop(permit);
        drop(app_guard);

        match pump_response {
            LocalDaemonResponse::TerminalOutput { records } => {
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].session_id, session_id);
                assert_eq!(records[0].bytes, b"parked buffered output".to_vec());
            }
            _ => panic!("unexpected pump response"),
        }
    }

    #[tokio::test]
    async fn session_inspection_reads_use_warmed_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let spawn_request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("reviewer".to_string()),
            provider: Some("claude-code".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        });
        let spawn_command =
            KernelCommand::from_local_request("cmd-inspection-spawn", None, None, &spawn_request);
        router
            .dispatch(spawn_command, spawn_request)
            .await
            .expect("spawn should refresh the session projection");

        let create_workflow_request = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session_id.clone(),
            alias: Some("inspection".to_string()),
        });
        let create_workflow_command = KernelCommand::from_local_request(
            "cmd-inspection-workflow",
            None,
            None,
            &create_workflow_request,
        );
        let workflow_id = match router
            .dispatch(create_workflow_command, create_workflow_request)
            .await
            .expect("workflow should create")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow.id().to_string(),
            _ => panic!("unexpected workflow response"),
        };

        let app_guard = app.lock().await;
        let list_agents_request = LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: session_id.clone(),
        });
        let list_workflows_request = LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session_id.clone(),
        });
        let resolve_workflow_request =
            LocalDaemonRequest::ResolveWorkflow(ResolveWorkflowRequest {
                session_id: session_id.clone(),
                workflow_ref: "inspection".to_string(),
            });
        let list_runs_request = LocalDaemonRequest::ListWorkflowRuns(ListWorkflowRunsRequest {
            session_id: session_id.clone(),
            workflow_ref: Some("inspection".to_string()),
        });
        let list_watchdogs_request =
            LocalDaemonRequest::ListWorkflowWatchdogs(ListWorkflowWatchdogsRequest {
                session_id: session_id.clone(),
                workflow_ref: Some("inspection".to_string()),
            });

        let list_agents_router = router.clone();
        let list_agents_task = tokio::spawn(async move {
            let command = KernelCommand::from_local_request(
                "cmd-inspection-agents",
                None,
                None,
                &list_agents_request,
            );
            list_agents_router
                .dispatch(command, list_agents_request)
                .await
        });
        let list_workflows_router = router.clone();
        let list_workflows_task = tokio::spawn(async move {
            let command = KernelCommand::from_local_request(
                "cmd-inspection-workflows",
                None,
                None,
                &list_workflows_request,
            );
            list_workflows_router
                .dispatch(command, list_workflows_request)
                .await
        });
        let resolve_workflow_router = router.clone();
        let resolve_workflow_task = tokio::spawn(async move {
            let command = KernelCommand::from_local_request(
                "cmd-inspection-resolve-workflow",
                None,
                None,
                &resolve_workflow_request,
            );
            resolve_workflow_router
                .dispatch(command, resolve_workflow_request)
                .await
        });
        let list_runs_router = router.clone();
        let list_runs_task = tokio::spawn(async move {
            let command = KernelCommand::from_local_request(
                "cmd-inspection-runs",
                None,
                None,
                &list_runs_request,
            );
            list_runs_router.dispatch(command, list_runs_request).await
        });
        let list_watchdogs_router = router.clone();
        let list_watchdogs_task = tokio::spawn(async move {
            let command = KernelCommand::from_local_request(
                "cmd-inspection-watchdogs",
                None,
                None,
                &list_watchdogs_request,
            );
            list_watchdogs_router
                .dispatch(command, list_watchdogs_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(list_agents_task.is_finished());
        assert!(list_workflows_task.is_finished());
        assert!(resolve_workflow_task.is_finished());
        assert!(list_runs_task.is_finished());
        assert!(list_watchdogs_task.is_finished());
        drop(app_guard);

        match list_agents_task
            .await
            .expect("list agents task should join")
            .expect("agents should list")
        {
            LocalDaemonResponse::AgentsListed { agents } => {
                assert_eq!(agents.len(), 2);
            }
            _ => panic!("unexpected agents response"),
        }
        match list_workflows_task
            .await
            .expect("list workflows task should join")
            .expect("workflows should list")
        {
            LocalDaemonResponse::WorkflowsListed { workflows } => {
                assert_eq!(workflows.len(), 1);
                assert_eq!(workflows[0].id(), workflow_id);
            }
            _ => panic!("unexpected workflows response"),
        }
        match resolve_workflow_task
            .await
            .expect("resolve workflow task should join")
            .expect("workflow should resolve")
        {
            LocalDaemonResponse::WorkflowResolved { workflow } => {
                assert_eq!(workflow.id(), workflow_id);
            }
            _ => panic!("unexpected workflow resolve response"),
        }
        match list_runs_task
            .await
            .expect("list runs task should join")
            .expect("workflow runs should list")
        {
            LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => {
                assert!(workflow_runs.is_empty());
            }
            _ => panic!("unexpected workflow runs response"),
        }
        match list_watchdogs_task
            .await
            .expect("list watchdogs task should join")
            .expect("workflow watchdogs should list")
        {
            LocalDaemonResponse::WorkflowWatchdogsListed { watchdogs } => {
                assert!(watchdogs.is_empty());
            }
            _ => panic!("unexpected workflow watchdogs response"),
        }
    }

    #[tokio::test]
    async fn warmed_session_list_projection_tracks_create_and_delete_responses() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_command =
            KernelCommand::from_local_request("cmd-list-empty", None, None, &list_request);
        router
            .dispatch(list_command, list_request)
            .await
            .expect("initial list should warm an empty projection");

        let create_request = LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
            "workspace-list-projection",
            "worktree-list-projection",
        ));
        let create_command =
            KernelCommand::from_local_request("cmd-create-for-list", None, None, &create_request);
        let created_session_id = match router
            .dispatch(create_command, create_request)
            .await
            .expect("create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session.id().to_string(),
            _ => panic!("unexpected create response"),
        };

        let app_guard = app.lock().await;
        let projected_list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let projected_list_command = KernelCommand::from_local_request(
            "cmd-list-after-create",
            None,
            None,
            &projected_list_request,
        );
        let list_router = router.clone();
        let list_task = tokio::spawn(async move {
            list_router
                .dispatch(projected_list_command, projected_list_request)
                .await
        });
        tokio::task::yield_now().await;
        assert!(list_task.is_finished());
        drop(app_guard);
        let list_response = list_task
            .await
            .expect("list task should join")
            .expect("list should resolve");
        match list_response {
            LocalDaemonResponse::SessionsListed { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].id(), created_session_id);
            }
            _ => panic!("unexpected list response"),
        }

        let delete_request = LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: created_session_id.clone(),
            workspace_id: None,
        });
        let delete_command =
            KernelCommand::from_local_request("cmd-delete-for-list", None, None, &delete_request);
        router
            .dispatch(delete_command, delete_request)
            .await
            .expect("delete should succeed");

        let app_guard = app.lock().await;
        let projected_list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let projected_list_command = KernelCommand::from_local_request(
            "cmd-list-after-delete",
            None,
            None,
            &projected_list_request,
        );
        let list_router = router.clone();
        let list_task = tokio::spawn(async move {
            list_router
                .dispatch(projected_list_command, projected_list_request)
                .await
        });
        tokio::task::yield_now().await;
        assert!(list_task.is_finished());
        drop(app_guard);
        let list_response = list_task
            .await
            .expect("list task should join")
            .expect("list should resolve");
        match list_response {
            LocalDaemonResponse::SessionsListed { sessions } => {
                assert!(sessions.is_empty());
            }
            _ => panic!("unexpected list response"),
        }
    }

    #[tokio::test]
    async fn delete_kernel_removes_current_kernel_sessions_from_projection() {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-kernel-delete",
                "worktree-kernel-delete",
            ))
            .expect("session should create");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_response = router
            .dispatch(
                KernelCommand::from_local_request(
                    "cmd-list-before-kernel-delete",
                    None,
                    None,
                    &list_request,
                ),
                list_request,
            )
            .await
            .expect("list before kernel delete should resolve");
        match list_response {
            LocalDaemonResponse::SessionsListed { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].id(), session_id);
            }
            _ => panic!("unexpected list response"),
        }

        let delete_request = LocalDaemonRequest::DeleteKernel(DeleteKernelRequest);
        let delete_response = router
            .dispatch(
                KernelCommand::from_local_request("cmd-delete-kernel", None, None, &delete_request),
                delete_request,
            )
            .await
            .expect("kernel delete should resolve");
        match delete_response {
            LocalDaemonResponse::KernelDeleted {
                deleted_sessions, ..
            } => assert_eq!(deleted_sessions.len(), 1),
            _ => panic!("unexpected kernel delete response"),
        }

        let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let list_response = router
            .dispatch(
                KernelCommand::from_local_request(
                    "cmd-list-after-kernel-delete",
                    None,
                    None,
                    &list_request,
                ),
                list_request,
            )
            .await
            .expect("list after kernel delete should resolve");
        match list_response {
            LocalDaemonResponse::SessionsListed { sessions } => assert!(sessions.is_empty()),
            _ => panic!("unexpected list response"),
        }
    }

    #[tokio::test]
    async fn remote_session_requests_require_membership() {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-membership",
                "worktree-a",
            ))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let denied = router
            .dispatch(
                remote_command_for_request(&request, Some("user-2")),
                request,
            )
            .await
            .expect_err("non-member should be rejected");
        assert!(matches!(
            denied,
            DaemonError::SessionAccessDenied {
                session_id: denied_session,
                user_id
            } if denied_session == session_id && user_id == "user-2"
        ));

        let request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest { session_id });
        let missing = router
            .dispatch(remote_command_for_request(&request, None), request)
            .await
            .expect_err("remote session request without user id should be rejected");
        assert!(matches!(
            missing,
            DaemonError::MissingSessionCallerIdentity { .. }
        ));
    }

    #[tokio::test]
    async fn remote_session_list_is_filtered_to_memberships() {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let session_a = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-membership",
                "worktree-a",
            ))
            .expect("session a should be created");
        let session_b = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-membership",
                "worktree-b",
            ))
            .expect("session b should be created");
        let session_a_id = session_a.id().to_string();
        let session_b_id = session_b.id().to_string();
        let (_, invite) = app
            .sessions_mut()
            .create_session_invite(
                &session_b_id,
                "invite-user-2".to_string(),
                DEFAULT_LOCAL_USER_ID.to_string(),
                None,
                Some(1),
            )
            .expect("invite should be created");
        app.sessions_mut()
            .join_session_invite(&session_b_id, invite.invite_id(), "user-2".to_string(), 1)
            .expect("user should join session b");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
        let response = router
            .dispatch(
                remote_command_for_request(&request, Some("user-2")),
                request,
            )
            .await
            .expect("member list should succeed");
        match response {
            LocalDaemonResponse::SessionsListed { sessions } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].id(), session_b_id);
                assert_ne!(sessions[0].id(), session_a_id);
            }
            _ => panic!("unexpected list response"),
        }
    }

    #[tokio::test]
    async fn remote_owned_session_objects_record_caller_user() {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-ownership",
                "worktree-ownership",
            ))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let (_, invite) = app
            .sessions_mut()
            .create_session_invite(
                &session_id,
                "invite-user-2".to_string(),
                DEFAULT_LOCAL_USER_ID.to_string(),
                None,
                Some(1),
            )
            .expect("invite should be created");
        app.sessions_mut()
            .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
            .expect("user should join session");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

        let spawn_one = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("owned-a".to_string()),
            provider: Some("dev-stub".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        });
        let agent_one = match router
            .dispatch(
                remote_command_for_request(&spawn_one, Some("user-2")),
                spawn_one,
            )
            .await
            .expect("agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            other => panic!("unexpected spawn response: {other:?}"),
        };
        assert_eq!(agent_one.owner_user_id(), "user-2");

        let spawn_two = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("owned-b".to_string()),
            provider: Some("dev-stub".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        });
        let agent_two = match router
            .dispatch(
                remote_command_for_request(&spawn_two, Some("user-2")),
                spawn_two,
            )
            .await
            .expect("second agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            other => panic!("unexpected spawn response: {other:?}"),
        };
        assert_eq!(agent_two.owner_user_id(), "user-2");

        let create_workflow = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session_id.clone(),
            alias: Some("owned-flow".to_string()),
        });
        let workflow_id = match router
            .dispatch(
                remote_command_for_request(&create_workflow, Some("user-2")),
                create_workflow,
            )
            .await
            .expect("workflow should create")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow.id().to_string(),
            other => panic!("unexpected workflow response: {other:?}"),
        };

        let add_first_node = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            agent_id: agent_one.id().to_string(),
            expected_workflow_revision: None,
        });
        let first_node = match router
            .dispatch(
                remote_command_for_request(&add_first_node, Some("user-2")),
                add_first_node,
            )
            .await
            .expect("first node should add")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            other => panic!("unexpected node response: {other:?}"),
        };
        assert_eq!(first_node.owner_user_id(), "user-2");
        assert_eq!(first_node.public_label(), agent_one.id());

        let add_second_node = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            agent_id: agent_two.id().to_string(),
            expected_workflow_revision: None,
        });
        let second_node = match router
            .dispatch(
                remote_command_for_request(&add_second_node, Some("user-2")),
                add_second_node,
            )
            .await
            .expect("second node should add")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            other => panic!("unexpected node response: {other:?}"),
        };
        assert_eq!(second_node.owner_user_id(), "user-2");

        let create_endpoint =
            LocalDaemonRequest::CreateWorkflowEndpoint(CreateWorkflowEndpointRequest {
                session_id: session_id.clone(),
                workflow_ref: workflow_id.clone(),
                entry_node_id: first_node.id().to_string(),
                alias: Some("owned-entry".to_string()),
                expected_workflow_revision: None,
            });
        let endpoint = match router
            .dispatch(
                remote_command_for_request(&create_endpoint, Some("user-2")),
                create_endpoint,
            )
            .await
            .expect("endpoint should create")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            other => panic!("unexpected endpoint response: {other:?}"),
        };
        assert_eq!(endpoint.owner_user_id(), "user-2");

        let add_edge = LocalDaemonRequest::AddWorkflowEdge(AddWorkflowEdgeRequest {
            session_id,
            workflow_ref: workflow_id,
            from_node_id: first_node.id().to_string(),
            to_node_id: second_node.id().to_string(),
            output_schema_ref: None,
            validation_policy: None,
            expected_workflow_revision: None,
        });
        let edge = match router
            .dispatch(
                remote_command_for_request(&add_edge, Some("user-2")),
                add_edge,
            )
            .await
            .expect("edge should add")
        {
            LocalDaemonResponse::WorkflowEdgeAdded { edge, .. } => edge,
            other => panic!("unexpected edge response: {other:?}"),
        };
        assert_eq!(edge.created_by_user_id(), "user-2");
    }

    #[tokio::test]
    async fn remote_created_session_records_caller_as_owner_and_default_agent_owner() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
        let create_session = LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
            "workspace-remote-session-owner",
            "worktree-remote-session-owner",
        ));

        let (session, agent) = match router
            .dispatch(
                remote_command_for_request(&create_session, Some("user-2")),
                create_session,
            )
            .await
            .expect("remote create session should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
            other => panic!("unexpected create session response: {other:?}"),
        };

        assert_eq!(session.owner_user_id(), "user-2");
        assert!(session.has_member("user-2"));
        assert!(!session.has_member(DEFAULT_LOCAL_USER_ID));
        assert_eq!(agent.owner_user_id(), "user-2");
    }

    #[tokio::test]
    async fn remote_user_cannot_control_other_users_agents_or_endpoint() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-authz",
                "worktree-authz",
            ))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let local_agent = spawn_test_agent(&mut app, &session_id, "local-owned", "dev-stub");
        let local_agent_id = local_agent.id().to_string();
        let workflow = app
            .sessions_mut()
            .create_workflow(&session_id, Some("authz-flow".to_string()))
            .expect("workflow should be created");
        let workflow_id = workflow.id().to_string();
        let local_node = app
            .sessions_mut()
            .add_workflow_node_owned(
                &session_id,
                &workflow_id,
                &local_agent_id,
                DEFAULT_LOCAL_USER_ID.to_string(),
                local_agent_id.clone(),
            )
            .expect("node should be created");
        let endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                &session_id,
                &workflow_id,
                local_node.id(),
                Some("local-entry".to_string()),
            )
            .expect("endpoint should be created");
        app.sessions_mut()
            .set_workflow_endpoint_owner(
                &session_id,
                &workflow_id,
                endpoint.id(),
                DEFAULT_LOCAL_USER_ID.to_string(),
            )
            .expect("endpoint owner should be set");
        for (invite_id, user_id) in [("invite-user-2", "user-2"), ("invite-user-3", "user-3")] {
            let (_, invite) = app
                .sessions_mut()
                .create_session_invite(
                    &session_id,
                    invite_id.to_string(),
                    DEFAULT_LOCAL_USER_ID.to_string(),
                    None,
                    Some(1),
                )
                .expect("invite should be created");
            app.sessions_mut()
                .join_session_invite(&session_id, invite.invite_id(), user_id.to_string(), 1)
                .expect("user should join session");
        }

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

        let focus = LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session_id.clone(),
            agent_id: local_agent_id.clone(),
        });
        assert_ownership_denied(
            router
                .dispatch(remote_command_for_request(&focus, Some("user-2")), focus)
                .await
                .expect_err("other user should not focus local agent"),
            "user-2",
            DEFAULT_LOCAL_USER_ID,
        );

        let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: "remote-attachment".to_string(),
            target_agent_id: Some(local_agent_id.clone()),
            prompt: "should be denied".to_string(),
            attachments: Vec::new(),
        });
        assert_ownership_denied(
            router
                .dispatch(remote_command_for_request(&submit, Some("user-2")), submit)
                .await
                .expect_err("other user should not submit to local agent"),
            "user-2",
            DEFAULT_LOCAL_USER_ID,
        );

        let add_local_node = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            agent_id: local_agent_id.clone(),
            expected_workflow_revision: None,
        });
        assert_ownership_denied(
            router
                .dispatch(
                    remote_command_for_request(&add_local_node, Some("user-2")),
                    add_local_node,
                )
                .await
                .expect_err("other user should not add local agent as node"),
            "user-2",
            DEFAULT_LOCAL_USER_ID,
        );

        let invoke = LocalDaemonRequest::InvokeWorkflowEndpoint(InvokeWorkflowEndpointRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            endpoint_ref: endpoint.id().to_string(),
            prompt: Some("should be denied".to_string()),
        });
        assert_ownership_denied(
            router
                .dispatch(remote_command_for_request(&invoke, Some("user-2")), invoke)
                .await
                .expect_err("other user should not invoke local endpoint"),
            "user-2",
            DEFAULT_LOCAL_USER_ID,
        );

        let spawn_user_two = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("user-two-owned".to_string()),
            provider: Some("dev-stub".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            machine_ref: None,
            worktree_placement: None,
        });
        let user_two_agent = match router
            .dispatch(
                remote_command_for_request(&spawn_user_two, Some("user-2")),
                spawn_user_two,
            )
            .await
            .expect("user two should spawn own agent")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            other => panic!("unexpected spawn response: {other:?}"),
        };
        let add_user_two_node = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            agent_id: user_two_agent.id().to_string(),
            expected_workflow_revision: None,
        });
        let user_two_node = match router
            .dispatch(
                remote_command_for_request(&add_user_two_node, Some("user-2")),
                add_user_two_node,
            )
            .await
            .expect("user two should add own node")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            other => panic!("unexpected node response: {other:?}"),
        };

        let add_cross_owner_edge = LocalDaemonRequest::AddWorkflowEdge(AddWorkflowEdgeRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            from_node_id: local_node.id().to_string(),
            to_node_id: user_two_node.id().to_string(),
            output_schema_ref: None,
            validation_policy: None,
            expected_workflow_revision: None,
        });
        let edge = match router
            .dispatch(
                remote_command_for_request(&add_cross_owner_edge, Some("user-2")),
                add_cross_owner_edge,
            )
            .await
            .expect("edge touching caller node should be allowed")
        {
            LocalDaemonResponse::WorkflowEdgeAdded { edge, .. } => edge,
            other => panic!("unexpected edge response: {other:?}"),
        };
        assert_eq!(edge.created_by_user_id(), "user-2");

        let remove_edge = LocalDaemonRequest::RemoveWorkflowEdge(RemoveWorkflowEdgeRequest {
            session_id,
            workflow_ref: workflow_id,
            edge_id: edge.id().to_string(),
            expected_workflow_revision: None,
        });
        assert_ownership_denied(
            router
                .dispatch(
                    remote_command_for_request(&remove_edge, Some("user-3")),
                    remove_edge,
                )
                .await
                .expect_err("unrelated user should not remove edge"),
            "user-3",
            "user-2",
        );
    }

    #[tokio::test]
    async fn remote_session_projection_redacts_other_users_private_agent_and_workflow_state() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-redaction",
                "worktree-redaction",
            ))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let (_, invite) = app
            .sessions_mut()
            .create_session_invite(
                &session_id,
                "invite-user-2".to_string(),
                DEFAULT_LOCAL_USER_ID.to_string(),
                None,
                Some(1),
            )
            .expect("invite should be created");
        app.sessions_mut()
            .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
            .expect("user should join session");
        let local_agent = spawn_test_agent(&mut app, &session_id, "local-owned", "dev-stub");
        let user_two_agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, "dev-stub")
                    .with_alias("user-two-owned")
                    .with_owner_user_id("user-2"),
            )
            .expect("user two agent should be created");
        let workflow = app
            .sessions_mut()
            .create_workflow(&session_id, Some("redaction-flow".to_string()))
            .expect("workflow should be created");
        let workflow_id = workflow.id().to_string();
        let local_node = app
            .sessions_mut()
            .add_workflow_node_owned(
                &session_id,
                &workflow_id,
                local_agent.id(),
                DEFAULT_LOCAL_USER_ID.to_string(),
                "local public".to_string(),
            )
            .expect("local node should be created");
        app.sessions_mut()
            .update_workflow_node_instructions(
                &session_id,
                &workflow_id,
                local_node.id(),
                Some("local private prompt".to_string()),
            )
            .expect("local node instructions should update");
        let user_two_node = app
            .sessions_mut()
            .add_workflow_node_owned(
                &session_id,
                &workflow_id,
                user_two_agent.id(),
                "user-2".to_string(),
                "user two public".to_string(),
            )
            .expect("user two node should be created");
        app.sessions_mut()
            .update_workflow_node_instructions(
                &session_id,
                &workflow_id,
                user_two_node.id(),
                Some("user two private prompt".to_string()),
            )
            .expect("user two node instructions should update");
        let provider_run = launch_test_provider(
            &mut app,
            &session_id,
            local_agent.id(),
            "dev-stub",
            "dev-stub",
            "redaction-model",
        );
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let redacted_session = match router
            .dispatch(
                remote_command_for_request(&state_request, Some("user-2")),
                state_request,
            )
            .await
            .expect("member should read redacted session state")
        {
            LocalDaemonResponse::SessionState { session } => session,
            other => panic!("unexpected session response: {other:?}"),
        };
        assert_eq!(redacted_session.agents().len(), 1);
        assert_eq!(redacted_session.agents()[0].id(), user_two_agent.id());
        let redacted_workflow = redacted_session
            .workflows()
            .iter()
            .find(|workflow| workflow.id() == workflow_id)
            .expect("workflow graph should remain visible");
        assert_eq!(redacted_workflow.nodes().len(), 2);
        let redacted_local_node = redacted_workflow
            .node(local_node.id())
            .expect("other user's node should remain visible");
        assert_eq!(redacted_local_node.public_label(), "local public");
        assert_eq!(redacted_local_node.instructions(), None);
        let visible_user_two_node = redacted_workflow
            .node(user_two_node.id())
            .expect("own node should remain visible");
        assert_eq!(
            visible_user_two_node.instructions(),
            Some("user two private prompt")
        );

        let list_agents = LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: session_id.clone(),
        });
        match router
            .dispatch(
                remote_command_for_request(&list_agents, Some("user-2")),
                list_agents,
            )
            .await
            .expect("member should list own agents")
        {
            LocalDaemonResponse::AgentsListed { agents } => {
                assert_eq!(agents.len(), 1);
                assert_eq!(agents[0].id(), user_two_agent.id());
            }
            other => panic!("unexpected agents response: {other:?}"),
        }

        let provider_request = LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
            provider_run_id: provider_run.id().to_string(),
        });
        assert_ownership_denied(
            router
                .dispatch(
                    remote_command_for_request(&provider_request, Some("user-2")),
                    provider_request,
                )
                .await
                .expect_err("other user should not read provider run"),
            "user-2",
            DEFAULT_LOCAL_USER_ID,
        );
    }

    #[tokio::test]
    async fn stale_workflow_revision_rejects_graph_mutation_before_state_changes() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-workflow-revision",
                "worktree-workflow-revision",
            ))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let first_agent = spawn_test_agent(&mut app, &session_id, "first", "dev-stub");
        let second_agent = spawn_test_agent(&mut app, &session_id, "second", "dev-stub");
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

        let create_workflow = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session_id.clone(),
            alias: Some("revision-flow".to_string()),
        });
        let workflow = match router
            .dispatch(
                KernelCommand::from_local_request("create-workflow", None, None, &create_workflow),
                create_workflow,
            )
            .await
            .expect("workflow should be created")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            other => panic!("unexpected workflow response: {other:?}"),
        };
        assert_eq!(workflow.revision(), 0);

        let add_first = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow.id().to_string(),
            agent_id: first_agent.id().to_string(),
            expected_workflow_revision: Some(workflow.revision()),
        });
        let workflow = match router
            .dispatch(
                KernelCommand::from_local_request("add-first", None, None, &add_first),
                add_first,
            )
            .await
            .expect("first mutation should match revision")
        {
            LocalDaemonResponse::WorkflowNodeAdded { workflow, .. } => workflow,
            other => panic!("unexpected add response: {other:?}"),
        };
        assert_eq!(workflow.revision(), 1);

        let stale_add = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow.id().to_string(),
            agent_id: second_agent.id().to_string(),
            expected_workflow_revision: Some(0),
        });
        let rejected = router
            .dispatch(
                KernelCommand::from_local_request("stale-add", None, None, &stale_add),
                stale_add,
            )
            .await
            .expect_err("stale revision should reject before mutation");
        assert!(matches!(
            rejected,
            DaemonError::WorkflowRevisionConflict {
                expected_revision: 0,
                current_revision: 1,
                ..
            }
        ));

        let resolve = LocalDaemonRequest::ResolveWorkflow(ResolveWorkflowRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow.id().to_string(),
        });
        match router
            .dispatch(
                KernelCommand::from_local_request("resolve-after-stale", None, None, &resolve),
                resolve,
            )
            .await
            .expect("workflow should resolve")
        {
            LocalDaemonResponse::WorkflowResolved { workflow } => {
                assert_eq!(workflow.revision(), 1);
                assert_eq!(workflow.nodes().len(), 1);
                assert_eq!(workflow.nodes()[0].agent_id(), first_agent.id());
            }
            other => panic!("unexpected resolve response: {other:?}"),
        }

        let fresh_add = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id,
            workflow_ref: workflow.id().to_string(),
            agent_id: second_agent.id().to_string(),
            expected_workflow_revision: Some(workflow.revision()),
        });
        match router
            .dispatch(
                KernelCommand::from_local_request("fresh-add", None, None, &fresh_add),
                fresh_add,
            )
            .await
            .expect("fresh revision should succeed")
        {
            LocalDaemonResponse::WorkflowNodeAdded { workflow, .. } => {
                assert_eq!(workflow.revision(), 2);
                assert_eq!(workflow.nodes().len(), 2);
            }
            other => panic!("unexpected fresh add response: {other:?}"),
        }
    }

    fn attach_request(session_id: &str, client_id: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.to_string(),
            client_id: client_id.to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        })
    }

    fn focus_request(session_id: &str, agent_id: &str) -> LocalDaemonRequest {
        LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
        })
    }

    fn assert_ownership_denied(error: DaemonError, user_id: &str, owner_user_id: &str) {
        assert!(
            matches!(
                error,
                DaemonError::OwnershipAccessDenied {
                    user_id: ref denied_user,
                    owner_user_id: ref denied_owner,
                    ..
                } if denied_user == user_id && denied_owner == owner_user_id
            ),
            "unexpected error: {error:?}"
        );
    }
}
