use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{sleep, Duration};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::history::SessionHistoryStore;
use crate::kernel::agent_actor::{AgentActor, AgentRuntime};
use crate::kernel::capability_executor::{
    execute_capability_request, CapabilityExecutorHealthStore,
};
use crate::kernel::command::{KernelCommand, KernelCommandPriority};
use crate::kernel::projection::{
    page_history_entries, AgentRuntimeProjectionStore, DaemonConfigProjectionStore,
    DaemonHealthProjection, ProviderCatalogProjectionStore, ProviderProcessProjectionStore,
    ProviderRunProjectionStore, SessionHistoryProjectionStore, SessionStateProjectionStore,
    TransportHealthStore,
};
use crate::kernel::prompt_state::PromptStateOwner;
use crate::kernel::session_actor::{FocusedAgentProjection, SessionActor, SessionRuntime};
use crate::kernel::workflow_actor::{is_workflow_command, WorkflowRuntime};
use crate::kernel::workspace_coordinator::WorkspaceCoordinator;
use crate::local::provider_requests::{
    launch_provider_request_from_local, load_provider_catalog, logout_provider_response,
    provider_auth_status_response, provider_command_catalogs_response,
    start_provider_login_response, PROVIDER_CATALOG_CACHE_TTL,
};
use crate::local::{
    GetSessionHistoryRequest, LaunchProviderRunRequest, ListProviderProcessesRequest,
    LocalDaemonRequest, LocalDaemonResponse, PumpTerminalOutputRequest, RelayStatus,
    TeardownProviderProcessesRequest,
};
use crate::provider::{ProviderRunOperationLanes, ProviderRunState};
use crate::terminal::{TerminalStreamHealthStore, TerminalStreamStore};
use crate::transport::relay_client::RelayClientState;

pub(crate) const INTERACTIVE_COMMAND_QUEUE_LIMIT: usize = 128;

#[derive(Debug)]
struct InteractiveCommandEnvelope {
    command: KernelCommand,
    request: LocalDaemonRequest,
    result_tx: oneshot::Sender<Result<LocalDaemonResponse, DaemonError>>,
}

#[derive(Clone)]
pub(crate) struct CommandRouter {
    app: Arc<Mutex<DaemonApp>>,
    interactive_tx: mpsc::Sender<InteractiveCommandEnvelope>,
    agent_runtime: AgentRuntime,
    session_runtime: SessionRuntime,
    workflow_runtime: WorkflowRuntime,
    provider_runtime_lanes: ProviderRunOperationLanes,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    history_store: SessionHistoryStore,
    history_projection: SessionHistoryProjectionStore,
    provider_catalog_projection: ProviderCatalogProjectionStore,
    provider_run_projection: ProviderRunProjectionStore,
    provider_process_projection: ProviderProcessProjectionStore,
    config_projection: DaemonConfigProjectionStore,
    relay_state: Arc<RwLock<RelayClientState>>,
    capability_health: CapabilityExecutorHealthStore,
    transport_health: TransportHealthStore,
    terminal_health: TerminalStreamHealthStore,
    terminal_stream: TerminalStreamStore,
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
        let (interactive_tx, interactive_rx) = mpsc::channel(interactive_capacity);
        let provider_runtime_lanes = ProviderRunOperationLanes::default();
        let focus_projection = FocusedAgentProjection::default();
        let (
            history_store,
            session_projection,
            history_projection,
            provider_catalog_projection,
            provider_run_projection,
            provider_process_projection,
            agent_runtime_projection,
            config_projection,
            relay_state,
            terminal_health,
            terminal_stream,
            workspace_coordinator,
            prompt_state_owner,
        ) = router_projection_stores(&app);
        let pending_provider_launch_sessions = Arc::new(Mutex::new(HashSet::new()));
        let agent_runtime = AgentRuntime::new(
            Arc::clone(&app),
            provider_runtime_lanes.clone(),
            focus_projection.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            prompt_state_owner.clone(),
        );
        let session_runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            Arc::clone(&app),
            session_capacity,
            focus_projection.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            terminal_stream.clone(),
        );
        let workflow_runtime = WorkflowRuntime::new(
            Arc::clone(&app),
            session_projection.clone(),
            agent_runtime_projection.clone(),
        );
        tokio::spawn(run_interactive_command_lane(
            Arc::clone(&app),
            interactive_rx,
        ));
        Self {
            app,
            interactive_tx,
            agent_runtime,
            session_runtime,
            workflow_runtime,
            provider_runtime_lanes,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            history_store,
            history_projection,
            provider_catalog_projection,
            provider_run_projection,
            provider_process_projection,
            config_projection,
            relay_state,
            capability_health: CapabilityExecutorHealthStore::default(),
            transport_health: TransportHealthStore::default(),
            terminal_health,
            terminal_stream,
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
        let (interactive_tx, interactive_rx) = mpsc::channel(interactive_capacity);
        let focus_projection = FocusedAgentProjection::default();
        let (
            history_store,
            session_projection,
            history_projection,
            provider_catalog_projection,
            provider_run_projection,
            provider_process_projection,
            agent_runtime_projection,
            config_projection,
            relay_state,
            terminal_health,
            terminal_stream,
            workspace_coordinator,
            prompt_state_owner,
        ) = router_projection_stores(&app);
        let pending_provider_launch_sessions = Arc::new(Mutex::new(HashSet::new()));
        let agent_runtime = AgentRuntime::new(
            Arc::clone(&app),
            provider_runtime_lanes.clone(),
            focus_projection.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            prompt_state_owner.clone(),
        );
        let session_runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            Arc::clone(&app),
            crate::kernel::session_actor::SESSION_COMMAND_QUEUE_LIMIT,
            focus_projection.clone(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            terminal_stream.clone(),
        );
        let workflow_runtime = WorkflowRuntime::new(
            Arc::clone(&app),
            session_projection.clone(),
            agent_runtime_projection.clone(),
        );
        tokio::spawn(run_interactive_command_lane(
            Arc::clone(&app),
            interactive_rx,
        ));
        Self {
            app,
            interactive_tx,
            agent_runtime,
            session_runtime,
            workflow_runtime,
            provider_runtime_lanes,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            history_store,
            history_projection,
            provider_catalog_projection,
            provider_run_projection,
            provider_process_projection,
            config_projection,
            relay_state,
            capability_health: CapabilityExecutorHealthStore::default(),
            transport_health,
            terminal_health,
            terminal_stream,
            workspace_coordinator,
            pending_provider_launch_sessions,
        }
    }

    pub(crate) fn runtime_mcp_bind_address(&self) -> (String, u16) {
        let config = self.config_projection.snapshot();
        (config.runtime_mcp_host, config.runtime_mcp_port)
    }

    pub(crate) async fn dispatch_authenticated_runtime_tool_call(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let mut app = self.app.lock().await;
        crate::transport::runtime_tools::dispatch_authenticated_runtime_tool_call(
            &mut app, auth_token, tool_name, arguments,
        )
    }

    pub(crate) async fn dispatch(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let focus_refresh = focus_projection_refresh(&request);
        if let LocalDaemonRequest::GetSessionState(request) = &request {
            if !self
                .has_unsettled_pending_provider_launch(&request.session_id)
                .await
            {
                if let Some(session) = self.session_projection.get(&request.session_id) {
                    return Ok(LocalDaemonResponse::SessionState { session });
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
                return Ok(LocalDaemonResponse::SessionResolved { session });
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
                return Ok(LocalDaemonResponse::SessionResolved { session });
            }
        }
        if matches!(request, LocalDaemonRequest::ListSessions(_)) {
            if let Some(sessions) = self.session_projection.list() {
                return Ok(LocalDaemonResponse::SessionsListed { sessions });
            }
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
            LocalDaemonRequest::GetProviderCommandCatalogs(_) => {
                return provider_command_catalogs_response();
            }
            _ => {}
        }
        if let Some(response) = self.projected_session_inspection_response(&request) {
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
                return Ok(LocalDaemonResponse::ProviderProcessesListed { processes });
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
            LocalDaemonRequest::GetSessionHistory(request) => {
                self.execute_session_history_request(request).await
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
                    execute_local_request_with_async_boundaries(
                        &self.app,
                        self.capability_health.clone(),
                        request,
                    )
                    .await
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
        result
    }

    fn projected_session_inspection_response(
        &self,
        request: &LocalDaemonRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        match request {
            LocalDaemonRequest::ListAgents(request) => {
                let session = match self.projected_session_or_absence(&request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
                Some(Ok(LocalDaemonResponse::AgentsListed {
                    agents: session.agents().to_vec(),
                }))
            }
            LocalDaemonRequest::ListWorkflows(request) => {
                let session = match self.projected_session_or_absence(&request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
                Some(Ok(LocalDaemonResponse::WorkflowsListed {
                    workflows: session.workflows().to_vec(),
                }))
            }
            LocalDaemonRequest::ResolveWorkflow(request) => {
                let session = match self.projected_session_or_absence(&request.session_id)? {
                    Ok(session) => session,
                    Err(error) => return Some(Err(error)),
                };
                Some(
                    projected_resolve_workflow(&session, &request.workflow_ref)
                        .map(|workflow| LocalDaemonResponse::WorkflowResolved { workflow }),
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
                    projected_resolve_workflow_run(&session, &request.workflow_run_ref)
                        .map(|workflow_run| LocalDaemonResponse::WorkflowRun { workflow_run }),
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
        let config = self.config_projection.snapshot();
        let connected = self.relay_state.read().await.connected();
        Ok(LocalDaemonResponse::RelayStatus {
            status: RelayStatus {
                configured: config.relay_url.is_some() && config.relay_token.is_some(),
                connected,
                relay_url: config.relay_url,
                relay_token_configured: config.relay_token.is_some(),
                daemon_id: config.daemon_id,
                machine_id: config.host_machine_id,
                machine_alias: config.host_machine_alias,
            },
        })
    }

    async fn projected_remote_machines_response(&self) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config_projection.snapshot();
        let machines = crate::transport::relay_discovery::list_live_machines(&config).await?;
        let machines = crate::local::provider_requests::remote_machine_records(
            machines,
            &config.host_machine_id,
        );
        Ok(LocalDaemonResponse::RemoteMachinesListed { machines })
    }

    async fn projected_remote_machine_kernels_response(
        &self,
        machine_ref: String,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let config = self.config_projection.snapshot();
        let machine_ref =
            crate::local::provider_requests::resolve_registered_or_raw_machine_ref(&machine_ref);
        let kernels =
            crate::transport::relay_discovery::list_live_kernels_for_machine(&config, &machine_ref)
                .await?;
        Ok(LocalDaemonResponse::RemoteMachineKernelsListed {
            machine_ref,
            kernels,
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

    async fn execute_terminal_output_request(
        &self,
        request: PumpTerminalOutputRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let Some(session) = self.session_projection.get(&request.session_id) else {
            let mut app = self.app.lock().await;
            let records = app.pump_terminal_output(&request.session_id, &request.attachment_id)?;
            if let Ok(session) = app.local_api_session_snapshot(&request.session_id) {
                self.agent_runtime_projection.update_session(&session);
                self.session_projection.update(session);
            }
            return Ok(LocalDaemonResponse::TerminalOutput { records });
        };
        let Some(provider_run_id) = session.active_provider_run_id().map(str::to_string) else {
            return Ok(LocalDaemonResponse::TerminalOutput {
                records: self
                    .terminal_stream
                    .drain_output_records(&request.session_id, &request.attachment_id),
            });
        };
        if self
            .provider_run_projection
            .get(&provider_run_id)
            .is_some_and(|run| {
                run.session_id() == request.session_id
                    && matches!(
                        run.state(),
                        ProviderRunState::Ended | ProviderRunState::Parked
                    )
            })
        {
            return Ok(LocalDaemonResponse::TerminalOutput {
                records: self
                    .terminal_stream
                    .drain_output_records(&request.session_id, &request.attachment_id),
            });
        }

        let recipient_attachment_ids = session.attachment_ids().iter().cloned().collect();
        let _permit = self.provider_runtime_lanes.acquire(&provider_run_id).await;
        {
            let mut app = self.app.lock().await;
            let _ = app.pump_provider_output(
                &request.session_id,
                &provider_run_id,
                recipient_attachment_ids,
            )?;
            if let Ok(session) = app.local_api_session_snapshot(&request.session_id) {
                self.agent_runtime_projection.update_session(&session);
                self.session_projection.update(session);
            }
        }
        Ok(LocalDaemonResponse::TerminalOutput {
            records: self
                .terminal_stream
                .drain_output_records(&request.session_id, &request.attachment_id),
        })
    }

    async fn execute_teardown_provider_processes_request(
        &self,
        request: TeardownProviderProcessesRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (processes, sessions) = {
            let mut app = self.app.lock().await;
            let processes = app.teardown_provider_processes(request.provider.as_deref())?;
            let session_ids = processes
                .iter()
                .flat_map(|process| process.owner_session_ids.iter())
                .cloned()
                .collect::<HashSet<_>>();
            let sessions = session_ids
                .into_iter()
                .filter_map(|session_id| app.local_api_session_snapshot(&session_id).ok())
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
        let history_projection = self.history_projection.clone();
        tokio::task::spawn_blocking(move || {
            let entries = history.load(&session)?;
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
                crate::kernel_transport::RECENT_EVENT_LIMIT,
                crate::kernel_transport::COMMAND_RESULT_CACHE_LIMIT,
                crate::kernel_transport::INBOUND_REQUEST_LIMIT,
            ),
            self.terminal_health.snapshot(),
            self.session_projection
                .workspace_coordination_snapshot(self.workspace_coordinator.active_claims()),
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
            request => {
                let (result_tx, result_rx) = oneshot::channel();
                self.interactive_tx
                    .try_send(InteractiveCommandEnvelope {
                        command,
                        request,
                        result_tx,
                    })
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "enqueue interactive kernel command",
                        message: format!("interactive command lane overloaded: {error}"),
                    })?;
                return result_rx
                    .await
                    .map_err(|error| DaemonError::LocalTransport {
                        operation: "await interactive kernel command",
                        message: error.to_string(),
                    })?;
            }
        }
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
    SessionStateProjectionStore,
    SessionHistoryProjectionStore,
    ProviderCatalogProjectionStore,
    ProviderRunProjectionStore,
    ProviderProcessProjectionStore,
    AgentRuntimeProjectionStore,
    DaemonConfigProjectionStore,
    Arc<RwLock<RelayClientState>>,
    TerminalStreamHealthStore,
    TerminalStreamStore,
    WorkspaceCoordinator,
    PromptStateOwner,
) {
    let app = app
        .try_lock()
        .expect("CommandRouter should be created before holding the app lock");
    (
        app.history_store(),
        app.session_state_projection_store(),
        app.session_history_projection_store(),
        app.provider_catalog_projection_store(),
        app.provider_run_projection_store(),
        app.provider_process_projection_store(),
        app.agent_runtime_projection_store(),
        app.config_projection_store(),
        app.relay_client_state(),
        app.terminal_health_store(),
        app.terminal_stream_store(),
        app.workspace_coordinator(),
        app.prompt_state_owner(),
    )
}

#[derive(Debug)]
enum FocusProjectionRefresh {
    None,
    AgentSpawn,
    SnapshotSession { session_id: String },
}

fn focus_projection_refresh(request: &LocalDaemonRequest) -> FocusProjectionRefresh {
    match request {
        LocalDaemonRequest::SpawnAgent(_) => FocusProjectionRefresh::AgentSpawn,
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

fn session_projection_refresh(request: &LocalDaemonRequest) -> SessionProjectionRefresh {
    match request {
        LocalDaemonRequest::AttachToSession(_)
        | LocalDaemonRequest::DetachFromSession(_)
        | LocalDaemonRequest::FocusAgent(_)
        | LocalDaemonRequest::CycleAgentFocus(_) => SessionProjectionRefresh::None,
        LocalDaemonRequest::SpawnAgent(_) | LocalDaemonRequest::DestroyAgent(_) => {
            SessionProjectionRefresh::SnapshotAgentResponse
        }
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
        | LocalDaemonResponse::SessionConfigUpdated { session, .. }
        | LocalDaemonResponse::SessionEnded { session }
        | LocalDaemonResponse::SessionAliased { session }
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
        _ => Vec::new(),
    }
}

async fn run_interactive_command_lane(
    app: Arc<Mutex<DaemonApp>>,
    mut rx: mpsc::Receiver<InteractiveCommandEnvelope>,
) {
    while let Some(envelope) = rx.recv().await {
        crate::logging::info_with_fields(
            "daemon.kernel_router",
            "interactive kernel command dispatched",
            serde_json::json!({
                "command_id": envelope.command.command_id,
                "command_type": envelope.command.command_type,
                "correlation_id": envelope.command.correlation_id,
                "session_id": envelope.command.session_id,
                "attachment_id": envelope.command.attachment_id,
                "agent_id": envelope.command.agent_id,
            }),
        );
        let result = execute_interactive_request(&app, &envelope.command, envelope.request).await;
        let _ = envelope.result_tx.send(result);
    }
}

async fn execute_interactive_request(
    app: &Arc<Mutex<DaemonApp>>,
    command: &KernelCommand,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    if SessionActor::is_session_interactive_command(&request) {
        let mut app = app.lock().await;
        return SessionActor::handle_interactive_command(&mut app, request).unwrap_or_else(|| {
            Err(DaemonError::LocalTransport {
                operation: "execute interactive kernel command",
                message: "request is not handled by the session runtime".to_string(),
            })
        });
    }
    if AgentActor::is_agent_interactive_command(&request) {
        let mut app = app.lock().await;
        return app.handle_agent_request(request);
    }
    Err(DaemonError::LocalTransport {
        operation: "execute interactive kernel command",
        message: format!(
            "unsupported interactive command `{}` reached the legacy interactive lane",
            command.command_type
        ),
    })
}

pub(crate) async fn execute_local_request_with_async_boundaries(
    app: &Arc<Mutex<DaemonApp>>,
    capability_health: CapabilityExecutorHealthStore,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::RelayStatus(_) => {
            let (config, relay_state) = {
                let app = app.lock().await;
                (app.config().clone(), app.relay_client_state())
            };
            let connected = relay_state.read().await.connected();
            Ok(LocalDaemonResponse::RelayStatus {
                status: RelayStatus {
                    configured: config.relay_url.is_some() && config.relay_token.is_some(),
                    connected,
                    relay_url: config.relay_url,
                    relay_token_configured: config.relay_token.is_some(),
                    daemon_id: config.daemon_id,
                    machine_id: config.host_machine_id,
                    machine_alias: config.host_machine_alias,
                },
            })
        }
        LocalDaemonRequest::ListRemoteMachines(_) => {
            let config = {
                let app = app.lock().await;
                app.config().clone()
            };
            let machines = crate::transport::relay_discovery::list_live_machines(&config).await?;
            let machines = crate::local::provider_requests::remote_machine_records(
                machines,
                &config.host_machine_id,
            );
            Ok(LocalDaemonResponse::RemoteMachinesListed { machines })
        }
        LocalDaemonRequest::ListRemoteMachineKernels(request) => {
            let config = {
                let app = app.lock().await;
                app.config().clone()
            };
            let machine_ref =
                crate::local::provider_requests::resolve_registered_or_raw_machine_ref(
                    &request.machine_ref,
                );
            let kernels = crate::transport::relay_discovery::list_live_kernels_for_machine(
                &config,
                &machine_ref,
            )
            .await?;
            Ok(LocalDaemonResponse::RemoteMachineKernelsListed {
                machine_ref,
                kernels,
            })
        }
        LocalDaemonRequest::GetSessionHistory(request) => {
            execute_session_history_request(app, request).await
        }
        LocalDaemonRequest::LaunchProviderRun(request) => {
            execute_launch_provider_run_request(app, request).await
        }
        LocalDaemonRequest::GetProviderCatalog(_) => execute_provider_catalog_request(app).await,
        LocalDaemonRequest::GetProviderCommandCatalogs(_) => provider_command_catalogs_response(),
        LocalDaemonRequest::GetProviderAuthStatus(request) => {
            tokio::task::spawn_blocking(move || provider_auth_status_response(request))
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "get provider auth status",
                    message: error.to_string(),
                })?
        }
        LocalDaemonRequest::StartProviderLogin(request) => {
            tokio::task::spawn_blocking(move || start_provider_login_response(request))
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "start provider login",
                    message: error.to_string(),
                })?
        }
        LocalDaemonRequest::LogoutProvider(request) => {
            let response = tokio::task::spawn_blocking(move || logout_provider_response(request))
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "logout provider",
                    message: error.to_string(),
                })??;
            let mut app = app.lock().await;
            app.invalidate_provider_catalog_cache();
            Ok(response)
        }
        LocalDaemonRequest::ListProviderProcesses(request) => {
            execute_list_provider_processes_request(app, request).await
        }
        LocalDaemonRequest::TeardownProviderProcesses(request) => {
            execute_teardown_provider_processes_request(app, request).await
        }
        request if is_capability_request(&request) => {
            execute_capability_request(app, capability_health, request)
                .await
                .unwrap_or_else(|| {
                    Err(DaemonError::LocalTransport {
                        operation: "route capability request",
                        message: "capability request was not handled by executor".to_string(),
                    })
                })
        }
        request => {
            if is_blocking_local_request(&request) {
                let app = Arc::clone(app);
                let handle = tokio::runtime::Handle::current();
                return tokio::task::spawn_blocking(move || {
                    handle.block_on(async move {
                        let mut app = app.lock().await;
                        app.handle_local_request(request)
                    })
                })
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "run blocking kernel request",
                    message: error.to_string(),
                })?;
            }
            let mut app = app.lock().await;
            app.handle_local_request(request)
        }
    }
}

fn is_capability_request(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        LocalDaemonRequest::RunShellCommand(_)
            | LocalDaemonRequest::ReadDirectoryTree(_)
            | LocalDaemonRequest::ReadFile(_)
            | LocalDaemonRequest::EditFile(_)
            | LocalDaemonRequest::InspectGit(_)
            | LocalDaemonRequest::CaptureScreenshot(_)
            | LocalDaemonRequest::StoreTransferredFile(_)
    )
}

async fn execute_provider_catalog_request(
    app: &Arc<Mutex<DaemonApp>>,
) -> Result<LocalDaemonResponse, DaemonError> {
    let config = {
        let app = app.lock().await;
        if let Some(catalog) = app.cached_provider_catalog() {
            return Ok(LocalDaemonResponse::ProviderCatalog { catalog });
        }
        app.config().clone()
    };

    let catalog = tokio::task::spawn_blocking(move || load_provider_catalog(config))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "load provider catalog",
            message: error.to_string(),
        })??;
    let mut app = app.lock().await;
    app.cache_provider_catalog(catalog.clone());
    Ok(LocalDaemonResponse::ProviderCatalog { catalog })
}

async fn execute_launch_provider_run_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: LaunchProviderRunRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (started, runtime_init_delay_ms) = {
        let mut app = app.lock().await;
        let launch_request = launch_provider_request_from_local(&app, request);
        (
            app.start_provider_launch(launch_request)?,
            app.config().provider_runtime_init_delay_ms,
        )
    };
    let accepted = started.run.clone();
    let app = Arc::clone(app);
    tokio::spawn(async move {
        if runtime_init_delay_ms > 0 {
            sleep(Duration::from_millis(runtime_init_delay_ms)).await;
        }
        let run = started.run.clone();
        let binding = tokio::task::spawn_blocking(move || {
            DaemonApp::initialize_provider_runtime_binding(&run)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "initialize provider runtime",
            message: error.to_string(),
        });

        match binding {
            Ok(Ok(binding)) => {
                let mut app = app.lock().await;
                if let Err(error) = app.finish_provider_launch(&started, binding) {
                    app.fail_provider_launch(&started, &error);
                }
            }
            Ok(Err(error)) | Err(error) => {
                let mut app = app.lock().await;
                app.fail_provider_launch(&started, &error);
            }
        }
    });
    Ok(LocalDaemonResponse::ProviderRunLaunchAccepted {
        provider_run: accepted,
    })
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

async fn execute_teardown_provider_processes_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: TeardownProviderProcessesRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut app = app.lock().await;
    let processes = app.teardown_provider_processes(request.provider.as_deref())?;
    Ok(LocalDaemonResponse::ProviderProcessesTornDown { processes })
}

async fn execute_session_history_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: GetSessionHistoryRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let (history, session) = {
        let app = app.lock().await;
        let session = app.sessions().get_session(&request.session_id)?;
        (app.history_store(), session)
    };

    tokio::task::spawn_blocking(move || {
        let entries = history.load(&session)?;
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

fn is_blocking_local_request(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        LocalDaemonRequest::GetProviderCatalog(_)
            | LocalDaemonRequest::GetProviderCommandCatalogs(_)
            | LocalDaemonRequest::GetProviderAuthStatus(_)
            | LocalDaemonRequest::StartProviderLogin(_)
            | LocalDaemonRequest::LogoutProvider(_)
            | LocalDaemonRequest::GetSessionHistory(_)
            | LocalDaemonRequest::RunShellCommand(_)
            | LocalDaemonRequest::ReadDirectoryTree(_)
            | LocalDaemonRequest::ReadFile(_)
            | LocalDaemonRequest::EditFile(_)
            | LocalDaemonRequest::InspectGit(_)
            | LocalDaemonRequest::CaptureScreenshot(_)
            | LocalDaemonRequest::StoreTransferredFile(_)
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    use crate::attachment::ClientCapabilityLevel;
    use crate::kernel::command::KernelCommand;
    use crate::kernel::router::CommandRouter;
    use crate::local::{
        AliasSessionRequest, AttachToSessionRequest, CancelActivePromptRequest,
        CompletePromptRequest, ConfigureRelayRequest, CreateWorkflowRequest,
        CycleAgentFocusRequest, DeleteSessionRequest, DestroyAgentRequest,
        DetachFromSessionRequest, EndSessionRequest, FocusAgentRequest, GetDaemonHealthRequest,
        GetProviderCatalogRequest, GetProviderCommandCatalogsRequest, GetProviderRunRequest,
        GetSessionHistoryRequest, GetSessionStateRequest, LaunchProviderRunRequest,
        ListAgentsRequest, ListProviderProcessesRequest, ListSessionsRequest,
        ListWorkflowRunsRequest, ListWorkflowWatchdogsRequest, ListWorkflowsRequest,
        LocalDaemonRequest, LocalDaemonResponse, PollRuntimeNoticesRequest,
        PumpTerminalOutputRequest, RelayStatusRequest, ResizeTerminalRequest,
        ResolveSessionRequest, ResolveWorkflowRequest, RunShellCapabilityRequest,
        SpawnAgentRequest, SubmitPromptRequest, TeardownProviderProcessesRequest,
        UpdateSessionConfigRequest,
    };
    use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, RuntimeProviderRun};
    use crate::session::{CreateSessionRequest, PromptStatus, PromptSubmissionOutcome};
    use crate::{DaemonApp, DaemonConfig, DaemonError};

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
        let (session, _agent) = app
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
    async fn rejects_session_commands_when_bounded_lane_is_full() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
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

    #[tokio::test]
    async fn focus_uses_session_lane_when_interactive_lane_is_full() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;

        let blocked_request = focus_request(&session_id, &agent_id);
        let blocked_command =
            KernelCommand::from_local_request("cmd-blocked-generic", None, None, &blocked_request);
        let (blocked_result_tx, blocked_result_rx) = tokio::sync::oneshot::channel();
        router
            .interactive_tx
            .try_send(super::InteractiveCommandEnvelope {
                command: blocked_command,
                request: blocked_request,
                result_tx: blocked_result_tx,
            })
            .expect("generic interactive lane should fill");

        let focus_request = focus_request(&session_id, &agent_id);
        let focus_command =
            KernelCommand::from_local_request("cmd-focus", None, None, &focus_request);
        let focus_router = router.clone();
        let focus_task =
            tokio::spawn(async move { focus_router.dispatch(focus_command, focus_request).await });

        tokio::task::yield_now().await;
        assert!(
            !focus_task.is_finished(),
            "focus should be admitted to the session lane instead of failing on the full generic lane"
        );

        drop(app_guard);
        let _ = blocked_result_rx
            .await
            .expect("blocked generic command should resolve");
        let focus_response = focus_task
            .await
            .expect("focus task should join")
            .expect("focus should succeed");
        assert!(matches!(
            focus_response,
            crate::local::LocalDaemonResponse::AgentFocused { .. }
        ));
    }

    #[tokio::test]
    async fn create_session_uses_session_runtime_when_interactive_lane_is_full() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;

        let blocked_request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let blocked_command = KernelCommand::from_local_request(
            "cmd-blocked-generic-create",
            None,
            None,
            &blocked_request,
        );
        let (blocked_result_tx, blocked_result_rx) = tokio::sync::oneshot::channel();
        router
            .interactive_tx
            .try_send(super::InteractiveCommandEnvelope {
                command: blocked_command,
                request: blocked_request,
                result_tx: blocked_result_tx,
            })
            .expect("generic interactive lane should fill");

        let create_request = LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
            "workspace-create-runtime",
            "worktree-create-runtime",
        ));
        let create_command =
            KernelCommand::from_local_request("cmd-create-runtime", None, None, &create_request);
        let create_router = router.clone();
        let create_task =
            tokio::spawn(
                async move { create_router.dispatch(create_command, create_request).await },
            );

        tokio::task::yield_now().await;
        assert!(
            !create_task.is_finished(),
            "create should be admitted to the session runtime instead of failing on the full generic lane"
        );
        assert!(
            router
                .session_runtime
                .has_lane(crate::kernel::session_actor::SESSION_CREATE_LANE_ID)
                .await
        );

        drop(app_guard);
        let _ = blocked_result_rx
            .await
            .expect("blocked generic command should resolve");
        let create_response = create_task
            .await
            .expect("create task should join")
            .expect("create should succeed");
        let session_id = match create_response {
            LocalDaemonResponse::SessionCreated { session, .. } => session.id().to_string(),
            _ => panic!("unexpected create response"),
        };
        assert!(
            router.session_projection.get(&session_id).is_some(),
            "session runtime should publish the created session projection"
        );
    }

    #[tokio::test]
    async fn legacy_interactive_lane_rejects_unsupported_requests_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;

        let request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let command =
            KernelCommand::from_local_request("cmd-unsupported-interactive", None, None, &request);
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        router
            .interactive_tx
            .try_send(super::InteractiveCommandEnvelope {
                command,
                request,
                result_tx,
            })
            .expect("unsupported command should enter the legacy lane for rejection");

        let result = timeout(Duration::from_millis(100), result_rx)
            .await
            .expect("unsupported command should not wait for the app lock")
            .expect("unsupported command result should resolve");
        drop(app_guard);

        let error = result.expect_err("unsupported command should be rejected");
        assert!(error
            .to_string()
            .contains("unsupported interactive command `daemon.health.get`"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_submit_does_not_wait_behind_slow_history_load() {
        let mut config = DaemonConfig::for_tests();
        config.session_history_read_delay_ms = 120;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-slow-history",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.append_user_prompt_history(
            &session_id,
            attachment.id(),
            &agent_id,
            "slow history entry",
            &[],
        );
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-slow-catalog",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

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
        let (session, first_agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let second_agent = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session_id.clone(),
                alias: Some("reviewer".to_string()),
                provider: "claude-code".to_string(),
                model: None,
                effort: None,
                worktree_id: None,
                machine_ref: None,
            }))
            .expect("spawn should succeed")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected spawn response"),
        };
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
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let spawn_request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("projected-agent".to_string()),
            provider: "claude-code".to_string(),
            model: None,
            effort: None,
            worktree_id: None,
            machine_ref: None,
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
            LocalDaemonResponse::SessionState { session } => {
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
        let (session, _agent) = app
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
    async fn delete_session_resolves_lane_from_warmed_projection_without_app_lock() {
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

        let mut lane_created = false;
        for _ in 0..50 {
            if router.session_runtime.has_lane(&session_id).await {
                lane_created = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            lane_created,
            "delete should resolve the session lane from the warmed projection before touching the app lock"
        );
        assert!(
            !delete_task.is_finished(),
            "session worker should still wait on the deliberately held app lock"
        );

        drop(app_guard);
        let delete_response = delete_task
            .await
            .expect("delete task should join")
            .expect("delete should succeed");
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
        let (session, _agent) = app
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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

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
    async fn agent_and_workflow_lanes_are_removed_when_session_ends() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-agent-lane-cleanup",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-agent-destroy-lane-cleanup",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

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
    async fn prompt_submit_uses_agent_lane_when_interactive_lane_is_full() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
        let app_guard = app.lock().await;

        let first_request = focus_request(&session_id, &agent_id);
        let first_command =
            KernelCommand::from_local_request("cmd-focus-1", None, None, &first_request);
        let first_router = router.clone();
        let first_task =
            tokio::spawn(async move { first_router.dispatch(first_command, first_request).await });

        for _ in 0..10 {
            if router.interactive_tx.capacity() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }

        let second_request = focus_request(&session_id, &agent_id);
        let second_command =
            KernelCommand::from_local_request("cmd-focus-2", None, None, &second_request);
        let (second_result_tx, second_result_rx) = tokio::sync::oneshot::channel();
        router
            .interactive_tx
            .try_send(super::InteractiveCommandEnvelope {
                command: second_command,
                request: second_request,
                result_tx: second_result_tx,
            })
            .expect("second command should fill the interactive lane");

        for _ in 0..10 {
            if router.interactive_tx.capacity() == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }

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

        tokio::task::yield_now().await;
        assert!(
            !prompt_task.is_finished(),
            "prompt should be admitted to the agent lane instead of failing on the full interactive lane"
        );

        drop(app_guard);
        let _ = first_task.await.expect("first focus should join");
        let _ = second_result_rx.await.expect("second focus should resolve");
        let prompt_response = prompt_task
            .await
            .expect("prompt task should join")
            .expect("prompt should submit");
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
        let (session, _default_agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let focused_agent = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session_id.clone(),
                alias: Some("focused".to_string()),
                provider: "claude-code".to_string(),
                model: None,
                effort: None,
                worktree_id: None,
                machine_ref: None,
            }))
            .expect("focused agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected spawn response"),
        };
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(focused_agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

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

        let mut focused_agent_lane_created = false;
        for _ in 0..50 {
            let projection = router.daemon_health_projection(0).await;
            if projection
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == focused_agent.id())
            {
                focused_agent_lane_created = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            focused_agent_lane_created,
            "prompt submit should resolve focused agent from the session projection before touching the app lock"
        );
        assert!(
            !prompt_task.is_finished(),
            "agent worker should still wait on the deliberately held app lock"
        );

        drop(app_guard);
        let prompt_response = prompt_task
            .await
            .expect("prompt task should join")
            .expect("prompt should submit");
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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-session-projection-focus",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

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

        let mut agent_lane_created = false;
        for _ in 0..50 {
            let projection = router.daemon_health_projection(0).await;
            if projection
                .agent_command_lanes
                .iter()
                .any(|lane| lane.lane_id == agent_id)
            {
                agent_lane_created = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            agent_lane_created,
            "prompt submit should resolve focus from warmed session projection before touching the app lock"
        );
        assert!(
            !prompt_task.is_finished(),
            "agent worker should still wait on the deliberately held app lock"
        );

        drop(app_guard);
        let prompt_response = prompt_task
            .await
            .expect("prompt task should join")
            .expect("prompt should submit");
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
        let (session, _default_agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = app
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
            provider: "claude-code".to_string(),
            model: None,
            effort: None,
            worktree_id: None,
            machine_ref: None,
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
            app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session_id.clone(),
                    agent_id: Some(spawned_agent.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                    variant: None,
                },
            ))
            .expect("provider run should launch");
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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

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
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = app
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
        let (session, _agent) = app
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
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let source = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-notice-source",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("source attachment should attach");
        let recipient = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-notice-recipient",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("recipient attachment should attach");
        app.update_session_config(
            &session_id,
            source.id(),
            BTreeMap::from([("theme".to_string(), "compact".to_string())]),
            false,
        )
        .expect("config update should create a notice");

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
        let (session, _agent) = app
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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-complete-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-prompt-shadow-refresh",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

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
            let mut app = app.lock().await;
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
        let (session, default_agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let default_agent_id = default_agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-complete-owner-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let spawned_agent = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session_id.clone(),
                alias: Some("worker".to_string()),
                provider: "claude-code".to_string(),
                model: None,
                effort: None,
                worktree_id: None,
                machine_ref: None,
            }))
            .expect("agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected spawn response"),
        };
        let spawned_agent_id = spawned_agent.id().to_string();
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(spawned_agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");
        app.handle_local_request(focus_request(&session_id, &default_agent_id))
            .expect("default agent should regain focus");
        let idle_session_snapshot = app
            .local_api_session_snapshot(&session_id)
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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-cancel-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");

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
        let (session, default_agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let default_agent_id = default_agent.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-cancel-owner-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let spawned_agent = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session_id.clone(),
                alias: Some("worker".to_string()),
                provider: "claude-code".to_string(),
                model: None,
                effort: None,
                worktree_id: None,
                machine_ref: None,
            }))
            .expect("agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected spawn response"),
        };
        let spawned_agent_id = spawned_agent.id().to_string();
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(spawned_agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");
        app.handle_local_request(focus_request(&session_id, &default_agent_id))
            .expect("default agent should regain focus");
        let idle_session_snapshot = app
            .local_api_session_snapshot(&session_id)
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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
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
    async fn warmed_session_history_projection_tracks_appends_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = app
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
        let (session, agent) = app
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
        let (session, agent) = app
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
        let (session, agent) = app
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
        let (mut session, agent) = app
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
        let (session, agent) = app
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
            let (session, agent) = app
                .create_session(CreateSessionRequest::new(
                    format!("workspace-{idx}"),
                    format!("worktree-{idx}"),
                ))
                .expect("session should be created");
            app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(agent.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: provider.to_string(),
                    account_profile: "default".to_string(),
                    model: model.to_string(),
                    variant: None,
                },
            ))
            .expect("provider run should launch");
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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        app.handle_local_request(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            },
        ))
        .expect("provider run should launch");
        app.list_provider_processes(None)
            .expect("process list should warm projection");
        app.teardown_provider_processes(None)
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
        let (session, agent) = app
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
        app.handle_local_request(LocalDaemonRequest::ConfigureRelay(ConfigureRelayRequest {
            relay_url: None,
            relay_token: None,
        }))
        .expect("relay configure should invalidate provider catalog projection");

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
        let (session, _agent) = app
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
        let (session, _agent) = app
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
        let (session, _agent) = app
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
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        app.attach(crate::attachment::AttachRequest::new(
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
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = app
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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-pump-active",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let provider_run_id = match app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session_id.clone(),
                    agent_id: Some(agent.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                    variant: None,
                },
            ))
            .expect("provider run should launch")
        {
            LocalDaemonResponse::ProviderRunLaunched { provider_run } => {
                provider_run.id().to_string()
            }
            _ => panic!("unexpected launch response"),
        };

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
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let attachment = app
            .attach(crate::attachment::AttachRequest::new(
                &session_id,
                "cli-pump-parked",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let mut projected_session = app
            .local_api_session_snapshot(&session_id)
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
        let (session, _default_agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

        let spawn_request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("reviewer".to_string()),
            provider: "claude-code".to_string(),
            model: None,
            effort: None,
            worktree_id: None,
            machine_ref: None,
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
}
