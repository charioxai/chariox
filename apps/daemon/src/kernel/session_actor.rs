use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel::projection::{
    ActorQueueSnapshot, AgentRuntimeProjectionStore, SessionStateProjectionStore,
};
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::terminal::TerminalStreamStore;

pub(crate) const SESSION_COMMAND_QUEUE_LIMIT: usize = 128;
pub(crate) const SESSION_CREATE_LANE_ID: &str = "__session_create__";

#[derive(Clone, Default)]
pub(crate) struct FocusedAgentProjection {
    focused_agents: Arc<Mutex<HashMap<String, String>>>,
}

impl FocusedAgentProjection {
    pub(crate) async fn update(&self, session_id: &str, agent_id: Option<&str>) {
        let mut focused_agents = self.focused_agents.lock().await;
        match agent_id {
            Some(agent_id) => {
                focused_agents.insert(session_id.to_string(), agent_id.to_string());
            }
            None => {
                focused_agents.remove(session_id);
            }
        }
    }

    pub(crate) async fn remove(&self, session_id: &str) {
        self.focused_agents.lock().await.remove(session_id);
    }

    pub(crate) async fn focused_agent_id(&self, session_id: &str) -> Option<String> {
        self.focused_agents.lock().await.get(session_id).cloned()
    }
}

#[derive(Debug)]
struct SessionCommandEnvelope {
    command_id: String,
    command_type: String,
    request: LocalDaemonRequest,
    result_tx: oneshot::Sender<Result<LocalDaemonResponse, DaemonError>>,
}

#[derive(Clone)]
pub(crate) struct SessionRuntime {
    app: Arc<Mutex<DaemonApp>>,
    queue_limit: usize,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    terminal_stream: TerminalStreamStore,
    lanes: Arc<Mutex<HashMap<String, mpsc::Sender<SessionCommandEnvelope>>>>,
}

impl SessionRuntime {
    pub(crate) fn with_queue_limit_and_focus_projection(
        app: Arc<Mutex<DaemonApp>>,
        queue_limit: usize,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        terminal_stream: TerminalStreamStore,
    ) -> Self {
        Self {
            app,
            queue_limit,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            terminal_stream,
            lanes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) async fn dispatch_session_command(
        &self,
        command: crate::kernel::command::KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session_id = self.resolve_session_lane_key(&request).await?;
        let lane = self.session_lane(&session_id).await;
        let (result_tx, result_rx) = oneshot::channel();
        lane.try_send(SessionCommandEnvelope {
            command_id: command.command_id,
            command_type: command.command_type,
            request,
            result_tx,
        })
        .map_err(|error| DaemonError::LocalTransport {
            operation: "enqueue session kernel command",
            message: format!("session command lane overloaded: {error}"),
        })?;
        let response = result_rx
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "await session kernel command",
                message: error.to_string(),
            })??;
        if matches!(
            response,
            LocalDaemonResponse::SessionEnded { .. } | LocalDaemonResponse::SessionDeleted { .. }
        ) {
            self.remove_session_lane(&session_id).await;
        }
        Ok(response)
    }

    async fn resolve_session_lane_key(
        &self,
        request: &LocalDaemonRequest,
    ) -> Result<String, DaemonError> {
        match request {
            LocalDaemonRequest::CreateSession(_) => Ok(SESSION_CREATE_LANE_ID.to_string()),
            LocalDaemonRequest::AttachToSession(request) => {
                self.resolve_direct_session_lane_key(&request.session_id)
            }
            LocalDaemonRequest::FocusAgent(request) => {
                self.resolve_direct_session_lane_key(&request.session_id)
            }
            LocalDaemonRequest::CycleAgentFocus(request) => {
                self.resolve_direct_session_lane_key(&request.session_id)
            }
            LocalDaemonRequest::ResizeTerminal(request) => {
                self.resolve_direct_session_lane_key(&request.session_id)
            }
            LocalDaemonRequest::PollRuntimeNotices(request) => self
                .resolve_attachment_scoped_session_lane_key(
                    &request.session_id,
                    &request.attachment_id,
                ),
            LocalDaemonRequest::UpdateSessionConfig(request) => self
                .resolve_attachment_scoped_session_lane_key(
                    &request.session_id,
                    &request.attachment_id,
                ),
            LocalDaemonRequest::AliasSession(request) => {
                self.resolve_direct_session_lane_key(&request.session_id)
            }
            LocalDaemonRequest::SpawnAgent(request) => {
                self.resolve_direct_session_lane_key(&request.session_id)
            }
            LocalDaemonRequest::DestroyAgent(request) => {
                self.resolve_direct_session_lane_key(&request.session_id)
            }
            LocalDaemonRequest::EndSession(request) => {
                self.resolve_direct_session_lane_key(&request.session_id)
            }
            LocalDaemonRequest::DeleteSession(request) => {
                if let Some(session_id) = self
                    .session_projection
                    .resolve_session_ref_id(&request.session_ref, request.workspace_id.as_deref())
                {
                    return Ok(session_id);
                }
                if let Some(result) = self
                    .session_projection
                    .resolve_session_ref_id_from_warmed_list(
                        &request.session_ref,
                        request.workspace_id.as_deref(),
                    )
                {
                    return result;
                }
                let app = self.app.lock().await;
                Ok(app
                    .resolve_session_ref(&request.session_ref, request.workspace_id.as_deref())?
                    .id()
                    .to_string())
            }
            LocalDaemonRequest::DetachFromSession(request) => {
                if let Some(session_id) = self
                    .session_projection
                    .session_id_for_attachment(&request.attachment_id)
                {
                    return Ok(session_id);
                }
                if self.session_projection.has_warmed_list() {
                    return Err(DaemonError::AttachmentNotFound {
                        attachment_id: request.attachment_id.clone(),
                    });
                }
                let app = self.app.lock().await;
                Ok(app
                    .attachments()
                    .get_attachment(&request.attachment_id)?
                    .session_id()
                    .to_string())
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "route session kernel command",
                message: "request is not handled by the session runtime".to_string(),
            }),
        }
    }

    fn resolve_direct_session_lane_key(&self, session_id: &str) -> Result<String, DaemonError> {
        if self.session_projection.get(session_id).is_some()
            || !self.session_projection.has_warmed_list()
        {
            return Ok(session_id.to_string());
        }
        Err(DaemonError::SessionNotFound {
            session_id: session_id.to_string(),
        })
    }

    fn resolve_attachment_scoped_session_lane_key(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<String, DaemonError> {
        if self
            .session_projection
            .get(session_id)
            .is_some_and(|session| session.has_attachment(attachment_id))
            || !self.session_projection.has_warmed_list()
        {
            return Ok(session_id.to_string());
        }
        if self
            .session_projection
            .session_id_for_attachment(attachment_id)
            .is_some()
        {
            return Err(DaemonError::AttachmentNotInSession {
                session_id: session_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }
        Err(DaemonError::AttachmentNotFound {
            attachment_id: attachment_id.to_string(),
        })
    }

    async fn session_lane(&self, session_id: &str) -> mpsc::Sender<SessionCommandEnvelope> {
        let mut lanes = self.lanes.lock().await;
        if let Some(lane) = lanes.get(session_id) {
            return lane.clone();
        }
        let (tx, rx) = mpsc::channel(self.queue_limit);
        lanes.insert(session_id.to_string(), tx.clone());
        tokio::spawn(run_session_command_lane(
            Arc::clone(&self.app),
            self.focus_projection.clone(),
            self.session_projection.clone(),
            self.agent_runtime_projection.clone(),
            self.terminal_stream.clone(),
            session_id.to_string(),
            rx,
        ));
        tx
    }

    async fn remove_session_lane(&self, session_id: &str) {
        let mut lanes = self.lanes.lock().await;
        lanes.remove(session_id);
    }

    #[allow(dead_code)]
    pub(crate) async fn queue_snapshots(&self) -> Vec<ActorQueueSnapshot> {
        let lanes = self.lanes.lock().await;
        let mut snapshots = lanes
            .iter()
            .map(|(session_id, sender)| {
                ActorQueueSnapshot::new(
                    session_id.clone(),
                    self.queue_limit,
                    self.queue_limit.saturating_sub(sender.capacity()),
                )
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.lane_id.cmp(&right.lane_id));
        snapshots
    }

    #[cfg(test)]
    pub(crate) async fn has_lane(&self, session_id: &str) -> bool {
        let lanes = self.lanes.lock().await;
        lanes.contains_key(session_id)
    }

    #[cfg(test)]
    pub(crate) async fn lane_capacity(&self, session_id: &str) -> Option<usize> {
        let lanes = self.lanes.lock().await;
        lanes.get(session_id).map(mpsc::Sender::capacity)
    }

    #[cfg(test)]
    pub(crate) async fn enqueue_for_test(
        &self,
        session_id: &str,
        command_id: impl Into<String>,
        command_type: impl Into<String>,
        request: LocalDaemonRequest,
    ) -> Result<oneshot::Receiver<Result<LocalDaemonResponse, DaemonError>>, DaemonError> {
        let lane = self.session_lane(session_id).await;
        let (result_tx, result_rx) = oneshot::channel();
        lane.try_send(SessionCommandEnvelope {
            command_id: command_id.into(),
            command_type: command_type.into(),
            request,
            result_tx,
        })
        .map_err(|error| DaemonError::LocalTransport {
            operation: "enqueue test session kernel command",
            message: format!("session command lane overloaded: {error}"),
        })?;
        Ok(result_rx)
    }
}

async fn run_session_command_lane(
    app: Arc<Mutex<DaemonApp>>,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    terminal_stream: TerminalStreamStore,
    session_id: String,
    mut rx: mpsc::Receiver<SessionCommandEnvelope>,
) {
    let executor = SessionRuntimeCommandExecutor::new(
        app,
        focus_projection,
        session_projection,
        agent_runtime_projection,
        terminal_stream,
        session_id.clone(),
    );
    while let Some(envelope) = rx.recv().await {
        crate::logging::info_with_fields(
            "daemon.kernel_session_actor",
            "session kernel command dispatched",
            serde_json::json!({
                "session_id": session_id,
                "command_id": envelope.command_id,
                "command_type": envelope.command_type,
            }),
        );
        let result = executor.execute(envelope.request).await;
        let _ = envelope.result_tx.send(result);
    }
}

#[derive(Clone)]
struct SessionRuntimeCommandExecutor {
    app: Arc<Mutex<DaemonApp>>,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    terminal_stream: TerminalStreamStore,
    session_id: String,
}

impl SessionRuntimeCommandExecutor {
    fn new(
        app: Arc<Mutex<DaemonApp>>,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        terminal_stream: TerminalStreamStore,
        session_id: String,
    ) -> Self {
        Self {
            app,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            terminal_stream,
            session_id,
        }
    }

    async fn execute(
        &self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (result, projection_action) = if let Some(result) = projected_runtime_notices_response(
            &self.session_projection,
            &self.terminal_stream,
            &request,
        ) {
            let projection_action = if result.is_ok() {
                session_id_for_projection_refresh(&result)
                    .and_then(|session_id| self.session_projection.get(&session_id))
                    .map(SessionProjectionAction::Update)
            } else {
                None
            };
            (result, projection_action)
        } else if let Some(result) =
            projected_resize_terminal_response(&self.session_projection, &request)
        {
            let projection_action = if result.is_ok() {
                session_id_for_projection_refresh(&result)
                    .and_then(|session_id| self.session_projection.get(&session_id))
                    .map(SessionProjectionAction::Update)
            } else {
                None
            };
            (result, projection_action)
        } else if let Some(result) =
            projected_config_update_absence_response(&self.session_projection, &request)
        {
            (result, None)
        } else if let Some(result) =
            projected_session_absence_response(&self.session_projection, &request)
        {
            (result, None)
        } else {
            let mut app = self.app.lock().await;
            let result = app.kernel_sessions().execute_request(request);
            let projection_action = if let Ok(response) = result.as_ref() {
                session_response_projection_action(response).or_else(|| {
                    session_id_for_projection_refresh(&result)
                        .and_then(|session_id| app.local_api_session_snapshot(&session_id).ok())
                        .map(SessionProjectionAction::Update)
                })
            } else {
                None
            };
            (result, projection_action)
        };
        let projected_session = match projection_action {
            Some(SessionProjectionAction::Update(session)) => {
                self.agent_runtime_projection.update_session(&session);
                self.session_projection.update(session.clone());
                Some(session)
            }
            Some(SessionProjectionAction::Remove { session_id }) => {
                self.agent_runtime_projection.remove_session(&session_id);
                self.session_projection.remove(&session_id);
                None
            }
            None => None,
        };
        update_focus_projection_after_session_command(
            &self.focus_projection,
            &self.session_id,
            &result,
            projected_session
                .as_ref()
                .and_then(|session| session.focused_agent_id()),
        )
        .await;
        if matches!(
            result,
            Ok(LocalDaemonResponse::SessionEnded { .. })
                | Ok(LocalDaemonResponse::SessionDeleted { .. })
        ) {
            self.terminal_stream.remove_session(&self.session_id);
        }
        result
    }
}

enum SessionProjectionAction {
    Update(crate::session::RuntimeSession),
    Remove { session_id: String },
}

async fn update_focus_projection_after_session_command(
    focus_projection: &FocusedAgentProjection,
    session_id: &str,
    result: &Result<LocalDaemonResponse, DaemonError>,
    focused_agent_id: Option<&str>,
) {
    match result {
        Ok(LocalDaemonResponse::SessionCreated { session, .. }) => {
            focus_projection
                .update(
                    session.id(),
                    focused_agent_id.or_else(|| session.focused_agent_id()),
                )
                .await;
        }
        Ok(LocalDaemonResponse::SessionEnded { .. })
        | Ok(LocalDaemonResponse::SessionDeleted { .. }) => {
            focus_projection.remove(session_id).await;
        }
        Ok(_) => {
            focus_projection.update(session_id, focused_agent_id).await;
        }
        Err(_) => {}
    }
}

fn session_response_projection_action(
    response: &LocalDaemonResponse,
) -> Option<SessionProjectionAction> {
    match response {
        LocalDaemonResponse::SessionCreated { session, .. }
        | LocalDaemonResponse::SessionConfigUpdated { session, .. }
        | LocalDaemonResponse::SessionEnded { session }
        | LocalDaemonResponse::SessionAliased { session } => {
            Some(SessionProjectionAction::Update(session.clone()))
        }
        LocalDaemonResponse::SessionDeleted { session } => Some(SessionProjectionAction::Remove {
            session_id: session.id().to_string(),
        }),
        _ => None,
    }
}

fn projected_runtime_notices_response(
    session_projection: &SessionStateProjectionStore,
    terminal_stream: &TerminalStreamStore,
    request: &LocalDaemonRequest,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    let LocalDaemonRequest::PollRuntimeNotices(request) = request else {
        return None;
    };
    if session_projection
        .get(&request.session_id)
        .is_some_and(|session| session.has_attachment(&request.attachment_id))
    {
        return Some(Ok(LocalDaemonResponse::RuntimeNotices {
            notices: terminal_stream
                .drain_notice_records(&request.session_id, &request.attachment_id),
        }));
    }
    if !session_projection.has_warmed_list() {
        return None;
    }
    let result = match session_projection.session_id_for_attachment(&request.attachment_id) {
        Some(_) => Err(DaemonError::AttachmentNotInSession {
            session_id: request.session_id.clone(),
            attachment_id: request.attachment_id.clone(),
        }),
        None => Err(DaemonError::AttachmentNotFound {
            attachment_id: request.attachment_id.clone(),
        }),
    };
    Some(result)
}

fn projected_resize_terminal_response(
    session_projection: &SessionStateProjectionStore,
    request: &LocalDaemonRequest,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    let LocalDaemonRequest::ResizeTerminal(request) = request else {
        return None;
    };
    if let Some(session) = session_projection.get(&request.session_id) {
        if session.active_provider_run_id().is_none() {
            return Some(Err(DaemonError::NoActiveProviderRun {
                session_id: request.session_id.clone(),
            }));
        }
        return None;
    }
    if !session_projection.has_warmed_list() {
        return None;
    }
    Some(Err(DaemonError::SessionNotFound {
        session_id: request.session_id.clone(),
    }))
}

fn projected_config_update_absence_response(
    session_projection: &SessionStateProjectionStore,
    request: &LocalDaemonRequest,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    let LocalDaemonRequest::UpdateSessionConfig(request) = request else {
        return None;
    };
    if session_projection
        .get(&request.session_id)
        .is_some_and(|session| session.has_attachment(&request.attachment_id))
    {
        return None;
    }
    if !session_projection.has_warmed_list() {
        return None;
    }
    let result = match session_projection.session_id_for_attachment(&request.attachment_id) {
        Some(_) => Err(DaemonError::AttachmentNotInSession {
            session_id: request.session_id.clone(),
            attachment_id: request.attachment_id.clone(),
        }),
        None => Err(DaemonError::AttachmentNotFound {
            attachment_id: request.attachment_id.clone(),
        }),
    };
    Some(result)
}

fn projected_session_absence_response(
    session_projection: &SessionStateProjectionStore,
    request: &LocalDaemonRequest,
) -> Option<Result<LocalDaemonResponse, DaemonError>> {
    let session_id = match request {
        LocalDaemonRequest::AttachToSession(request) => &request.session_id,
        LocalDaemonRequest::FocusAgent(request) => &request.session_id,
        LocalDaemonRequest::CycleAgentFocus(request) => &request.session_id,
        LocalDaemonRequest::AliasSession(request) => &request.session_id,
        LocalDaemonRequest::EndSession(request) => &request.session_id,
        _ => return None,
    };
    let Some(session) = session_projection.get(session_id) else {
        if session_projection.has_warmed_list() {
            return Some(Err(DaemonError::SessionNotFound {
                session_id: session_id.clone(),
            }));
        }
        return None;
    };
    if let LocalDaemonRequest::FocusAgent(request) = request {
        if !session
            .agents()
            .iter()
            .any(|agent| agent.id() == request.agent_id)
        {
            return Some(Err(DaemonError::AgentNotInSession {
                session_id: request.session_id.clone(),
                agent_id: request.agent_id.clone(),
            }));
        }
    }
    None
}

fn session_id_for_projection_refresh(
    result: &Result<LocalDaemonResponse, DaemonError>,
) -> Option<String> {
    match result {
        Ok(LocalDaemonResponse::SessionAttached { attachment })
        | Ok(LocalDaemonResponse::SessionDetached { attachment }) => {
            Some(attachment.session_id().to_string())
        }
        Ok(LocalDaemonResponse::SessionCreated { session, .. }) => Some(session.id().to_string()),
        Ok(LocalDaemonResponse::AgentFocused { agent }) => Some(agent.session_id().to_string()),
        Ok(LocalDaemonResponse::AgentFocusCycled { agent: Some(agent) }) => {
            Some(agent.session_id().to_string())
        }
        Ok(LocalDaemonResponse::AgentFocusCycled { agent: None }) => None,
        Ok(LocalDaemonResponse::TerminalResized { session_id, .. }) => Some(session_id.clone()),
        Ok(LocalDaemonResponse::SessionConfigUpdated { session, .. }) => {
            Some(session.id().to_string())
        }
        Ok(LocalDaemonResponse::SessionAliased { session }) => Some(session.id().to_string()),
        _ => None,
    }
}

pub(crate) struct SessionActor;

impl SessionActor {
    pub(crate) fn is_session_interactive_command(request: &LocalDaemonRequest) -> bool {
        matches!(
            request,
            LocalDaemonRequest::CreateSession(_)
                | LocalDaemonRequest::AttachToSession(_)
                | LocalDaemonRequest::DetachFromSession(_)
                | LocalDaemonRequest::FocusAgent(_)
                | LocalDaemonRequest::CycleAgentFocus(_)
                | LocalDaemonRequest::ResizeTerminal(_)
                | LocalDaemonRequest::PollRuntimeNotices(_)
                | LocalDaemonRequest::UpdateSessionConfig(_)
                | LocalDaemonRequest::AliasSession(_)
                | LocalDaemonRequest::SpawnAgent(_)
                | LocalDaemonRequest::DestroyAgent(_)
                | LocalDaemonRequest::EndSession(_)
                | LocalDaemonRequest::DeleteSession(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::kernel::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
    use crate::kernel::session_actor::{
        projected_config_update_absence_response, session_response_projection_action,
        FocusedAgentProjection, SessionProjectionAction, SessionRuntime,
    };
    use crate::local::{
        AttachToSessionRequest, EndSessionRequest, FocusAgentRequest, LaunchProviderRunRequest,
        LocalDaemonRequest, LocalDaemonResponse, PollRuntimeNoticesRequest, ResizeTerminalRequest,
        SpawnAgentRequest, SubmitPromptRequest, UpdateSessionConfigRequest,
    };
    use crate::session::{CreateSessionRequest, PromptSubmissionOutcome};
    use crate::terminal::TerminalOutputKind;
    use crate::{DaemonApp, DaemonConfig, DaemonError};
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    #[test]
    fn session_response_projection_action_uses_response_session_and_removes_deleted_sessions() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_snapshot = app
            .local_api_session_snapshot(session.id())
            .expect("session snapshot should be available");

        match session_response_projection_action(&LocalDaemonResponse::SessionAliased {
            session: session_snapshot.clone(),
        }) {
            Some(SessionProjectionAction::Update(projected)) => {
                assert_eq!(projected.id(), session.id());
            }
            _ => panic!("session-bearing response should update projections"),
        }

        match session_response_projection_action(&LocalDaemonResponse::SessionDeleted {
            session: session_snapshot,
        }) {
            Some(SessionProjectionAction::Remove { session_id }) => {
                assert_eq!(session_id, session.id());
            }
            _ => panic!("deleted-session response should remove projections"),
        }
    }

    #[tokio::test]
    async fn direct_session_lane_resolution_rejects_warmed_missing_session_without_lane() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let session_projection = SessionStateProjectionStore::default();
        session_projection.update_list(Vec::new());
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            Arc::clone(&app),
            1,
            FocusedAgentProjection::default(),
            session_projection,
            AgentRuntimeProjectionStore::default(),
            {
                let app = app.lock().await;
                app.terminal_stream_store()
            },
        );

        let _locked_app = app.lock().await;
        let request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
            session_id: "missing-session".to_string(),
            cols: 80,
            rows: 24,
        });
        let error = timeout(
            Duration::from_millis(100),
            runtime.resolve_session_lane_key(&request),
        )
        .await
        .expect("warmed missing session lane resolution should not wait for the app lock")
        .expect_err("missing direct session lane should fail");

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
        assert!(
            !runtime.has_lane("missing-session").await,
            "missing direct session should be rejected before creating a session lane"
        );
    }

    #[tokio::test]
    async fn config_update_rejects_warmed_missing_attachment_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let session_projection = SessionStateProjectionStore::default();
        session_projection.update_list(Vec::new());
        let request = LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
            session_id: "missing-session".to_string(),
            attachment_id: "missing-attachment".to_string(),
            values: Default::default(),
            requires_idle: false,
        });

        let _locked_app = app.lock().await;
        let result = timeout(Duration::from_millis(100), async {
            projected_config_update_absence_response(&session_projection, &request)
        })
        .await
        .expect("projected config validation should not wait for the app lock")
        .expect("warmed projection should handle missing attachment");
        let error = result.expect_err("missing attachment should fail");

        match error {
            DaemonError::AttachmentNotFound { attachment_id } => {
                assert_eq!(attachment_id, "missing-attachment");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn attachment_scoped_session_lane_resolution_rejects_warmed_missing_attachment_without_lane(
    ) {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let session_projection = SessionStateProjectionStore::default();
        session_projection.update_list(Vec::new());
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            Arc::clone(&app),
            1,
            FocusedAgentProjection::default(),
            session_projection,
            AgentRuntimeProjectionStore::default(),
            {
                let app = app.lock().await;
                app.terminal_stream_store()
            },
        );

        let _locked_app = app.lock().await;
        let request = LocalDaemonRequest::PollRuntimeNotices(PollRuntimeNoticesRequest {
            session_id: "missing-session".to_string(),
            attachment_id: "missing-attachment".to_string(),
        });
        let error = timeout(
            Duration::from_millis(100),
            runtime.resolve_session_lane_key(&request),
        )
        .await
        .expect("warmed missing attachment lane resolution should not wait for the app lock")
        .expect_err("missing attachment should fail before lane creation");

        match error {
            DaemonError::AttachmentNotFound { attachment_id } => {
                assert_eq!(attachment_id, "missing-attachment");
            }
            error => panic!("unexpected error: {error}"),
        }
        assert!(
            !runtime.has_lane("missing-session").await,
            "missing attachment should be rejected before creating a session lane"
        );
    }

    #[tokio::test]
    async fn session_end_clears_terminal_stream_records() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (session_id, terminal_stream) = {
            let mut app = app.lock().await;
            let (session, _agent) = app
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let attachment = app
                .attach(AttachRequest::new(
                    session.id(),
                    "cli-terminal-cleanup",
                    ClientCapabilityLevel::FullTerminal,
                ))
                .expect("attachment should attach");
            let terminal_stream = app.terminal_stream_store();
            terminal_stream.record_input(session.id(), "provider-run-1", attachment.id(), b"input");
            terminal_stream.fan_out_output(
                session.id(),
                "provider-run-1",
                None,
                TerminalOutputKind::ProviderOutput,
                None,
                vec![attachment.id().to_string()],
                b"output",
            );
            terminal_stream.record_notice(
                session.id(),
                None,
                None,
                vec![attachment.id().to_string()],
                "notice",
            );
            terminal_stream.record_assistant_message_completion(
                session.id(),
                "provider-run-1",
                None,
                vec![attachment.id().to_string()],
                "message-1",
                1,
            );
            (session.id().to_string(), terminal_stream)
        };
        assert_eq!(terminal_stream.health_snapshot().pending_output_records, 1);
        assert_eq!(terminal_stream.health_snapshot().pending_notice_records, 1);
        assert_eq!(
            terminal_stream.health_snapshot().pending_completion_records,
            1
        );

        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            Arc::clone(&app),
            1,
            FocusedAgentProjection::default(),
            SessionStateProjectionStore::default(),
            AgentRuntimeProjectionStore::default(),
            terminal_stream.clone(),
        );
        let request = LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session_id.clone(),
        });
        let command = crate::kernel::command::KernelCommand::from_local_request(
            "cmd-end-session-cleanup",
            None,
            None,
            &request,
        );
        runtime
            .dispatch_session_command(command, request)
            .await
            .expect("session end should succeed");

        assert!(terminal_stream.input_records().is_empty());
        assert!(terminal_stream.output_records().is_empty());
        assert!(terminal_stream.notice_records().is_empty());
        assert_eq!(terminal_stream.health_snapshot().pending_output_records, 0);
        assert_eq!(terminal_stream.health_snapshot().pending_notice_records, 0);
        assert_eq!(
            terminal_stream.health_snapshot().pending_completion_records,
            0
        );
    }

    #[test]
    fn handles_attach_through_session_actor_surface() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let response = app
            .kernel_sessions()
            .execute_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "cli-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed");

        assert!(matches!(
            response,
            LocalDaemonResponse::SessionAttached { .. }
        ));
    }

    #[test]
    fn focus_does_not_disturb_multi_agent_provider_liveness() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, default_agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let default_run = match app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(default_agent.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                    variant: None,
                },
            ))
            .expect("default provider launch should succeed")
        {
            LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
            _ => panic!("unexpected local response"),
        };

        let second_agent = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("agent-b".to_string()),
                provider: "claude-code".to_string(),
                model: None,
                effort: None,
                worktree_id: None,
                machine_ref: None,
            }))
            .expect("second agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        };

        let _second_run = match app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(second_agent.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "opus".to_string(),
                    variant: None,
                },
            ))
            .expect("second provider launch should succeed")
        {
            LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
            _ => panic!("unexpected local response"),
        };

        app.kernel_sessions()
            .execute_request(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: session.id().to_string(),
                agent_id: default_agent.id().to_string(),
            }))
            .expect("focus should succeed");

        let started = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: None,
                prompt: "hello".to_string(),
                attachments: Vec::new(),
            }))
            .expect("prompt should start");

        match started {
            LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), default_agent.id());
                }
                _ => panic!("expected prompt to start immediately"),
            },
            _ => panic!("unexpected local response"),
        }

        let response = app
            .kernel_sessions()
            .execute_request(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: session.id().to_string(),
                agent_id: second_agent.id().to_string(),
            }))
            .expect("focus should succeed");

        assert!(matches!(response, LocalDaemonResponse::AgentFocused { .. }));
        let session_state = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist");
        assert_eq!(session_state.focused_agent_id(), Some(second_agent.id()));
        assert_eq!(
            session_state.active_provider_run_id(),
            Some(default_run.id())
        );
        assert!(session_state
            .active_prompt_for_agent(default_agent.id())
            .is_some());
    }
}
