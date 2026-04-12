use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{sleep, Duration};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::history::SessionHistoryStore;
use crate::kernel::agent_actor::{AgentActor, AgentRuntime};
use crate::kernel::capability_executor::execute_capability_request;
use crate::kernel::command::{KernelCommand, KernelCommandPriority};
use crate::kernel::projection::{
    page_history_entries, DaemonHealthProjection, SessionHistoryProjectionStore,
    SessionStateProjectionStore,
};
use crate::kernel::session_actor::{FocusedAgentProjection, SessionActor, SessionRuntime};
use crate::local::provider_requests::{
    launch_provider_request_from_local, load_provider_catalog, logout_provider_response,
    provider_auth_status_response, provider_command_catalogs_response,
    start_provider_login_response,
};
use crate::local::{
    GetSessionHistoryRequest, LaunchProviderRunRequest, ListProviderProcessesRequest,
    LocalDaemonRequest, LocalDaemonResponse, RelayStatus, TeardownProviderProcessesRequest,
};
use crate::provider::{ProviderRunOperationLanes, ProviderRunState};

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
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    history_store: SessionHistoryStore,
    history_projection: SessionHistoryProjectionStore,
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
        let session_projection = SessionStateProjectionStore::default();
        let (history_store, history_projection) = router_history_stores(&app);
        let pending_provider_launch_sessions = Arc::new(Mutex::new(HashSet::new()));
        let agent_runtime = AgentRuntime::new(
            Arc::clone(&app),
            provider_runtime_lanes,
            focus_projection.clone(),
        );
        let session_runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            Arc::clone(&app),
            session_capacity,
            focus_projection.clone(),
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
            focus_projection,
            session_projection,
            history_store,
            history_projection,
            pending_provider_launch_sessions,
        }
    }

    pub(crate) fn with_interactive_capacity_and_provider_lanes(
        app: Arc<Mutex<DaemonApp>>,
        interactive_capacity: usize,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) -> Self {
        let (interactive_tx, interactive_rx) = mpsc::channel(interactive_capacity);
        let focus_projection = FocusedAgentProjection::default();
        let session_projection = SessionStateProjectionStore::default();
        let (history_store, history_projection) = router_history_stores(&app);
        let pending_provider_launch_sessions = Arc::new(Mutex::new(HashSet::new()));
        let agent_runtime = AgentRuntime::new(
            Arc::clone(&app),
            provider_runtime_lanes.clone(),
            focus_projection.clone(),
        );
        let session_runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            Arc::clone(&app),
            crate::kernel::session_actor::SESSION_COMMAND_QUEUE_LIMIT,
            focus_projection.clone(),
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
            focus_projection,
            session_projection,
            history_store,
            history_projection,
            pending_provider_launch_sessions,
        }
    }

    pub(crate) async fn dispatch(
        &self,
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let focus_refresh = focus_projection_refresh(&request);
        if let LocalDaemonRequest::GetSessionState(request) = &request {
            if !self.has_pending_provider_launch(&request.session_id).await {
                if let Some(session) = self.session_projection.get(&request.session_id).await {
                    return Ok(LocalDaemonResponse::SessionState { session });
                }
            }
        }
        if matches!(request, LocalDaemonRequest::ListSessions(_)) {
            if let Some(sessions) = self.session_projection.list().await {
                return Ok(LocalDaemonResponse::SessionsListed { sessions });
            }
        }
        if let LocalDaemonRequest::GetSessionHistory(request) = &request {
            if let Some(response) = self.projected_session_history_response(request).await {
                return Ok(response);
            }
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
            request => match command.priority {
                KernelCommandPriority::Interactive => {
                    self.dispatch_interactive(command, request).await
                }
                KernelCommandPriority::Normal | KernelCommandPriority::Background => {
                    execute_local_request_with_async_boundaries(&self.app, request).await
                }
            },
        };
        self.apply_focus_projection_refresh(focus_refresh, &result)
            .await;
        self.apply_session_projection_refresh(session_refresh, &result)
            .await;
        self.apply_provider_launch_projection_state(&result).await;
        result
    }

    async fn projected_session_history_response(
        &self,
        request: &GetSessionHistoryRequest,
    ) -> Option<LocalDaemonResponse> {
        if let Some(page) = self.history_projection.page(
            &request.session_id,
            request.agent_id.as_deref(),
            request.round_count,
            request.max_chars,
            request.before_entry_index,
            request.before_entry_char_offset,
        ) {
            return Some(LocalDaemonResponse::SessionHistory {
                entries: page.entries,
                next_cursor: page.next_cursor,
            });
        }

        let session = self.session_projection.get(&request.session_id).await?;
        self.execute_session_history_request_from_session(session, request.clone())
            .await
            .ok()
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
                let focused_agent_id = {
                    let app = self.app.lock().await;
                    app.sessions()
                        .get_session(&session_id)
                        .ok()
                        .and_then(|session| session.focused_agent_id().map(str::to_string))
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
            self.session_projection.update(session).await;
        }
        if let LocalDaemonResponse::SessionsListed { sessions } = response {
            self.session_projection.update_list(sessions.clone()).await;
        }
        for session_id in response_removed_session_ids(response) {
            self.session_projection.remove(session_id).await;
            self.history_projection.remove(session_id);
            refreshed_session_ids.push(session_id.to_string());
        }

        let mut snapshot_session_ids = refresh.session_ids(response);
        snapshot_session_ids.sort();
        snapshot_session_ids.dedup();
        for session_id in snapshot_session_ids {
            let session = {
                let app = self.app.lock().await;
                app.local_api_session_snapshot(&session_id).ok()
            };
            if let Some(session) = session {
                refreshed_session_ids.push(session.id().to_string());
                self.session_projection.update(session).await;
            } else {
                self.session_projection.remove(&session_id).await;
                refreshed_session_ids.push(session_id);
            }
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

    async fn has_pending_provider_launch(&self, session_id: &str) -> bool {
        self.pending_provider_launch_sessions
            .lock()
            .await
            .contains(session_id)
    }

    async fn clear_provider_launch_pending_if_settled(&self, session_id: &str) {
        if !self.has_pending_provider_launch(session_id).await {
            return;
        }
        let is_still_starting = {
            let app = self.app.lock().await;
            app.sessions()
                .get_session(session_id)
                .ok()
                .and_then(|session| session.active_provider_run_id().map(str::to_string))
                .and_then(|provider_run_id| app.providers().get_run(&provider_run_id).ok())
                .is_some_and(|run| run.state() == ProviderRunState::Starting)
        };
        if !is_still_starting {
            self.pending_provider_launch_sessions
                .lock()
                .await
                .remove(session_id);
        }
    }
}

fn router_history_stores(
    app: &Arc<Mutex<DaemonApp>>,
) -> (SessionHistoryStore, SessionHistoryProjectionStore) {
    let app = app
        .try_lock()
        .expect("CommandRouter should be created before holding the app lock");
    (app.history_store(), app.session_history_projection_store())
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
    SnapshotRequestSession { session_id: String },
    SnapshotAttachmentResponse,
    SnapshotAgentResponse,
}

impl SessionProjectionRefresh {
    fn session_ids(&self, response: &LocalDaemonResponse) -> Vec<String> {
        match self {
            SessionProjectionRefresh::None => Vec::new(),
            SessionProjectionRefresh::SnapshotRequestSession { session_id } => {
                vec![session_id.clone()]
            }
            SessionProjectionRefresh::SnapshotAttachmentResponse => match response {
                LocalDaemonResponse::SessionAttached { attachment }
                | LocalDaemonResponse::SessionDetached { attachment } => {
                    vec![attachment.session_id().to_string()]
                }
                _ => Vec::new(),
            },
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
        LocalDaemonRequest::AttachToSession(_) | LocalDaemonRequest::DetachFromSession(_) => {
            SessionProjectionRefresh::SnapshotAttachmentResponse
        }
        LocalDaemonRequest::FocusAgent(_)
        | LocalDaemonRequest::CycleAgentFocus(_)
        | LocalDaemonRequest::SpawnAgent(_)
        | LocalDaemonRequest::DestroyAgent(_) => SessionProjectionRefresh::SnapshotAgentResponse,
        LocalDaemonRequest::CompletePrompt(request) => {
            SessionProjectionRefresh::SnapshotRequestSession {
                session_id: request.session_id.clone(),
            }
        }
        LocalDaemonRequest::CancelActivePrompt(request) => {
            SessionProjectionRefresh::SnapshotRequestSession {
                session_id: request.session_id.clone(),
            }
        }
        LocalDaemonRequest::PumpTerminalOutput(request) => {
            SessionProjectionRefresh::SnapshotRequestSession {
                session_id: request.session_id.clone(),
            }
        }
        LocalDaemonRequest::PollRuntimeNotices(request) => {
            SessionProjectionRefresh::SnapshotRequestSession {
                session_id: request.session_id.clone(),
            }
        }
        LocalDaemonRequest::ResizeTerminal(request) => {
            SessionProjectionRefresh::SnapshotRequestSession {
                session_id: request.session_id.clone(),
            }
        }
        _ => SessionProjectionRefresh::None,
    }
}

fn response_sessions(response: &LocalDaemonResponse) -> Vec<crate::session::RuntimeSession> {
    match response {
        LocalDaemonResponse::SessionCreated { session, .. }
        | LocalDaemonResponse::SessionResolved { session }
        | LocalDaemonResponse::SessionState { session }
        | LocalDaemonResponse::PromptSubmitted { session, .. }
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
        let result = execute_interactive_request(&app, envelope.request).await;
        let _ = envelope.result_tx.send(result);
    }
}

async fn execute_interactive_request(
    app: &Arc<Mutex<DaemonApp>>,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut app = app.lock().await;
    if let Some(result) = SessionActor::handle_interactive_command(&mut app, request.clone()) {
        return result;
    }
    if let Some(result) = AgentActor::handle_interactive_command(&mut app, request.clone()) {
        return result;
    }
    app.handle_local_request(request)
}

pub(crate) async fn execute_local_request_with_async_boundaries(
    app: &Arc<Mutex<DaemonApp>>,
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
        request if is_capability_request(&request) => execute_capability_request(app, request)
            .await
            .unwrap_or_else(|| {
                Err(DaemonError::LocalTransport {
                    operation: "route capability request",
                    message: "capability request was not handled by executor".to_string(),
                })
            }),
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
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    use crate::attachment::ClientCapabilityLevel;
    use crate::kernel::command::KernelCommand;
    use crate::kernel::router::CommandRouter;
    use crate::local::{
        AttachToSessionRequest, DeleteSessionRequest, EndSessionRequest, FocusAgentRequest,
        GetDaemonHealthRequest, GetSessionHistoryRequest, GetSessionStateRequest,
        LaunchProviderRunRequest, ListSessionsRequest, LocalDaemonRequest, LocalDaemonResponse,
        SpawnAgentRequest, SubmitPromptRequest,
    };
    use crate::session::{CreateSessionRequest, PromptSubmissionOutcome};
    use crate::{DaemonApp, DaemonConfig};

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
