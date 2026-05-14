use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::time::Duration;

use crate::app::{DaemonApp, PromptActivityStore};
use crate::error::DaemonError;
use crate::history::OperationalHistoryStore;
use crate::history::SessionHistoryStore;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::ProviderRunOperationLanes;
use crate::runtime::agent_actor::AgentRuntime;
use crate::runtime::agent_control_executor::{
    execute_grant_agent_capability_request, execute_move_agent_to_remote_request,
    execute_revoke_agent_capability_request,
};
use crate::runtime::agent_utility_executor::{
    execute_generate_workspace_commit_message_request, execute_run_agent_utility_request,
};
use crate::runtime::capability_executor::{
    execute_required_capability_request, CapabilityExecutorHealthStore, CapabilityRuntimeStore,
};
use crate::runtime::capability_registry::{
    execute_get_mcp_server_request, execute_get_skill_request, execute_import_mcp_servers_request,
    execute_import_skills_request, execute_install_mcp_server_request,
    execute_install_skill_request, execute_list_mcp_servers_request, execute_list_skills_request,
    execute_uninstall_mcp_server_request, execute_uninstall_skill_request,
    execute_update_mcp_server_request, execute_update_skill_request,
};
use crate::runtime::cloud_api_client::{issue_cloud_runtime_token, post_cloud_json};
use crate::runtime::cloud_relay_control::{
    cloud_kernel_presence_body, cloud_relay_profile_has_runtime_credentials,
    cloud_relay_runtime_token_is_fresh, cloud_relay_token_refresh_due, cloud_runtime_token_subject,
    CLOUD_RELAY_RUNTIME_TOKEN_TTL_MS,
};
use crate::runtime::cloud_relay_executor::{
    clear_cloud_profile_if_stale, execute_accept_cloud_session_invite_request,
    execute_cloud_relay_status_request, execute_connect_cloud_relay_request,
    execute_create_cloud_session_invite_request, execute_issue_cloud_relay_client_token_request,
    execute_list_cloud_collaborators_request, execute_list_cloud_session_members_request,
    execute_logout_cloud_relay_request, execute_pair_cloud_relay_client_request,
    execute_pair_cloud_relay_machine_request, execute_poll_cloud_relay_login_request,
    execute_revoke_cloud_session_invite_request, execute_show_cloud_session_invite_request,
    execute_start_cloud_relay_login_request,
};
use crate::runtime::command::{KernelCommand, KernelCommandPriority, KernelCommandSource};
use crate::runtime::daemon_health_projection::{
    build_daemon_health_projection, DaemonHealthProjectionInput,
};
use crate::runtime::history_executor::{
    execute_prompt_input_history_request, execute_query_history_request,
    execute_record_prompt_input_history_request, execute_semantic_search_history_request,
    execute_session_history_request, projected_session_history_response,
};
use crate::runtime::history_requests::{
    history_query_from_request, history_query_from_search_request,
};
use crate::runtime::kernel_lifecycle_executor::execute_delete_kernel_request;
use crate::runtime::native_interaction_bridge::{
    forward_relay_native_interaction, install_provider_native_interaction_bridge,
};
use crate::runtime::pairing_invite_executor::{
    execute_create_pairing_invite_request, execute_create_terminal_pairing_link_request,
    execute_join_pairing_invite_request, execute_join_terminal_pairing_link_request,
};
use crate::runtime::projection::{
    AgentRuntimeProjectionStore, DaemonConfigProjectionStore, DaemonHealthProjection,
    ProviderCatalogProjectionStore, ProviderProcessProjectionStore, ProviderRunProjectionStore,
    RemoteRelayInventoryProjectionStore, SessionHistoryProjectionStore,
    SessionStateProjectionStore, TransportHealthStore,
};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::runtime::provider_auth_control::{
    execute_get_provider_auth_status_request, execute_start_provider_login_request,
};
use crate::runtime::provider_catalog_control::{
    execute_get_provider_catalog_request, execute_get_provider_command_catalogs_request,
};
use crate::runtime::provider_launch_executor::{
    ProviderLaunchCommandExecutor, ProviderLaunchPendingTracker,
};
use crate::runtime::provider_process_control::{
    execute_list_provider_processes_request, execute_teardown_provider_processes_request,
    provider_processes_visible_to_user_from_projection,
};
use crate::runtime::provider_run_control::{
    ensure_provider_run_visible_to_user, execute_get_provider_run_request,
    execute_logout_provider_and_invalidate_catalog_request,
    execute_update_provider_run_selection_request,
};
use crate::runtime::relay_config_control::{
    execute_configure_relay_request, projected_relay_status_response,
};
use crate::runtime::remote_machine_registry::{
    execute_approve_remote_machine_request, execute_forget_remote_machine_request,
    execute_rename_remote_machine_request,
};
use crate::runtime::remote_relay_inventory::{
    execute_list_remote_machine_kernels_request, execute_list_remote_machines_request,
};
use crate::runtime::response_redaction::redact_response_for_user;
use crate::runtime::session_actor::{FocusedAgentProjection, SessionActor, SessionRuntime};
use crate::runtime::session_collaboration_executor::{
    execute_attach_workspace_link_request, execute_create_session_invite_request,
    execute_create_workspace_link_request, execute_detach_workspace_link_request,
    execute_join_session_invite_request, execute_list_session_members_request,
    execute_list_workspace_links_request, execute_revoke_session_invite_request,
    execute_show_workspace_link_request,
};
use crate::runtime::session_membership::authorize_session_membership;
use crate::runtime::session_projection_refresh::{
    apply_focus_projection_refresh, focus_projection_refresh, response_removed_session_ids,
    response_sessions, session_projection_refresh,
    should_update_agent_runtime_projection_from_response, SessionProjectionRefresh,
};
use crate::runtime::session_read_control::{
    execute_get_session_state_request, execute_list_agents_request, execute_list_sessions_request,
    execute_resolve_session_request, projected_list_sessions_response,
    projected_resolve_session_response, projected_session_inspection_response,
    projected_session_or_absence, projected_session_state_response,
};
use crate::runtime::slice_command_executor::{
    execute_create_slice_request, execute_delete_slice_request,
    execute_get_slice_display_endpoint_request, execute_get_slice_request,
    execute_import_slice_provider_auth_request, execute_list_slices_request,
    execute_start_slice_request, execute_stop_slice_request,
};
use crate::runtime::state::KernelRuntimeState;
use crate::runtime::terminal_output_executor::{
    execute_append_native_provider_output_request, TerminalOutputExecutor,
};
use crate::runtime::terminal_pairings::{
    execute_list_paired_clients_request, execute_list_terminals_request,
    execute_record_paired_client_request, execute_revoke_paired_client_request,
};
use crate::runtime::user_config_executor::{
    execute_delete_credential_secret_request, execute_get_user_config_request,
    execute_get_user_config_schema_request, execute_set_credential_secret_request,
    execute_set_user_config_value_request, execute_unset_user_config_value_request,
};
use crate::runtime::waiting_room_control::{
    execute_waiting_room_inventory_request, execute_waiting_room_public_snapshot_request,
    waiting_room_inventory_version,
};
use crate::runtime::workflow_actor::{is_workflow_command, WorkflowRuntime};
use crate::runtime::workspace_command_executor::{
    execute_commit_and_push_workspace_changes_request, execute_commit_workspace_changes_request,
    execute_create_workspace_directory_request, execute_create_workspace_pull_request_request,
    execute_create_workspace_worktree_request, execute_delete_workspace_worktree_request,
    execute_get_workspace_file_content_request, execute_get_workspace_git_overview_request,
    execute_list_workspace_files_request, execute_list_workspace_worktrees_request,
    execute_push_workspace_branch_request, execute_search_workspace_directories_request,
};
use crate::runtime::workspace_coordinator::WorkspaceCoordinator;
use crate::session::{unix_epoch_ms, PromptIdAllocator, DEFAULT_LOCAL_USER_ID};
use crate::terminal::{TerminalStreamHealthStore, TerminalStreamStore};
use crate::transport::relay_client::RelayClientState;

pub(crate) const INTERACTIVE_COMMAND_QUEUE_LIMIT: usize = 128;

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
    active_turns: crate::app::ActiveTurnStore,
    prompt_activity: PromptActivityStore,
    config_projection: DaemonConfigProjectionStore,
    remote_relay_inventory_projection: RemoteRelayInventoryProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    capability_health: CapabilityExecutorHealthStore,
    capability_runtime: CapabilityRuntimeStore,
    transport_health: TransportHealthStore,
    terminal_health: TerminalStreamHealthStore,
    terminal_output_executor: TerminalOutputExecutor,
    workspace_coordinator: WorkspaceCoordinator,
    provider_launch_pending: ProviderLaunchPendingTracker,
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
            slice_store,
            active_turns,
            prompt_activity,
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
            slice_store.clone(),
            session_projection.clone(),
            provider_run_projection.clone(),
            history_store.clone(),
            operational_history_store.clone(),
            durable_state_store.clone(),
            history_projection.clone(),
            prompt_state_owner.clone(),
            active_turns.clone(),
            prompt_activity.clone(),
            prompt_workspace_claims.clone(),
            structured_output_records.clone(),
            terminal_stream.clone(),
            workspace_coordinator.clone(),
        );
        install_provider_native_interaction_bridge(
            Arc::clone(&app),
            runtime_state.clone(),
            &provider_store,
        );
        let provider_launch_pending = ProviderLaunchPendingTracker::default();
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
            active_turns,
            prompt_activity,
            remote_relay_inventory_projection,
            config_projection,
            relay_state,
            capability_health: CapabilityExecutorHealthStore::default(),
            capability_runtime,
            transport_health: TransportHealthStore::default(),
            terminal_health,
            terminal_output_executor,
            workspace_coordinator,
            provider_launch_pending,
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
            slice_store,
            active_turns,
            prompt_activity,
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
            slice_store.clone(),
            session_projection.clone(),
            provider_run_projection.clone(),
            history_store.clone(),
            operational_history_store.clone(),
            durable_state_store.clone(),
            history_projection.clone(),
            prompt_state_owner.clone(),
            active_turns.clone(),
            prompt_activity.clone(),
            prompt_workspace_claims.clone(),
            structured_output_records.clone(),
            terminal_stream.clone(),
            workspace_coordinator.clone(),
        );
        install_provider_native_interaction_bridge(
            Arc::clone(&app),
            runtime_state.clone(),
            &provider_store,
        );
        let provider_launch_pending = ProviderLaunchPendingTracker::default();
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
            active_turns,
            prompt_activity,
            remote_relay_inventory_projection,
            config_projection,
            relay_state,
            capability_health: CapabilityExecutorHealthStore::default(),
            capability_runtime,
            transport_health,
            terminal_health,
            terminal_output_executor,
            workspace_coordinator,
            provider_launch_pending,
        }
    }

    pub(crate) async fn local_command_caller(
        &self,
        source: KernelCommandSource,
    ) -> crate::runtime::command::KernelCaller {
        let mut caller = crate::runtime::command::KernelCaller::for_source(&source);
        let cloud_profile = self.config_projection.snapshot().cloud_relay;
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
        cloud_relay_token_refresh_due(&config, crate::session::unix_epoch_ms())
    }

    pub(crate) async fn ensure_cloud_relay_connection(&self) -> Result<(), DaemonError> {
        let config = self.config_projection.snapshot();
        let Some(profile) = config.cloud_relay.clone() else {
            return Ok(());
        };
        if !cloud_relay_profile_has_runtime_credentials(&profile) {
            return Ok(());
        }
        let now_ms = crate::session::unix_epoch_ms();
        if cloud_relay_runtime_token_is_fresh(&config, &profile, now_ms) {
            return Ok(());
        }

        let token_subject = cloud_runtime_token_subject(&config, &profile);
        let issued = match issue_cloud_runtime_token(
            &profile,
            &token_subject.subject,
            token_subject.subject_kind,
            None,
            None,
            token_subject.machine_id,
            None,
        )
        .await
        {
            Ok(issued) => issued,
            Err(error) => {
                clear_cloud_profile_if_stale(&self.app, &self.config_projection, &error).await?;
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
        let Some(profile) = config.cloud_relay.as_ref() else {
            return Ok(());
        };
        let Some(body) = cloud_kernel_presence_body(&config, profile, online) else {
            return Ok(());
        };
        let _: serde_json::Value =
            post_cloud_json(profile.api_url.clone(), "/kernels/presence", body).await?;
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
        last_workflow_design_sequence: u64,
    ) -> crate::runtime_transport::WatchResult {
        let mut app = self.app.lock().await;
        crate::runtime_transport::watch_subscription_state(
            &mut app,
            session_id,
            attachment_id,
            tick,
            previous_snapshot,
            last_workflow_design_sequence,
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

    pub(crate) async fn relay_update_leased_agent_config(
        &self,
        leased_agent_id: &str,
        execution_mode: crate::provider::AgentExecutionMode,
        permission_level: crate::provider::AgentPermissionLevel,
    ) -> Result<crate::execution_lease::LeasedAgent, DaemonError> {
        let mut app = self.app.lock().await;
        crate::app::RemoteLeaseRuntime::new(&mut app).update_leased_agent_config(
            leased_agent_id,
            execution_mode,
            permission_level,
        )
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
        forward_relay_native_interaction(&self.runtime_state, context, interaction).await
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

    pub(crate) fn runtime_tool_specs_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
        self.runtime_state
            .runtime_tool_specs_for_auth_token(auth_token)
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
        let caller_user_id =
            authorize_session_membership(&self.app, &self.session_projection, &command, &request)
                .await?;
        if let LocalDaemonRequest::GetSessionState(request) = &request {
            if !self
                .provider_launch_pending
                .has_unsettled_launch(
                    &request.session_id,
                    &self.session_projection,
                    &self.provider_run_projection,
                )
                .await
            {
                if let Some(response) = projected_session_state_response(
                    &self.session_projection,
                    &self.provider_run_projection,
                    &self.prompt_activity,
                    &self.active_turns,
                    request,
                    &caller_user_id,
                ) {
                    return response;
                }
            }
        }
        if let LocalDaemonRequest::ResolveSession(request) = &request {
            if let Some(response) = projected_resolve_session_response(
                &self.session_projection,
                request,
                &caller_user_id,
            ) {
                return response;
            }
        }
        if matches!(request, LocalDaemonRequest::ListSessions(_)) {
            return projected_list_sessions_response(
                &self.app,
                &self.session_projection,
                &caller_user_id,
            )
            .await;
        }
        match &request {
            LocalDaemonRequest::RelayStatus(_) => {
                return projected_relay_status_response(
                    Arc::clone(&self.relay_state),
                    self.config_projection.clone(),
                )
                .await;
            }
            LocalDaemonRequest::ListRemoteMachines(request) => {
                return execute_list_remote_machines_request(
                    Arc::clone(&self.app),
                    Arc::clone(&self.relay_state),
                    self.config_projection.clone(),
                    self.remote_relay_inventory_projection.clone(),
                    request.clone(),
                )
                .await;
            }
            LocalDaemonRequest::ListRemoteMachineKernels(request) => {
                return execute_list_remote_machine_kernels_request(
                    Arc::clone(&self.app),
                    Arc::clone(&self.relay_state),
                    self.config_projection.clone(),
                    self.remote_relay_inventory_projection.clone(),
                    request.clone(),
                )
                .await;
            }
            LocalDaemonRequest::SearchWorkspaceDirectories(request) => {
                return execute_search_workspace_directories_request(request.clone());
            }
            LocalDaemonRequest::CreateWorkspaceDirectory(request) => {
                return execute_create_workspace_directory_request(request.clone());
            }
            LocalDaemonRequest::ListWorkspaceWorktrees(request) => {
                return execute_list_workspace_worktrees_request(request.clone());
            }
            LocalDaemonRequest::CreateWorkspaceWorktree(request) => {
                return execute_create_workspace_worktree_request(request.clone());
            }
            LocalDaemonRequest::DeleteWorkspaceWorktree(request) => {
                return execute_delete_workspace_worktree_request(
                    request.clone(),
                    &self.session_projection,
                    &self.app,
                )
                .await;
            }
            LocalDaemonRequest::CreateWorkspacePullRequest(request) => {
                return execute_create_workspace_pull_request_request(request.clone());
            }
            LocalDaemonRequest::GetWorkspaceGitOverview(request) => {
                return execute_get_workspace_git_overview_request(request.clone());
            }
            LocalDaemonRequest::ListWorkspaceFiles(request) => {
                return execute_list_workspace_files_request(request.clone());
            }
            LocalDaemonRequest::GetWorkspaceFileContent(request) => {
                return execute_get_workspace_file_content_request(request.clone());
            }
            LocalDaemonRequest::RunAgentUtility(request) => {
                return execute_run_agent_utility_request(
                    Arc::clone(&self.app),
                    &self.config_projection,
                    request.clone(),
                )
                .await;
            }
            LocalDaemonRequest::GenerateWorkspaceCommitMessage(request) => {
                return execute_generate_workspace_commit_message_request(
                    Arc::clone(&self.app),
                    &self.config_projection,
                    request.clone(),
                )
                .await;
            }
            LocalDaemonRequest::CommitWorkspaceChanges(request) => {
                return execute_commit_workspace_changes_request(request.clone());
            }
            LocalDaemonRequest::PushWorkspaceBranch(request) => {
                return execute_push_workspace_branch_request(request.clone());
            }
            LocalDaemonRequest::CommitAndPushWorkspaceChanges(request) => {
                return execute_commit_and_push_workspace_changes_request(request.clone());
            }
            LocalDaemonRequest::GetProviderCommandCatalogs(_) => {
                return execute_get_provider_command_catalogs_request();
            }
            LocalDaemonRequest::InstallMcpServer(request) => {
                return execute_install_mcp_server_request(request.clone());
            }
            LocalDaemonRequest::UpdateMcpServer(request) => {
                return execute_update_mcp_server_request(request.clone());
            }
            LocalDaemonRequest::UninstallMcpServer(request) => {
                return execute_uninstall_mcp_server_request(request.clone());
            }
            LocalDaemonRequest::ImportMcpServers(request) => {
                return execute_import_mcp_servers_request(request.clone());
            }
            LocalDaemonRequest::GetMcpServer(request) => {
                return execute_get_mcp_server_request(request.clone());
            }
            LocalDaemonRequest::ListMcpServers(request) => {
                return execute_list_mcp_servers_request(request.clone());
            }
            LocalDaemonRequest::InstallSkill(request) => {
                return execute_install_skill_request(request.clone());
            }
            LocalDaemonRequest::UpdateSkill(request) => {
                return execute_update_skill_request(request.clone());
            }
            LocalDaemonRequest::UninstallSkill(request) => {
                return execute_uninstall_skill_request(request.clone());
            }
            LocalDaemonRequest::ImportSkills(request) => {
                return execute_import_skills_request(request.clone());
            }
            LocalDaemonRequest::GetSkill(request) => {
                return execute_get_skill_request(request.clone());
            }
            LocalDaemonRequest::ListSkills(request) => {
                return execute_list_skills_request(request.clone());
            }
            _ => {}
        }
        if let Some(response) = projected_session_inspection_response(
            &self.session_projection,
            &request,
            &caller_user_id,
        ) {
            return response;
        }
        if let LocalDaemonRequest::PumpTerminalOutput(request) = &request {
            if let Some(response) = self.terminal_output_executor.projected_response(request) {
                return response;
            }
        }
        if let LocalDaemonRequest::GetSessionHistory(request) = &request {
            if let Some(response) = projected_session_history_response(
                self.history_store.clone(),
                self.operational_history_store.clone(),
                self.history_projection.clone(),
                projected_session_or_absence(&self.session_projection, &request.session_id),
                request,
            )
            .await
            {
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
                    processes: provider_processes_visible_to_user_from_projection(
                        processes,
                        &self.provider_run_projection,
                        &caller_user_id,
                    ),
                });
            }
        }
        if matches!(request, LocalDaemonRequest::GetProviderCatalog(_)) {
            return execute_get_provider_catalog_request(
                &self.provider_catalog_projection,
                &self.config_projection,
            )
            .await;
        }
        if matches!(request, LocalDaemonRequest::GetDaemonHealth(_)) {
            return Ok(LocalDaemonResponse::DaemonHealth {
                projection: self.daemon_health_projection(0).await,
            });
        }

        let session_refresh = session_projection_refresh(&request);
        let result = match request {
            LocalDaemonRequest::ConfigureRelay(request) => {
                execute_configure_relay_request(
                    &self.app,
                    Arc::clone(&self.relay_state),
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::CloudRelayStatus(_) => {
                execute_cloud_relay_status_request(&self.config_projection).await
            }
            LocalDaemonRequest::StartCloudRelayLogin(request) => {
                execute_start_cloud_relay_login_request(request).await
            }
            LocalDaemonRequest::PollCloudRelayLogin(request) => {
                execute_poll_cloud_relay_login_request(&self.app, &self.config_projection, request)
                    .await
            }
            LocalDaemonRequest::LogoutCloudRelay(request) => {
                execute_logout_cloud_relay_request(&self.app, &self.config_projection, request)
                    .await
            }
            LocalDaemonRequest::PairCloudRelayClient(request) => {
                execute_pair_cloud_relay_client_request(&self.app, &self.config_projection, request)
                    .await
            }
            LocalDaemonRequest::PairCloudRelayMachine(request) => {
                execute_pair_cloud_relay_machine_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ConnectCloudRelay(request) => {
                execute_connect_cloud_relay_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    self.relay_state.clone(),
                    request,
                )
                .await
            }
            LocalDaemonRequest::IssueCloudRelayClientToken(request) => {
                execute_issue_cloud_relay_client_token_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::CreateCloudSessionInvite(request) => {
                execute_create_cloud_session_invite_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ShowCloudSessionInvite(request) => {
                execute_show_cloud_session_invite_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::AcceptCloudSessionInvite(request) => {
                execute_accept_cloud_session_invite_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::RevokeCloudSessionInvite(request) => {
                execute_revoke_cloud_session_invite_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ListCloudSessionMembers(request) => {
                execute_list_cloud_session_members_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ListCloudCollaborators(request) => {
                execute_list_cloud_collaborators_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::GetUserConfig(request) => {
                execute_get_user_config_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::GetUserConfigSchema(request) => {
                execute_get_user_config_schema_request(request).await
            }
            LocalDaemonRequest::SetUserConfigValue(request) => {
                execute_set_user_config_value_request(
                    &self.app,
                    &self.config_projection,
                    &self.runtime_state,
                    request,
                )
                .await
            }
            LocalDaemonRequest::UnsetUserConfigValue(request) => {
                execute_unset_user_config_value_request(
                    &self.app,
                    &self.config_projection,
                    &self.runtime_state,
                    request,
                )
                .await
            }
            LocalDaemonRequest::SetCredentialSecret(request) => {
                execute_set_credential_secret_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::DeleteCredentialSecret(request) => {
                execute_delete_credential_secret_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::ListSlices(request) => {
                execute_list_slices_request(&self.app, request).await
            }
            LocalDaemonRequest::CreateSlice(request) => {
                execute_create_slice_request(&self.app, request).await
            }
            LocalDaemonRequest::GetSlice(request) => {
                execute_get_slice_request(&self.app, request).await
            }
            LocalDaemonRequest::StartSlice(request) => {
                execute_start_slice_request(&self.app, &self.config_projection, request).await
            }
            LocalDaemonRequest::StopSlice(request) => {
                execute_stop_slice_request(&self.app, request).await
            }
            LocalDaemonRequest::DeleteSlice(request) => {
                execute_delete_slice_request(&self.app, request).await
            }
            LocalDaemonRequest::ImportSliceProviderAuth(request) => {
                execute_import_slice_provider_auth_request(&self.app, request).await
            }
            LocalDaemonRequest::GetSliceDisplayEndpoint(request) => {
                execute_get_slice_display_endpoint_request(&self.app, request).await
            }
            LocalDaemonRequest::DeleteKernel(request) => {
                execute_delete_kernel_request(&self.config_projection, &self.runtime_state, request)
                    .await
            }
            LocalDaemonRequest::ApproveRemoteMachine(request) => {
                execute_approve_remote_machine_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ForgetRemoteMachine(request) => {
                execute_forget_remote_machine_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::RenameRemoteMachine(request) => {
                execute_rename_remote_machine_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ListSessionMembers(request) => {
                execute_list_session_members_request(&self.app, request).await
            }
            LocalDaemonRequest::CreateSessionInvite(request) => {
                execute_create_session_invite_request(
                    &self.app,
                    &self.session_projection,
                    &command,
                    request,
                )
                .await
            }
            LocalDaemonRequest::JoinSessionInvite(request) => {
                execute_join_session_invite_request(&self.app, &self.session_projection, request)
                    .await
            }
            LocalDaemonRequest::RevokeSessionInvite(request) => {
                execute_revoke_session_invite_request(&self.app, &self.session_projection, request)
                    .await
            }
            LocalDaemonRequest::CreateWorkspaceLink(request) => {
                execute_create_workspace_link_request(
                    &self.app,
                    &self.session_projection,
                    &command,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ListWorkspaceLinks(request) => {
                execute_list_workspace_links_request(&self.app, request).await
            }
            LocalDaemonRequest::ShowWorkspaceLink(request) => {
                execute_show_workspace_link_request(&self.app, request).await
            }
            LocalDaemonRequest::AttachWorkspaceLink(request) => {
                let config = self.config_projection.snapshot();
                execute_attach_workspace_link_request(
                    &self.app,
                    &self.session_projection,
                    &command,
                    config.host_machine_id,
                    config.daemon_id,
                    request,
                )
                .await
            }
            LocalDaemonRequest::DetachWorkspaceLink(request) => {
                execute_detach_workspace_link_request(
                    &self.app,
                    &self.session_projection,
                    &command,
                    request,
                )
                .await
            }
            LocalDaemonRequest::CreatePairingInvite(request) => {
                execute_create_pairing_invite_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::JoinPairingInvite(request) => {
                execute_join_pairing_invite_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::CreateTerminalPairingLink(request) => {
                execute_create_terminal_pairing_link_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::JoinTerminalPairingLink(request) => {
                execute_join_terminal_pairing_link_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::ListTerminals(_) => execute_list_terminals_request(),
            LocalDaemonRequest::ListPairedClients(_) => execute_list_paired_clients_request(),
            LocalDaemonRequest::RecordPairedClient(request) => {
                execute_record_paired_client_request(request, unix_epoch_ms)
            }
            LocalDaemonRequest::RevokePairedClient(request) => {
                execute_revoke_paired_client_request(request)
            }
            LocalDaemonRequest::GetSessionHistory(request) => {
                execute_session_history_request(
                    &self.app,
                    self.history_store.clone(),
                    self.operational_history_store.clone(),
                    self.history_projection.clone(),
                    request,
                )
                .await
            }
            LocalDaemonRequest::GetPromptInputHistory(request) => {
                execute_prompt_input_history_request(
                    self.operational_history_store.clone(),
                    request,
                )
                .await
            }
            LocalDaemonRequest::RecordPromptInputHistory(request) => {
                execute_record_prompt_input_history_request(
                    self.operational_history_store.clone(),
                    request,
                )
                .await
            }
            LocalDaemonRequest::QueryHistory(request) => {
                execute_query_history_request(
                    self.operational_history_store.clone(),
                    &self.config_projection,
                    history_query_from_request(request),
                )
                .await
            }
            LocalDaemonRequest::SearchHistory(request) => {
                execute_query_history_request(
                    self.operational_history_store.clone(),
                    &self.config_projection,
                    history_query_from_search_request(request),
                )
                .await
            }
            LocalDaemonRequest::SemanticSearchHistory(request) => {
                execute_semantic_search_history_request(
                    &self.app,
                    &self.runtime_state,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::PumpTerminalOutput(request) => {
                self.terminal_output_executor.execute(request).await
            }
            LocalDaemonRequest::TeardownProviderProcesses(request) => {
                execute_teardown_provider_processes_request(
                    &self.app,
                    &self.session_projection,
                    &self.agent_runtime_projection,
                    request,
                )
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
        apply_focus_projection_refresh(
            &self.app,
            &self.focus_projection,
            &self.session_projection,
            focus_refresh,
            &result,
        )
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
        result.and_then(|response| {
            redact_response_for_user(response, caller_user_id, &self.provider_run_projection)
        })
    }

    pub(crate) async fn waiting_room_inventory_version(&self) -> Result<String, DaemonError> {
        waiting_room_inventory_version(
            &self.app,
            Arc::clone(&self.relay_state),
            self.config_projection.clone(),
        )
        .await
    }

    #[allow(dead_code)]
    pub(crate) async fn daemon_health_projection(
        &self,
        last_event_id: u64,
    ) -> DaemonHealthProjection {
        build_daemon_health_projection(DaemonHealthProjectionInput {
            last_event_id,
            session_runtime: &self.session_runtime,
            agent_runtime: &self.agent_runtime,
            workflow_runtime: &self.workflow_runtime,
            provider_runtime_lanes: &self.provider_runtime_lanes,
            capability_health: &self.capability_health,
            session_projection: &self.session_projection,
            agent_runtime_projection: &self.agent_runtime_projection,
            provider_catalog_projection: &self.provider_catalog_projection,
            transport_health: &self.transport_health,
            terminal_health: &self.terminal_health,
            workspace_coordinator: &self.workspace_coordinator,
            runtime_state: &self.runtime_state,
        })
        .await
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
                let caller_user_id = command_caller_user_id(&command);
                return execute_grant_agent_capability_request(
                    &self.runtime_state,
                    &caller_user_id,
                    request,
                )
                .await;
            }
            LocalDaemonRequest::MoveAgentToRemote(request) => {
                let caller_user_id = command_caller_user_id(&command);
                return execute_move_agent_to_remote_request(
                    &self.runtime_state,
                    &caller_user_id,
                    request,
                )
                .await;
            }
            LocalDaemonRequest::RevokeAgentCapability(request) => {
                let caller_user_id = command_caller_user_id(&command);
                return execute_revoke_agent_capability_request(
                    &self.runtime_state,
                    &caller_user_id,
                    request,
                )
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
                execute_list_sessions_request(&self.app, request).await
            }
            LocalDaemonRequest::ResolveSession(request) => {
                execute_resolve_session_request(&self.app, request).await
            }
            LocalDaemonRequest::GetSessionState(request) => {
                execute_get_session_state_request(&self.app, request).await
            }
            LocalDaemonRequest::GetDaemonHealth(_) => Ok(LocalDaemonResponse::DaemonHealth {
                projection: self.daemon_health_projection(0).await,
            }),
            LocalDaemonRequest::GetProviderRun(request) => {
                execute_get_provider_run_request(&self.app, request).await
            }
            LocalDaemonRequest::UpdateProviderRunSelection(request) => {
                execute_update_provider_run_selection_request(&self.app, request).await
            }
            LocalDaemonRequest::GetPromptInputHistory(request) => {
                execute_prompt_input_history_request(
                    self.operational_history_store.clone(),
                    request,
                )
                .await
            }
            LocalDaemonRequest::RecordPromptInputHistory(request) => {
                execute_record_prompt_input_history_request(
                    self.operational_history_store.clone(),
                    request,
                )
                .await
            }
            LocalDaemonRequest::ListSlices(request) => {
                execute_list_slices_request(&self.app, request).await
            }
            LocalDaemonRequest::CreateSlice(request) => {
                execute_create_slice_request(&self.app, request).await
            }
            LocalDaemonRequest::GetSlice(request) => {
                execute_get_slice_request(&self.app, request).await
            }
            LocalDaemonRequest::StartSlice(request) => {
                execute_start_slice_request(&self.app, &self.config_projection, request).await
            }
            LocalDaemonRequest::StopSlice(request) => {
                execute_stop_slice_request(&self.app, request).await
            }
            LocalDaemonRequest::DeleteSlice(request) => {
                execute_delete_slice_request(&self.app, request).await
            }
            LocalDaemonRequest::ImportSliceProviderAuth(request) => {
                execute_import_slice_provider_auth_request(&self.app, request).await
            }
            LocalDaemonRequest::GetSliceDisplayEndpoint(request) => {
                execute_get_slice_display_endpoint_request(&self.app, request).await
            }
            LocalDaemonRequest::GetProviderCatalog(_) => {
                execute_get_provider_catalog_request(
                    &self.provider_catalog_projection,
                    &self.config_projection,
                )
                .await
            }
            LocalDaemonRequest::GetProviderCommandCatalogs(_) => {
                execute_get_provider_command_catalogs_request()
            }
            LocalDaemonRequest::InstallMcpServer(request) => {
                execute_install_mcp_server_request(request)
            }
            LocalDaemonRequest::UpdateMcpServer(request) => {
                execute_update_mcp_server_request(request)
            }
            LocalDaemonRequest::UninstallMcpServer(request) => {
                execute_uninstall_mcp_server_request(request)
            }
            LocalDaemonRequest::ImportMcpServers(request) => {
                execute_import_mcp_servers_request(request)
            }
            LocalDaemonRequest::GetMcpServer(request) => execute_get_mcp_server_request(request),
            LocalDaemonRequest::ListMcpServers(request) => {
                execute_list_mcp_servers_request(request)
            }
            LocalDaemonRequest::InstallSkill(request) => execute_install_skill_request(request),
            LocalDaemonRequest::UpdateSkill(request) => execute_update_skill_request(request),
            LocalDaemonRequest::UninstallSkill(request) => execute_uninstall_skill_request(request),
            LocalDaemonRequest::ImportSkills(request) => execute_import_skills_request(request),
            LocalDaemonRequest::GetSkill(request) => execute_get_skill_request(request),
            LocalDaemonRequest::ListSkills(request) => execute_list_skills_request(request),
            LocalDaemonRequest::RelayStatus(_) => {
                projected_relay_status_response(
                    Arc::clone(&self.relay_state),
                    self.config_projection.clone(),
                )
                .await
            }
            LocalDaemonRequest::ConfigureRelay(request) => {
                execute_configure_relay_request(
                    &self.app,
                    Arc::clone(&self.relay_state),
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::CloudRelayStatus(_) => {
                execute_cloud_relay_status_request(&self.config_projection).await
            }
            LocalDaemonRequest::StartCloudRelayLogin(request) => {
                execute_start_cloud_relay_login_request(request).await
            }
            LocalDaemonRequest::PollCloudRelayLogin(request) => {
                execute_poll_cloud_relay_login_request(&self.app, &self.config_projection, request)
                    .await
            }
            LocalDaemonRequest::LogoutCloudRelay(request) => {
                execute_logout_cloud_relay_request(&self.app, &self.config_projection, request)
                    .await
            }
            LocalDaemonRequest::PairCloudRelayClient(request) => {
                execute_pair_cloud_relay_client_request(&self.app, &self.config_projection, request)
                    .await
            }
            LocalDaemonRequest::PairCloudRelayMachine(request) => {
                execute_pair_cloud_relay_machine_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ConnectCloudRelay(request) => {
                execute_connect_cloud_relay_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    self.relay_state.clone(),
                    request,
                )
                .await
            }
            LocalDaemonRequest::IssueCloudRelayClientToken(request) => {
                execute_issue_cloud_relay_client_token_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::CreateCloudSessionInvite(request) => {
                execute_create_cloud_session_invite_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ShowCloudSessionInvite(request) => {
                execute_show_cloud_session_invite_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::AcceptCloudSessionInvite(request) => {
                execute_accept_cloud_session_invite_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::RevokeCloudSessionInvite(request) => {
                execute_revoke_cloud_session_invite_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ListCloudSessionMembers(request) => {
                execute_list_cloud_session_members_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ListCloudCollaborators(request) => {
                execute_list_cloud_collaborators_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::GetUserConfig(request) => {
                execute_get_user_config_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::GetUserConfigSchema(request) => {
                execute_get_user_config_schema_request(request).await
            }
            LocalDaemonRequest::SetUserConfigValue(request) => {
                execute_set_user_config_value_request(
                    &self.app,
                    &self.config_projection,
                    &self.runtime_state,
                    request,
                )
                .await
            }
            LocalDaemonRequest::UnsetUserConfigValue(request) => {
                execute_unset_user_config_value_request(
                    &self.app,
                    &self.config_projection,
                    &self.runtime_state,
                    request,
                )
                .await
            }
            LocalDaemonRequest::SetCredentialSecret(request) => {
                execute_set_credential_secret_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::DeleteCredentialSecret(request) => {
                execute_delete_credential_secret_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::DeleteKernel(request) => {
                execute_delete_kernel_request(&self.config_projection, &self.runtime_state, request)
                    .await
            }
            LocalDaemonRequest::ListRemoteMachines(request) => {
                execute_list_remote_machines_request(
                    Arc::clone(&self.app),
                    Arc::clone(&self.relay_state),
                    self.config_projection.clone(),
                    self.remote_relay_inventory_projection.clone(),
                    request,
                )
                .await
            }
            LocalDaemonRequest::ListRemoteMachineKernels(request) => {
                execute_list_remote_machine_kernels_request(
                    Arc::clone(&self.app),
                    Arc::clone(&self.relay_state),
                    self.config_projection.clone(),
                    self.remote_relay_inventory_projection.clone(),
                    request,
                )
                .await
            }
            LocalDaemonRequest::GetWaitingRoomInventory(_) => {
                execute_waiting_room_inventory_request(
                    &self.app,
                    Arc::clone(&self.relay_state),
                    self.config_projection.clone(),
                )
                .await
            }
            LocalDaemonRequest::GetWaitingRoomPublicSnapshot(_) => {
                execute_waiting_room_public_snapshot_request(
                    &self.app,
                    Arc::clone(&self.relay_state),
                    self.config_projection.clone(),
                )
                .await
            }
            LocalDaemonRequest::SearchWorkspaceDirectories(request) => {
                execute_search_workspace_directories_request(request)
            }
            LocalDaemonRequest::CreateWorkspaceDirectory(request) => {
                execute_create_workspace_directory_request(request)
            }
            LocalDaemonRequest::ListWorkspaceWorktrees(request) => {
                execute_list_workspace_worktrees_request(request)
            }
            LocalDaemonRequest::CreateWorkspaceWorktree(request) => {
                execute_create_workspace_worktree_request(request)
            }
            LocalDaemonRequest::DeleteWorkspaceWorktree(request) => {
                execute_delete_workspace_worktree_request(
                    request,
                    &self.session_projection,
                    &self.app,
                )
                .await
            }
            LocalDaemonRequest::CreateWorkspacePullRequest(request) => {
                execute_create_workspace_pull_request_request(request)
            }
            LocalDaemonRequest::GetWorkspaceGitOverview(request) => {
                execute_get_workspace_git_overview_request(request)
            }
            LocalDaemonRequest::ListWorkspaceFiles(request) => {
                execute_list_workspace_files_request(request)
            }
            LocalDaemonRequest::GetWorkspaceFileContent(request) => {
                execute_get_workspace_file_content_request(request)
            }
            LocalDaemonRequest::RunAgentUtility(request) => {
                execute_run_agent_utility_request(
                    Arc::clone(&self.app),
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::GenerateWorkspaceCommitMessage(request) => {
                execute_generate_workspace_commit_message_request(
                    Arc::clone(&self.app),
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::CommitWorkspaceChanges(request) => {
                execute_commit_workspace_changes_request(request)
            }
            LocalDaemonRequest::PushWorkspaceBranch(request) => {
                execute_push_workspace_branch_request(request)
            }
            LocalDaemonRequest::CommitAndPushWorkspaceChanges(request) => {
                execute_commit_and_push_workspace_changes_request(request)
            }
            LocalDaemonRequest::ApproveRemoteMachine(request) => {
                execute_approve_remote_machine_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ForgetRemoteMachine(request) => {
                execute_forget_remote_machine_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::RenameRemoteMachine(request) => {
                execute_rename_remote_machine_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ListSessionMembers(request) => {
                execute_list_session_members_request(&self.app, request).await
            }
            LocalDaemonRequest::CreateSessionInvite(request) => {
                execute_create_session_invite_request(
                    &self.app,
                    &self.session_projection,
                    &command,
                    request,
                )
                .await
            }
            LocalDaemonRequest::JoinSessionInvite(request) => {
                execute_join_session_invite_request(&self.app, &self.session_projection, request)
                    .await
            }
            LocalDaemonRequest::RevokeSessionInvite(request) => {
                execute_revoke_session_invite_request(&self.app, &self.session_projection, request)
                    .await
            }
            LocalDaemonRequest::CreateWorkspaceLink(request) => {
                execute_create_workspace_link_request(
                    &self.app,
                    &self.session_projection,
                    &command,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ListWorkspaceLinks(request) => {
                execute_list_workspace_links_request(&self.app, request).await
            }
            LocalDaemonRequest::ShowWorkspaceLink(request) => {
                execute_show_workspace_link_request(&self.app, request).await
            }
            LocalDaemonRequest::AttachWorkspaceLink(request) => {
                let config = self.config_projection.snapshot();
                execute_attach_workspace_link_request(
                    &self.app,
                    &self.session_projection,
                    &command,
                    config.host_machine_id,
                    config.daemon_id,
                    request,
                )
                .await
            }
            LocalDaemonRequest::DetachWorkspaceLink(request) => {
                execute_detach_workspace_link_request(
                    &self.app,
                    &self.session_projection,
                    &command,
                    request,
                )
                .await
            }
            LocalDaemonRequest::CreatePairingInvite(request) => {
                execute_create_pairing_invite_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::JoinPairingInvite(request) => {
                execute_join_pairing_invite_request(
                    &self.app,
                    &self.config_projection,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::CreateTerminalPairingLink(request) => {
                execute_create_terminal_pairing_link_request(
                    &self.app,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::JoinTerminalPairingLink(request) => {
                execute_join_terminal_pairing_link_request(&self.config_projection, request).await
            }
            LocalDaemonRequest::ListTerminals(_) => execute_list_terminals_request(),
            LocalDaemonRequest::ListPairedClients(_) => execute_list_paired_clients_request(),
            LocalDaemonRequest::RecordPairedClient(request) => {
                execute_record_paired_client_request(request, unix_epoch_ms)
            }
            LocalDaemonRequest::RevokePairedClient(request) => {
                execute_revoke_paired_client_request(request)
            }
            LocalDaemonRequest::GetProviderAuthStatus(request) => {
                execute_get_provider_auth_status_request(request).await
            }
            LocalDaemonRequest::StartProviderLogin(request) => {
                execute_start_provider_login_request(request).await
            }
            LocalDaemonRequest::LogoutProvider(request) => {
                execute_logout_provider_and_invalidate_catalog_request(
                    &self.app,
                    &self.provider_catalog_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::ListProviderProcesses(request) => {
                execute_list_provider_processes_request(&self.app, request).await
            }
            LocalDaemonRequest::TeardownProviderProcesses(request) => {
                execute_teardown_provider_processes_request(
                    &self.app,
                    &self.session_projection,
                    &self.agent_runtime_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::GetSessionHistory(request) => {
                execute_session_history_request(
                    &self.app,
                    self.history_store.clone(),
                    self.operational_history_store.clone(),
                    self.history_projection.clone(),
                    request,
                )
                .await
            }
            LocalDaemonRequest::QueryHistory(request) => {
                execute_query_history_request(
                    self.operational_history_store.clone(),
                    &self.config_projection,
                    history_query_from_request(request),
                )
                .await
            }
            LocalDaemonRequest::SearchHistory(request) => {
                execute_query_history_request(
                    self.operational_history_store.clone(),
                    &self.config_projection,
                    history_query_from_search_request(request),
                )
                .await
            }
            LocalDaemonRequest::SemanticSearchHistory(request) => {
                execute_semantic_search_history_request(
                    &self.app,
                    &self.runtime_state,
                    &self.config_projection,
                    request,
                )
                .await
            }
            LocalDaemonRequest::PumpTerminalOutput(request) => {
                self.terminal_output_executor.execute(request).await
            }
            LocalDaemonRequest::AppendNativeProviderOutput(request) => {
                execute_append_native_provider_output_request(
                    &self.app,
                    &self.session_projection,
                    &self.agent_runtime_projection,
                    request,
                )
                .await
            }
            request @ (LocalDaemonRequest::RunShellCommand(_)
            | LocalDaemonRequest::ReadDirectoryTree(_)
            | LocalDaemonRequest::ReadFile(_)
            | LocalDaemonRequest::EditFile(_)
            | LocalDaemonRequest::InspectGit(_)
            | LocalDaemonRequest::CaptureScreenshot(_)
            | LocalDaemonRequest::StoreTransferredFile(_)) => {
                execute_required_capability_request(
                    &self.capability_runtime,
                    self.capability_health.clone(),
                    request,
                )
                .await
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
                execute_list_agents_request(&self.app, request).await
            }
            request @ (LocalDaemonRequest::CreateWorkflow(_)
            | LocalDaemonRequest::ApplyWorkflowDesignOp(_)
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
            | LocalDaemonRequest::UpdateWorkflowCanvasLayout(_)
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
            | LocalDaemonRequest::SendTerminalInput(_)
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
            self.provider_launch_pending
                .clear_if_settled(
                    &self.app,
                    &session_id,
                    &self.session_projection,
                    &self.provider_run_projection,
                )
                .await;
        }
    }

    async fn apply_provider_launch_projection_state(
        &self,
        result: &Result<LocalDaemonResponse, DaemonError>,
    ) {
        self.provider_launch_pending.track_response(result).await;
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
}

fn command_caller_user_id(command: &KernelCommand) -> String {
    command
        .caller
        .user_id
        .clone()
        .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string())
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
    crate::slice::SliceStore,
    crate::app::ActiveTurnStore,
    crate::app::PromptActivityStore,
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
        app.slices(),
        app.active_turn_store(),
        app.prompt_activity_store(),
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

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
        CreateSessionRequest, PromptStatus, PromptSubmissionOutcome, RuntimeInteraction,
        RuntimeInteractionChoice, RuntimeInteractionChoiceStyle, RuntimeInteractionKind,
        RuntimeInteractionLevel, SessionStatus, DEFAULT_LOCAL_USER_ID,
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

        let results = crate::runtime::workspace_search::search_workspace_directories(
            &root.join("arroba").display().to_string(),
            20,
            None,
        )
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
        let results =
            crate::runtime::workspace_search::search_workspace_directories(&query, 20, None)
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

        let results = crate::runtime::workspace_search::search_workspace_directories(
            &root.join(".arroba").display().to_string(),
            20,
            None,
        )
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
            .provider_launch_pending
            .insert_for_tests("cold-session")
            .await;

        let app_guard = app.lock().await;
        let cleanup_router = router.clone();
        let cleanup_task = tokio::spawn(async move {
            cleanup_router
                .provider_launch_pending
                .clear_if_settled(
                    &cleanup_router.app,
                    "cold-session",
                    &cleanup_router.session_projection,
                    &cleanup_router.provider_run_projection,
                )
                .await;
        });

        timeout(Duration::from_millis(100), cleanup_task)
            .await
            .expect("cold pending launch cleanup should not wait for the app lock")
            .expect("cleanup task should join");
        drop(app_guard);

        assert!(
            router
                .provider_launch_pending
                .contains_for_tests("cold-session")
                .await,
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
        assert!(restored_agent
            .skill_grants()
            .contains(&"review".to_string()));
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
        assert!(app
            .agents
            .get_session_agents(&deleted_session_id)
            .is_empty());
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
        assert!(error
            .to_string()
            .contains("session command lane overloaded"));

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
            LocalDaemonResponse::SessionState {
                session,
                agent_activity,
            } => {
                assert!(session.has_attachment(&attachment_id));
                assert_eq!(session.focused_agent_id(), Some(second_agent.id()));
                assert!(agent_activity.contains_key(second_agent.id()));
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
            kernel_ref: None,
            slice_ref: None,
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
            LocalDaemonResponse::SessionState { session, .. } => {
                assert!(session
                    .agents()
                    .iter()
                    .any(|agent| agent.id() == spawned_agent_id));
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
            LocalDaemonResponse::SessionState { session, .. } => {
                assert!(!session
                    .agents()
                    .iter()
                    .any(|agent| agent.id() == spawned_agent_id));
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
        assert!(projection
            .session_command_lanes
            .iter()
            .any(|lane| lane.lane_id == session_id && lane.queue_limit == 128));
        assert!(projection
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id && lane.queue_limit == 128));
        assert!(projection
            .workflow_command_lanes
            .iter()
            .any(|lane| lane.lane_id == session_id && lane.queue_limit == 128));
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
        assert!(router
            .daemon_health_projection(0)
            .await
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id));
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

        assert!(!router
            .daemon_health_projection(0)
            .await
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id));
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
        assert!(router
            .daemon_health_projection(0)
            .await
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id));

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

        assert!(!router
            .daemon_health_projection(0)
            .await
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id));
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
            kernel_ref: None,
            slice_ref: None,
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
            LocalDaemonResponse::SessionState { session, .. } => {
                assert!(session.active_prompt_for_agent(&agent_id).is_some());
                assert_eq!(session.agents().len(), 1);
            }
            _ => panic!("unexpected state response"),
        }
    }

    #[tokio::test]
    async fn get_session_state_keeps_activity_after_runtime_interaction_projection_refresh() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let provider_run = launch_test_provider(
            &mut app,
            &session_id,
            &agent_id,
            "dev-stub",
            "claude-code",
            "sonnet",
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        router.active_turns.start(crate::app::ActiveTurnState::new(
            session_id.clone(),
            agent_id.clone(),
            "prompt-1".to_string(),
            provider_run.id().to_string(),
        ));
        let interaction = RuntimeInteraction::new(
            "interaction-1",
            &agent_id,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Info,
            Some("Approve file changes?".to_string()),
            "Approve file changes?",
            vec![RuntimeInteractionChoice::new(
                "allow_once",
                "Allow once",
                "allow",
                Some(RuntimeInteractionChoiceStyle::Primary),
            )],
            None,
            None,
            None,
        );
        let _resolution = router
            .runtime_state
            .create_runtime_interaction(&session_id, interaction)
            .await
            .expect("interaction should register");

        let app_guard = app.lock().await;
        let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        });
        let state_command =
            KernelCommand::from_local_request("cmd-state-interaction", None, None, &state_request);
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
            LocalDaemonResponse::SessionState {
                session,
                agent_activity,
            } => {
                assert_eq!(session.focused_agent_id(), Some(agent_id.as_str()));
                assert_eq!(session.agents().len(), 1);
                assert_eq!(session.active_interactions().len(), 1);
                let activity = agent_activity
                    .get(&agent_id)
                    .expect("agent activity should include focused agent");
                assert!(
                    activity.busy,
                    "active turn must keep focused agent working during permission popup"
                );
                assert!(
                    activity.active_turn.is_some(),
                    "active turn projection must survive interaction projection refresh"
                );
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
            LocalDaemonResponse::SessionState { session, .. } => {
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
            LocalDaemonResponse::SessionState { session, .. } => {
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
        assert!(router
            .agent_runtime_projection
            .get(&agent_id)
            .and_then(|projection| projection.active_prompt)
            .is_some());

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
        assert!(router
            .agent_runtime_projection
            .get(&agent_id)
            .and_then(|projection| projection.active_prompt)
            .is_some());

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
            LocalDaemonResponse::SessionState { session, .. } => {
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
    async fn agent_scoped_session_history_warms_full_session_projection() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let first_agent_id = first_agent.id().to_string();
        let second_agent = spawn_test_agent(&mut app, &session_id, "second", "dev-stub");
        let second_agent_id = second_agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-history-projection-agents",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.append_user_prompt_history(
            &session_id,
            attachment.id(),
            &first_agent_id,
            "first agent transcript",
            &[],
        );
        app.append_user_prompt_history(
            &session_id,
            attachment.id(),
            &second_agent_id,
            "second agent transcript",
            &[],
        );

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let first_history_request =
            LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
                session_id: session_id.clone(),
                agent_id: Some(first_agent_id.clone()),
                round_count: Some(10),
                max_chars: None,
                before_entry_index: None,
                before_entry_char_offset: None,
            });
        let first_history_command = KernelCommand::from_local_request(
            "cmd-history-first-agent-warm",
            None,
            None,
            &first_history_request,
        );
        let first_response = router
            .dispatch(first_history_command, first_history_request)
            .await
            .expect("first agent history should resolve");
        match first_response {
            LocalDaemonResponse::SessionHistory { entries, .. } => {
                let texts = entries
                    .into_iter()
                    .map(|entry| entry.entry.text.trim_end().to_string())
                    .collect::<Vec<_>>();
                assert_eq!(texts, vec!["first agent transcript".to_string()]);
            }
            _ => panic!("unexpected history response"),
        }

        let app_guard = app.lock().await;
        let second_history_request =
            LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
                session_id: session_id.clone(),
                agent_id: Some(second_agent_id.clone()),
                round_count: Some(10),
                max_chars: None,
                before_entry_index: None,
                before_entry_char_offset: None,
            });
        let second_history_command = KernelCommand::from_local_request(
            "cmd-history-second-agent-projection",
            None,
            None,
            &second_history_request,
        );
        let history_router = router.clone();
        let second_history_task = tokio::spawn(async move {
            history_router
                .dispatch(second_history_command, second_history_request)
                .await
        });

        tokio::task::yield_now().await;
        assert!(
            second_history_task.is_finished(),
            "agent-scoped warmed GetSessionHistory should use the session projection without app lock access"
        );
        drop(app_guard);

        let second_response = second_history_task
            .await
            .expect("history task should join")
            .expect("second agent history should resolve");
        match second_response {
            LocalDaemonResponse::SessionHistory { entries, .. } => {
                let texts = entries
                    .into_iter()
                    .map(|entry| entry.entry.text.trim_end().to_string())
                    .collect::<Vec<_>>();
                assert_eq!(texts, vec!["second agent transcript".to_string()]);
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
            structured_endpoint: None,
            provider_session_id: None,
            native_tui: false,
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
            structured_endpoint: None,
            provider_session_id: None,
            native_tui: false,
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
            LocalDaemonResponse::SessionState { session, .. } => {
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
            .provider_launch_pending
            .insert_for_tests(session_id.clone())
            .await;

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
            LocalDaemonResponse::SessionState { session, .. } => {
                assert_eq!(session.id(), session_id);
                assert_eq!(session.active_provider_run_id(), Some("projected-run"));
            }
            _ => panic!("unexpected state response"),
        }
        assert!(
            !router
                .provider_launch_pending
                .contains_for_tests(&session_id)
                .await,
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
            structured_endpoint: None,
            provider_session_id: None,
            native_tui: false,
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
            structured_endpoint: None,
            provider_session_id: None,
            native_tui: false,
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
            LocalDaemonResponse::SessionState { session, .. } => {
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
            LocalDaemonResponse::SessionState { session, .. } => {
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
            kernel_ref: None,
            slice_ref: None,
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
            kernel_ref: None,
            slice_ref: None,
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
            kernel_ref: None,
            slice_ref: None,
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
            kernel_ref: None,
            slice_ref: None,
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
            LocalDaemonResponse::SessionState { session, .. } => session,
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
