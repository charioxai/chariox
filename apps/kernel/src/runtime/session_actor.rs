use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::command::{KernelCallerKind, KernelCommand};
use crate::runtime::projection::{
    ActorQueueSnapshot, AgentRuntimeProjectionStore, SessionStateProjectionStore,
};
use crate::runtime::state::KernelRuntimeState;
use crate::session::DEFAULT_LOCAL_USER_ID;
use crate::terminal::TerminalStreamStore;

mod lane_resolution;
mod projection_policy;
mod store;

use projection_policy::{
    projected_config_update_absence_response, projected_resize_terminal_response,
    projected_runtime_notices_response, projected_session_absence_response,
    projected_terminal_input_absence_response, session_id_for_projection_refresh,
    update_focus_projection_after_session_command, SessionProjectionAction,
};
use store::SessionRuntimeStore;

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
    caller_user_id: String,
    request: LocalDaemonRequest,
    result_tx: oneshot::Sender<Result<LocalDaemonResponse, DaemonError>>,
}

#[derive(Clone)]
pub(crate) struct SessionRuntime {
    store: SessionRuntimeStore,
    queue_limit: usize,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    terminal_stream: TerminalStreamStore,
    lanes: Arc<Mutex<HashMap<String, mpsc::Sender<SessionCommandEnvelope>>>>,
}

impl SessionRuntime {
    pub(crate) fn with_queue_limit_and_focus_projection(
        state: KernelRuntimeState,
        queue_limit: usize,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        terminal_stream: TerminalStreamStore,
    ) -> Self {
        Self::with_store_and_focus_projection(
            SessionRuntimeStore::new(state),
            queue_limit,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            terminal_stream,
        )
    }

    pub(crate) fn with_store_and_focus_projection(
        store: SessionRuntimeStore,
        queue_limit: usize,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        terminal_stream: TerminalStreamStore,
    ) -> Self {
        Self {
            store,
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
        command: KernelCommand,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session_id = self.resolve_session_lane_key(&request).await?;
        let lane = self.session_lane(&session_id).await;
        let (result_tx, result_rx) = oneshot::channel();
        let caller_user_id = command_session_actor_user_id(&command);
        lane.try_send(SessionCommandEnvelope {
            command_id: command.command_id,
            command_type: command.command_type,
            caller_user_id,
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
        lane_resolution::resolve_session_lane_key(&self.store, &self.session_projection, request)
            .await
    }

    async fn session_lane(&self, session_id: &str) -> mpsc::Sender<SessionCommandEnvelope> {
        let mut lanes = self.lanes.lock().await;
        if let Some(lane) = lanes.get(session_id) {
            return lane.clone();
        }
        let (tx, rx) = mpsc::channel(self.queue_limit);
        lanes.insert(session_id.to_string(), tx.clone());
        tokio::spawn(run_session_command_lane(
            self.store.clone(),
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
            caller_user_id: DEFAULT_LOCAL_USER_ID.to_string(),
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
    store: SessionRuntimeStore,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    terminal_stream: TerminalStreamStore,
    session_id: String,
    mut rx: mpsc::Receiver<SessionCommandEnvelope>,
) {
    let executor = SessionRuntimeCommandExecutor::new(
        store,
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
        let result = executor
            .execute(envelope.request, envelope.caller_user_id)
            .await;
        match &result {
            Ok(response) => crate::logging::info_with_fields(
                "daemon.kernel_session_actor",
                "session kernel command completed",
                serde_json::json!({
                    "session_id": session_id,
                    "command_id": envelope.command_id,
                    "command_type": envelope.command_type,
                    "response_kind": local_response_kind(response),
                }),
            ),
            Err(error) => crate::logging::warn_with_fields(
                "daemon.kernel_session_actor",
                "session kernel command failed",
                serde_json::json!({
                    "session_id": session_id,
                    "command_id": envelope.command_id,
                    "command_type": envelope.command_type,
                    "error": error.to_string(),
                }),
            ),
        }
        let _ = envelope.result_tx.send(result);
    }
}

#[derive(Clone)]
struct SessionRuntimeCommandExecutor {
    store: SessionRuntimeStore,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    terminal_stream: TerminalStreamStore,
    session_id: String,
}

impl SessionRuntimeCommandExecutor {
    fn new(
        store: SessionRuntimeStore,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        terminal_stream: TerminalStreamStore,
        session_id: String,
    ) -> Self {
        Self {
            store,
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
        caller_user_id: String,
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
            projected_terminal_input_absence_response(&self.session_projection, &request)
        {
            (result, None)
        } else if let Some(result) =
            projected_config_update_absence_response(&self.session_projection, &request)
        {
            (result, None)
        } else if let Some(result) =
            projected_session_absence_response(&self.session_projection, &request)
        {
            (result, None)
        } else {
            self.execute_store_request(request, caller_user_id).await
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

    async fn execute_store_request(
        &self,
        request: LocalDaemonRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<SessionProjectionAction>,
    ) {
        match request {
            LocalDaemonRequest::CreateSession(request) => {
                self.store.create_session(request, caller_user_id).await
            }
            LocalDaemonRequest::AttachToSession(request) => {
                self.store.attach_to_session(request).await
            }
            LocalDaemonRequest::DetachFromSession(request) => {
                self.store.detach_from_session(request).await
            }
            LocalDaemonRequest::FocusAgent(request) => {
                self.store.focus_agent(request, caller_user_id).await
            }
            LocalDaemonRequest::CycleAgentFocus(request) => {
                self.store.cycle_agent_focus(request, caller_user_id).await
            }
            LocalDaemonRequest::ResizeTerminal(request) => {
                self.store.resize_terminal(request).await
            }
            LocalDaemonRequest::SendTerminalInput(request) => {
                self.store.send_terminal_input(request).await
            }
            LocalDaemonRequest::PollRuntimeNotices(request) => {
                self.store.poll_runtime_notices(request).await
            }
            LocalDaemonRequest::UpdateSessionConfig(request) => {
                self.store.update_session_config(request).await
            }
            LocalDaemonRequest::UpdateAgentConfig(request) => {
                self.store
                    .update_agent_config(request, caller_user_id)
                    .await
            }
            LocalDaemonRequest::UpdateAgentProfile(request) => {
                self.store
                    .update_agent_profile(request, caller_user_id)
                    .await
            }
            LocalDaemonRequest::AliasAgent(request) => {
                self.store.alias_agent(request, caller_user_id).await
            }
            LocalDaemonRequest::UpdateAgentSubstitutes(request) => {
                self.store
                    .update_agent_substitutes(request, caller_user_id)
                    .await
            }
            LocalDaemonRequest::RespondToInteraction(request) => {
                self.store.respond_to_interaction(request).await
            }
            LocalDaemonRequest::AliasSession(request) => self.store.alias_session(request).await,
            LocalDaemonRequest::SpawnAgent(request) => {
                self.store.spawn_agent(request, caller_user_id).await
            }
            LocalDaemonRequest::DestroyAgent(request) => {
                self.store.destroy_agent(request, caller_user_id).await
            }
            LocalDaemonRequest::EndSession(request) => self.store.end_session(request).await,
            LocalDaemonRequest::DeleteSession(request) => self.store.delete_session(request).await,
            _ => (
                Err(DaemonError::LocalTransport {
                    operation: "execute session request",
                    message: "request is not handled by the session runtime".to_string(),
                }),
                None,
            ),
        }
    }
}

fn command_session_actor_user_id(command: &KernelCommand) -> String {
    match command.caller.caller_kind {
        KernelCallerKind::LocalClient => command
            .caller
            .user_id
            .clone()
            .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string()),
        KernelCallerKind::RemoteClient
        | KernelCallerKind::RemoteKernel
        | KernelCallerKind::HostedService => command
            .caller
            .user_id
            .clone()
            .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string()),
    }
}

fn local_response_kind(response: &LocalDaemonResponse) -> &'static str {
    match response {
        LocalDaemonResponse::SessionCreated { .. } => "SessionCreated",
        LocalDaemonResponse::SessionAttached { .. } => "SessionAttached",
        LocalDaemonResponse::SessionDetached { .. } => "SessionDetached",
        LocalDaemonResponse::SessionConfigUpdated { .. } => "SessionConfigUpdated",
        LocalDaemonResponse::SessionAliased { .. } => "SessionAliased",
        LocalDaemonResponse::SessionEnded { .. } => "SessionEnded",
        LocalDaemonResponse::SessionDeleted { .. } => "SessionDeleted",
        LocalDaemonResponse::AgentSpawned { .. } => "AgentSpawned",
        LocalDaemonResponse::AgentDestroyed { .. } => "AgentDestroyed",
        LocalDaemonResponse::AgentFocused { .. } => "AgentFocused",
        LocalDaemonResponse::AgentFocusCycled { .. } => "AgentFocusCycled",
        LocalDaemonResponse::AgentAliased { .. } => "AgentAliased",
        LocalDaemonResponse::AgentConfigUpdated { .. } => "AgentConfigUpdated",
        LocalDaemonResponse::AgentProfileUpdated { .. } => "AgentProfileUpdated",
        LocalDaemonResponse::TerminalResized { .. } => "TerminalResized",
        LocalDaemonResponse::RuntimeNotices { .. } => "RuntimeNotices",
        _ => "Other",
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
                | LocalDaemonRequest::SendTerminalInput(_)
                | LocalDaemonRequest::PollRuntimeNotices(_)
                | LocalDaemonRequest::RespondToInteraction(_)
                | LocalDaemonRequest::UpdateSessionConfig(_)
                | LocalDaemonRequest::AliasAgent(_)
                | LocalDaemonRequest::UpdateAgentConfig(_)
                | LocalDaemonRequest::UpdateAgentProfile(_)
                | LocalDaemonRequest::UpdateAgentSubstitutes(_)
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
    use super::projection_policy::{
        projected_config_update_absence_response, session_response_projection_action,
        SessionProjectionAction,
    };
    use crate::agent::CreateAgentRequest;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::local::{
        AliasSessionRequest, AttachToSessionRequest, CycleAgentFocusRequest, DeleteSessionRequest,
        DestroyAgentRequest, DetachFromSessionRequest, EndSessionRequest, FocusAgentRequest,
        LocalDaemonRequest, LocalDaemonResponse, PollRuntimeNoticesRequest, ResizeTerminalRequest,
        UpdateAgentConfigRequest, UpdateSessionConfigRequest,
    };
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest};
    use crate::runtime::command::KernelCommand;
    use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
    use crate::runtime::session_actor::{FocusedAgentProjection, SessionRuntime};
    use crate::runtime::state::KernelRuntimeState;
    use crate::session::{
        CreateSessionRequest, PromptSubmissionOutcome, SessionAgentDefaults, DEFAULT_LOCAL_USER_ID,
    };
    use crate::terminal::TerminalOutputKind;
    use crate::{DaemonApp, DaemonConfig, DaemonError};
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    fn launch_dev_stub_provider(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
        model: &str,
    ) -> crate::provider::RuntimeProviderRun {
        launch_provider_for_adapter(app, session_id, agent_id, "dev-stub", model)
    }

    fn launch_provider_for_adapter(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
        adapter_key: &str,
        model: &str,
    ) -> crate::provider::RuntimeProviderRun {
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session_id,
                    adapter_key,
                    "claude-code",
                    "default",
                    model,
                )
                .with_agent_id(agent_id),
            )
            .expect("provider launch should succeed");
        app.update_provider_run_projection(provider_run.clone());
        provider_run
    }

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let app_locked = app.lock().await;
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.slices(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.history_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
            app_locked.session_history_projection_store(),
            app_locked.prompt_state_owner(),
            app_locked.active_turn_store(),
            app_locked.prompt_activity_store(),
            app_locked.prompt_workspace_claim_store(),
            app_locked.structured_output_record_store(),
            app_locked.terminal_stream_store(),
            app_locked.workspace_coordinator(),
        )
    }

    #[test]
    fn session_response_projection_action_uses_response_session_and_removes_deleted_sessions() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(session.id())
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
            owned_runtime_state(&app).await,
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
    async fn create_session_uses_owned_runtime_state_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let session_projection = SessionStateProjectionStore::default();
        let agent_runtime_projection = AgentRuntimeProjectionStore::default();
        let terminal_stream = {
            let app_locked = app.lock().await;
            app_locked.terminal_stream_store()
        };
        let durable_state_store = {
            let app_locked = app.lock().await;
            app_locked.durable_state_store()
        };
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            terminal_stream,
        );

        let request = LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
            "owned-workspace",
            "owned-worktree",
        ));
        let command =
            KernelCommand::from_local_request("owned-session-create", None, None, &request);
        let locked_app = app.lock().await;
        let response = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(command, request),
        )
        .await
        .expect("owned create-session path should not wait for the app lock")
        .expect("session creation should succeed");

        let LocalDaemonResponse::SessionCreated { session, agent } = response else {
            panic!("unexpected response");
        };
        assert_eq!(session.workspace_id(), "owned-workspace");
        assert_eq!(agent.session_id(), session.id());
        assert_eq!(session.focused_agent_id(), Some(agent.id()));
        drop(locked_app);
        let durable_events = durable_state_store
            .load_events_after(0)
            .expect("durable state events should load");
        assert!(
            durable_events.iter().any(|event| {
                event.kind == "session.created"
                    && event.subject_id.as_deref() == Some(session.id())
                    && event
                        .payload
                        .get("default_agent")
                        .and_then(|agent| agent.get("id"))
                        .and_then(|id| id.as_str())
                        == Some(agent.id())
            }),
            "owned runtime create-session path should persist the session.created durable event"
        );
        assert!(session_projection.get(session.id()).is_some());
        assert!(
            agent_runtime_projection
                .get(agent.id())
                .filter(|projection| projection.session_id == session.id())
                .is_some(),
            "session runtime should publish agent-runtime projection from the owned create response"
        );
    }

    #[tokio::test]
    async fn update_session_config_uses_owned_runtime_state_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (session_id, attachment_id, terminal_stream) = {
            let mut app_locked = app.lock().await;
            let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let attachment = crate::app::KernelSessionService::new(&mut app_locked)
                .attach(AttachRequest::new(
                    session.id(),
                    "config-client",
                    ClientCapabilityLevel::FullTerminal,
                ))
                .expect("attachment should attach");
            (
                session.id().to_string(),
                attachment.id().to_string(),
                app_locked.terminal_stream_store(),
            )
        };
        let session_projection = SessionStateProjectionStore::default();
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            session_projection.clone(),
            AgentRuntimeProjectionStore::default(),
            terminal_stream,
        );

        let request = LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
            session_id: session_id.clone(),
            attachment_id,
            values: [("mode".to_string(), "owned".to_string())].into(),
            requires_idle: false,
        });
        let command =
            KernelCommand::from_local_request("owned-session-config", None, None, &request);
        let _locked_app = app.lock().await;
        let response = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(command, request),
        )
        .await
        .expect("owned config-update path should not wait for the app lock")
        .expect("config update should succeed");

        let LocalDaemonResponse::SessionConfigUpdated { config, session } = response else {
            panic!("unexpected response");
        };
        assert_eq!(session.id(), session_id);
        assert_eq!(
            config.values().get("mode").map(String::as_str),
            Some("owned")
        );
        assert_eq!(
            session
                .config_state()
                .values()
                .get("mode")
                .map(String::as_str),
            Some("owned")
        );
        assert!(session_projection.get(&session_id).is_some());
    }

    #[tokio::test]
    async fn alias_session_uses_owned_runtime_state_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (session_id, terminal_stream) = {
            let mut app_locked = app.lock().await;
            let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            (session.id().to_string(), app_locked.terminal_stream_store())
        };
        let session_projection = SessionStateProjectionStore::default();
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            session_projection.clone(),
            AgentRuntimeProjectionStore::default(),
            terminal_stream,
        );

        let request = LocalDaemonRequest::AliasSession(AliasSessionRequest {
            session_id: session_id.clone(),
            alias: "owned-alias".to_string(),
        });
        let command =
            KernelCommand::from_local_request("owned-session-alias", None, None, &request);
        let _locked_app = app.lock().await;
        let response = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(command, request),
        )
        .await
        .expect("owned alias path should not wait for the app lock")
        .expect("alias update should succeed");

        let LocalDaemonResponse::SessionAliased { session } = response else {
            panic!("unexpected response");
        };
        assert_eq!(session.id(), session_id);
        assert_eq!(session.alias(), Some("owned-alias"));
        assert_eq!(
            session_projection
                .get(&session_id)
                .and_then(|projected| projected.alias().map(str::to_string)),
            Some("owned-alias".to_string())
        );
    }

    #[tokio::test]
    async fn local_spawn_agent_uses_owned_runtime_state_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (session_id, terminal_stream) = {
            let mut app_locked = app.lock().await;
            let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            (session.id().to_string(), app_locked.terminal_stream_store())
        };
        let session_projection = SessionStateProjectionStore::default();
        let agent_runtime_projection = AgentRuntimeProjectionStore::default();
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            terminal_stream,
        );

        let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("owned-agent".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("default".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: Some("worktree".to_string()),
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
        });
        let command =
            KernelCommand::from_local_request("owned-local-agent-spawn", None, None, &request);
        let _locked_app = app.lock().await;
        let response = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(command, request),
        )
        .await
        .expect("owned local agent spawn should not wait for the app lock")
        .expect("agent spawn should succeed");

        let LocalDaemonResponse::AgentSpawned { agent } = response else {
            panic!("unexpected response");
        };
        assert_eq!(agent.session_id(), session_id);
        assert_eq!(agent.alias(), Some("owned-agent"));
        let projected = session_projection
            .get(&session_id)
            .expect("spawn should refresh session projection");
        assert_eq!(projected.focused_agent_id(), Some(agent.id()));
        assert!(
            agent_runtime_projection
                .get(agent.id())
                .filter(|projection| projection.session_id == session_id)
                .is_some(),
            "spawn should refresh agent-runtime projection"
        );
    }

    #[tokio::test]
    async fn local_spawn_agent_inherits_session_agent_defaults_when_omitted() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (session_id, terminal_stream) = {
            let mut app_locked = app.lock().await;
            let defaults = SessionAgentDefaults::new("opencode")
                .with_model("moonshotai/kimi-k2")
                .with_effort("high")
                .with_execution_mode(AgentExecutionMode::Plan)
                .with_permission_level(AgentPermissionLevel::Required);
            let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(
                    CreateSessionRequest::new("workspace", "worktree")
                        .with_agent_defaults(defaults),
                )
                .expect("session should be created");
            (session.id().to_string(), app_locked.terminal_stream_store())
        };
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            SessionStateProjectionStore::default(),
            AgentRuntimeProjectionStore::default(),
            terminal_stream,
        );

        let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("inherited-agent".to_string()),
            provider: None,
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: Some("worktree".to_string()),
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
        });
        let command =
            KernelCommand::from_local_request("owned-default-agent-spawn", None, None, &request);
        let response = runtime
            .dispatch_session_command(command, request)
            .await
            .expect("agent spawn should succeed");

        let LocalDaemonResponse::AgentSpawned { agent } = response else {
            panic!("unexpected response");
        };
        assert_eq!(agent.session_id(), session_id);
        assert_eq!(agent.alias(), Some("inherited-agent"));
        assert_eq!(agent.provider(), "opencode");
        assert_eq!(agent.model(), Some("moonshotai/kimi-k2"));
        assert_eq!(agent.effort(), Some("high"));
        assert_eq!(
            agent.execution_mode_override(),
            Some(AgentExecutionMode::Plan)
        );
        assert_eq!(
            agent.permission_level_override(),
            Some(AgentPermissionLevel::Required)
        );
    }

    #[tokio::test]
    async fn update_agent_config_invalidates_only_that_agents_idle_provider_run() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (
            session_id,
            first_agent_id,
            second_agent_id,
            first_run_id,
            second_run_id,
            terminal_stream,
        ) = {
            let mut app_locked = app.lock().await;
            let (session, first_agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let second_agent = crate::app::KernelSessionService::new(&mut app_locked)
                .spawn_agent(
                    CreateAgentRequest::new(session.id(), "dev-stub")
                        .with_alias("agent-b")
                        .with_worktree("worktree"),
                )
                .expect("second agent should be created");
            let first_run =
                launch_dev_stub_provider(&mut app_locked, session.id(), first_agent.id(), "sonnet");
            let second_run =
                launch_dev_stub_provider(&mut app_locked, session.id(), second_agent.id(), "opus");
            (
                session.id().to_string(),
                first_agent.id().to_string(),
                second_agent.id().to_string(),
                first_run.id().to_string(),
                second_run.id().to_string(),
                app_locked.terminal_stream_store(),
            )
        };
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            SessionStateProjectionStore::default(),
            AgentRuntimeProjectionStore::default(),
            terminal_stream,
        );

        let request = LocalDaemonRequest::UpdateAgentConfig(UpdateAgentConfigRequest {
            session_id: session_id.clone(),
            agent_id: first_agent_id.clone(),
            execution_mode: Some(AgentExecutionMode::Plan),
            clear_execution_mode: false,
            permission_level: None,
            clear_permission_level: false,
            workspace_id: None,
            clear_workspace_id: false,
            worktree_id: None,
            clear_worktree_id: false,
        });
        let command =
            KernelCommand::from_local_request("owned-agent-config-update", None, None, &request);
        let response = runtime
            .dispatch_session_command(command, request)
            .await
            .expect("agent config update should succeed");

        let LocalDaemonResponse::AgentConfigUpdated { agent, session } = response else {
            panic!("unexpected response");
        };
        assert_eq!(agent.id(), first_agent_id);
        assert_eq!(
            agent.execution_mode_override(),
            Some(AgentExecutionMode::Plan)
        );
        let second_agent = session
            .agents()
            .iter()
            .find(|agent| agent.id() == second_agent_id)
            .expect("second agent should remain in session");
        assert_eq!(second_agent.execution_mode_override(), None);

        let app_locked = app.lock().await;
        assert_eq!(
            app_locked
                .providers()
                .get_run(&first_run_id)
                .expect("first run should still be recorded")
                .state(),
            crate::provider::ProviderRunState::Ended
        );
        assert_eq!(
            app_locked
                .providers()
                .get_run(&second_run_id)
                .expect("second run should still be recorded")
                .state(),
            crate::provider::ProviderRunState::Running
        );
    }

    #[tokio::test]
    async fn update_agent_config_keeps_turn_scoped_provider_run_alive() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (session_id, agent_id, provider_run_id, terminal_stream) = {
            let mut app_locked = app.lock().await;
            let (session, agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(
                    CreateSessionRequest::new("workspace", "worktree")
                        .with_agent_defaults(SessionAgentDefaults::new("managed-dev-stub")),
                )
                .expect("session should be created");
            let run = launch_provider_for_adapter(
                &mut app_locked,
                session.id(),
                agent.id(),
                "managed-dev-stub",
                "sonnet",
            );
            (
                session.id().to_string(),
                agent.id().to_string(),
                run.id().to_string(),
                app_locked.terminal_stream_store(),
            )
        };
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            SessionStateProjectionStore::default(),
            AgentRuntimeProjectionStore::default(),
            terminal_stream,
        );

        let request = LocalDaemonRequest::UpdateAgentConfig(UpdateAgentConfigRequest {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
            execution_mode: Some(AgentExecutionMode::Plan),
            clear_execution_mode: false,
            permission_level: Some(AgentPermissionLevel::Required),
            clear_permission_level: false,
            workspace_id: None,
            clear_workspace_id: false,
            worktree_id: None,
            clear_worktree_id: false,
        });
        let command = KernelCommand::from_local_request(
            "owned-turn-scoped-config-update",
            None,
            None,
            &request,
        );
        let response = runtime
            .dispatch_session_command(command, request)
            .await
            .expect("agent config update should succeed");

        let LocalDaemonResponse::AgentConfigUpdated { agent, .. } = response else {
            panic!("unexpected response");
        };
        assert_eq!(agent.id(), agent_id);
        assert_eq!(
            agent.execution_mode_override(),
            Some(AgentExecutionMode::Plan)
        );

        let app_locked = app.lock().await;
        let provider_run = app_locked
            .providers()
            .get_run(&provider_run_id)
            .expect("provider run should still be recorded");
        assert_eq!(
            provider_run.state(),
            crate::provider::ProviderRunState::Running
        );
        assert_eq!(provider_run.execution_mode(), AgentExecutionMode::Plan);
        assert_eq!(
            provider_run.permission_level(),
            AgentPermissionLevel::Required
        );
    }

    #[tokio::test]
    async fn local_destroy_agent_uses_owned_runtime_state_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (session_id, agent_id, provider_run_id, terminal_stream) = {
            let mut app_locked = app.lock().await;
            let (session, default_agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let extra_agent = crate::app::KernelSessionService::new(&mut app_locked)
                .spawn_agent(
                    CreateAgentRequest::new(session.id(), "dev-stub")
                        .with_alias("destroy-me")
                        .with_worktree("worktree"),
                )
                .expect("extra agent should be created");
            let provider_run =
                launch_dev_stub_provider(&mut app_locked, session.id(), extra_agent.id(), "opus");
            assert_ne!(default_agent.id(), extra_agent.id());
            (
                session.id().to_string(),
                extra_agent.id().to_string(),
                provider_run.id().to_string(),
                app_locked.terminal_stream_store(),
            )
        };
        let session_projection = SessionStateProjectionStore::default();
        let agent_runtime_projection = AgentRuntimeProjectionStore::default();
        let state = owned_runtime_state(&app).await;
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            state.clone(),
            1,
            FocusedAgentProjection::default(),
            session_projection.clone(),
            agent_runtime_projection.clone(),
            terminal_stream,
        );
        let session = state
            .session_snapshot(&session_id)
            .await
            .expect("session snapshot should be available");
        session_projection.update(session.clone());
        agent_runtime_projection.update_session(&session);
        assert!(
            agent_runtime_projection.get(&agent_id).is_some(),
            "agent projection should be warmed before destroy"
        );

        let request = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
        });
        let command =
            KernelCommand::from_local_request("owned-local-agent-destroy", None, None, &request);
        let _locked_app = app.lock().await;
        let response = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(command, request),
        )
        .await
        .expect("owned local agent destroy should not wait for the app lock")
        .expect("agent destroy should succeed");

        let LocalDaemonResponse::AgentDestroyed { agent } = response else {
            panic!("unexpected response");
        };
        assert_eq!(agent.id(), agent_id);
        let projected = session_projection
            .get(&session_id)
            .expect("destroy should refresh session projection");
        assert!(
            projected
                .agents()
                .iter()
                .all(|agent| agent.id() != agent_id),
            "destroyed agent should be removed from session projection"
        );
        assert!(
            agent_runtime_projection.get(&agent_id).is_none(),
            "destroyed agent should be removed from agent-runtime projection"
        );
        let provider_run = _locked_app
            .providers()
            .get_run(&provider_run_id)
            .expect("destroyed agent provider run should still be addressable");
        assert_eq!(
            provider_run.state(),
            crate::provider::ProviderRunState::Ended,
            "destroying an agent should end its provider run"
        );
    }

    #[tokio::test]
    async fn attach_and_detach_use_owned_runtime_state_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (session_id, terminal_stream) = {
            let mut app_locked = app.lock().await;
            let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            (session.id().to_string(), app_locked.terminal_stream_store())
        };
        let session_projection = SessionStateProjectionStore::default();
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            session_projection.clone(),
            AgentRuntimeProjectionStore::default(),
            terminal_stream,
        );

        let attach_request = LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "owned-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        });
        let attach_command =
            KernelCommand::from_local_request("owned-attach", None, None, &attach_request);
        let _locked_app = app.lock().await;
        let attach_response = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(attach_command, attach_request),
        )
        .await
        .expect("owned attach should not wait for the app lock")
        .expect("attach should succeed");
        let LocalDaemonResponse::SessionAttached { attachment } = attach_response else {
            panic!("unexpected attach response");
        };
        assert_eq!(attachment.session_id(), session_id);
        assert!(
            session_projection
                .get(&session_id)
                .is_some_and(|session| session.has_attachment(attachment.id())),
            "attach should refresh session projection"
        );

        let detach_request = LocalDaemonRequest::DetachFromSession(DetachFromSessionRequest {
            attachment_id: attachment.id().to_string(),
        });
        let detach_command =
            KernelCommand::from_local_request("owned-detach", None, None, &detach_request);
        let detach_response = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(detach_command, detach_request),
        )
        .await
        .expect("owned detach should not wait for the app lock")
        .expect("detach should succeed");
        assert!(matches!(
            detach_response,
            LocalDaemonResponse::SessionDetached { .. }
        ));
        assert!(
            session_projection
                .get(&session_id)
                .is_some_and(|session| !session.has_attachment(attachment.id())),
            "detach should refresh session projection"
        );
    }

    #[tokio::test]
    async fn focus_and_cycle_use_owned_runtime_state_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (session_id, default_agent_id, extra_agent_id, terminal_stream) = {
            let mut app_locked = app.lock().await;
            let (session, default_agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let extra_agent = crate::app::KernelSessionService::new(&mut app_locked)
                .spawn_agent(
                    CreateAgentRequest::new(session.id(), "dev-stub")
                        .with_alias("cycle-me")
                        .with_worktree("worktree"),
                )
                .expect("extra agent should be created");
            (
                session.id().to_string(),
                default_agent.id().to_string(),
                extra_agent.id().to_string(),
                app_locked.terminal_stream_store(),
            )
        };
        let session_projection = SessionStateProjectionStore::default();
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            session_projection.clone(),
            AgentRuntimeProjectionStore::default(),
            terminal_stream,
        );

        let focus_request = LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session_id.clone(),
            agent_id: default_agent_id.clone(),
        });
        let focus_command =
            KernelCommand::from_local_request("owned-focus", None, None, &focus_request);
        let _locked_app = app.lock().await;
        let focus_response = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(focus_command, focus_request),
        )
        .await
        .expect("owned focus should not wait for the app lock")
        .expect("focus should succeed");
        assert!(matches!(
            focus_response,
            LocalDaemonResponse::AgentFocused { .. }
        ));
        assert_eq!(
            session_projection
                .get(&session_id)
                .and_then(|session| session.focused_agent_id().map(str::to_string)),
            Some(default_agent_id)
        );

        let cycle_request = LocalDaemonRequest::CycleAgentFocus(CycleAgentFocusRequest {
            session_id: session_id.clone(),
        });
        let cycle_command =
            KernelCommand::from_local_request("owned-cycle", None, None, &cycle_request);
        let cycle_response = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(cycle_command, cycle_request),
        )
        .await
        .expect("owned focus cycle should not wait for the app lock")
        .expect("cycle should succeed");
        let LocalDaemonResponse::AgentFocusCycled { agent: Some(agent) } = cycle_response else {
            panic!("unexpected cycle response");
        };
        assert_eq!(agent.id(), extra_agent_id);
    }

    #[tokio::test]
    async fn owned_multi_agent_reattach_resumes_focused_run_before_focus_cycle() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (
            session_id,
            attachment_id,
            default_agent_id,
            extra_agent_id,
            default_run_id,
            extra_run_id,
        ) = {
            let mut app_locked = app.lock().await;
            let (session, default_agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let extra_agent = crate::app::KernelSessionService::new(&mut app_locked)
                .spawn_agent(
                    CreateAgentRequest::new(session.id(), "dev-stub")
                        .with_alias("cycle-me")
                        .with_worktree("worktree"),
                )
                .expect("extra agent should be created");
            let attachment = crate::app::KernelSessionService::new(&mut app_locked)
                .attach(AttachRequest::new(
                    session.id(),
                    "client-a",
                    ClientCapabilityLevel::FullTerminal,
                ))
                .expect("attachment should attach");
            let default_run = launch_dev_stub_provider(
                &mut app_locked,
                session.id(),
                default_agent.id(),
                "default",
            );
            crate::app::KernelSessionService::new(&mut app_locked)
                .focus_agent(session.id(), extra_agent.id())
                .expect("extra agent should focus");
            let extra_run =
                launch_dev_stub_provider(&mut app_locked, session.id(), extra_agent.id(), "extra");
            crate::app::KernelSessionService::new(&mut app_locked)
                .focus_agent(session.id(), default_agent.id())
                .expect("default agent should refocus");
            (
                session.id().to_string(),
                attachment.id().to_string(),
                default_agent.id().to_string(),
                extra_agent.id().to_string(),
                default_run.id().to_string(),
                extra_run.id().to_string(),
            )
        };
        let state = owned_runtime_state(&app).await;

        state
            .detach(&attachment_id)
            .await
            .expect("last attachment should detach cleanly");
        {
            let app_locked = app.lock().await;
            assert_eq!(
                app_locked
                    .providers()
                    .get_run(&default_run_id)
                    .expect("default run should remain")
                    .state(),
                crate::provider::ProviderRunState::Parked
            );
            assert_eq!(
                app_locked
                    .sessions()
                    .get_session(&session_id)
                    .expect("session should remain")
                    .active_provider_run_id(),
                None
            );
        }

        state
            .attach(AttachRequest::new(
                &session_id,
                "client-b",
                ClientCapabilityLevel::FullTerminal,
            ))
            .await
            .expect("reattach should resume the focused provider run");
        {
            let app_locked = app.lock().await;
            assert_eq!(
                app_locked
                    .sessions()
                    .get_session(&session_id)
                    .expect("session should remain")
                    .active_provider_run_id(),
                Some(default_run_id.as_str())
            );
            assert_eq!(
                app_locked
                    .providers()
                    .get_run(&default_run_id)
                    .expect("default run should remain")
                    .state(),
                crate::provider::ProviderRunState::Running
            );
        }

        let cycled = state
            .cycle_agent_focus(&session_id, DEFAULT_LOCAL_USER_ID)
            .await
            .expect("cycling focus after reattach should not park an already parked run")
            .expect("another agent should be focused");
        assert_eq!(cycled.id(), extra_agent_id);
        let app_locked = app.lock().await;
        assert_eq!(
            app_locked
                .sessions()
                .get_session(&session_id)
                .expect("session should remain")
                .active_provider_run_id(),
            Some(extra_run_id.as_str())
        );
        assert_eq!(
            app_locked
                .providers()
                .get_run(&extra_run_id)
                .expect("extra run should remain")
                .state(),
            crate::provider::ProviderRunState::Running
        );
        assert_ne!(default_agent_id, extra_agent_id);
    }

    #[tokio::test]
    async fn end_and_delete_use_owned_runtime_state_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (end_session_id, delete_session_id, terminal_stream) = {
            let mut app_locked = app.lock().await;
            let (end_session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("end session should be created");
            let (delete_session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(
                    CreateSessionRequest::new("workspace", "worktree").with_alias("delete-owned"),
                )
                .expect("delete session should be created");
            (
                end_session.id().to_string(),
                delete_session.id().to_string(),
                app_locked.terminal_stream_store(),
            )
        };
        let session_projection = SessionStateProjectionStore::default();
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            session_projection.clone(),
            AgentRuntimeProjectionStore::default(),
            terminal_stream,
        );

        let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: end_session_id.clone(),
        });
        let end_command = KernelCommand::from_local_request("owned-end", None, None, &end_request);
        let _locked_app = app.lock().await;
        let end_response = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(end_command, end_request),
        )
        .await
        .expect("owned end should not wait for the app lock")
        .expect("end should succeed");
        assert!(matches!(
            end_response,
            LocalDaemonResponse::SessionEnded { .. }
        ));
        assert!(
            session_projection.get(&end_session_id).is_some(),
            "ended session should remain projected"
        );

        let delete_request = LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: "delete-owned".to_string(),
            workspace_id: Some("workspace".to_string()),
        });
        let delete_command =
            KernelCommand::from_local_request("owned-delete", None, None, &delete_request);
        let delete_response = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(delete_command, delete_request),
        )
        .await
        .expect("owned delete should not wait for the app lock")
        .expect("delete should succeed");
        assert!(matches!(
            delete_response,
            LocalDaemonResponse::SessionDeleted { .. }
        ));
        assert!(
            session_projection.get(&delete_session_id).is_none(),
            "deleted session should be removed from projection"
        );
    }

    #[tokio::test]
    async fn resize_terminal_validates_owned_session_state_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let (session_id, terminal_stream) = {
            let mut app_locked = app.lock().await;
            let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            (session.id().to_string(), app_locked.terminal_stream_store())
        };
        let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            SessionStateProjectionStore::default(),
            AgentRuntimeProjectionStore::default(),
            terminal_stream,
        );

        let request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
            session_id: session_id.clone(),
            cols: 120,
            rows: 40,
        });
        let command =
            KernelCommand::from_local_request("owned-resize-validation", None, None, &request);
        let _locked_app = app.lock().await;
        let error = timeout(
            Duration::from_millis(100),
            runtime.dispatch_session_command(command, request),
        )
        .await
        .expect("owned resize validation should not wait for the app lock")
        .expect_err("resize without an active provider run should fail");
        assert!(matches!(
            error,
            DaemonError::NoActiveProviderRun { session_id: id } if id == session_id
        ));
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
            owned_runtime_state(&app).await,
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
            let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created");
            let attachment = crate::app::KernelSessionService::new(&mut app)
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
            owned_runtime_state(&app).await,
            1,
            FocusedAgentProjection::default(),
            SessionStateProjectionStore::default(),
            AgentRuntimeProjectionStore::default(),
            terminal_stream.clone(),
        );
        let request = LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session_id.clone(),
        });
        let command = crate::runtime::command::KernelCommand::from_local_request(
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
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attach should succeed");
        let response = LocalDaemonResponse::SessionAttached { attachment };

        assert!(matches!(
            response,
            LocalDaemonResponse::SessionAttached { .. }
        ));
    }

    #[test]
    fn focus_does_not_disturb_multi_agent_provider_liveness() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let default_run =
            launch_dev_stub_provider(&mut app, session.id(), default_agent.id(), "sonnet");

        let second_agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "claude-code").with_alias("agent-b"))
            .expect("second agent should spawn");

        let _second_run =
            launch_dev_stub_provider(&mut app, session.id(), second_agent.id(), "opus");

        crate::app::KernelSessionService::new(&mut app)
            .focus_agent(session.id(), default_agent.id())
            .expect("focus should succeed");

        let started = LocalDaemonResponse::PromptSubmitted {
            outcome: crate::app::KernelAgentService::new(&mut app)
                .submit_prompt(session.id(), attachment.id(), None, "hello", Vec::new())
                .expect("prompt should start"),
            session: crate::app::KernelSessionReadService::new(&app)
                .session_snapshot(session.id())
                .expect("session snapshot should load"),
            agent_activity: std::collections::BTreeMap::new(),
        };

        match started {
            LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), default_agent.id());
                }
                _ => panic!("expected prompt to start immediately"),
            },
            _ => panic!("unexpected local response"),
        }

        let agent = crate::app::KernelSessionService::new(&mut app)
            .focus_agent(session.id(), second_agent.id())
            .expect("focus should succeed");
        let response = LocalDaemonResponse::AgentFocused { agent };

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
