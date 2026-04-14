use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::app::{DaemonApp, KernelPreparedPromptSubmission};
use crate::error::DaemonError;
use crate::kernel::agent_prompt_service::AgentPromptCommandService;
use crate::kernel::projection::{
    ActorQueueSnapshot, AgentRuntimeProjection, AgentRuntimeProjectionStore,
    SessionStateProjectionStore,
};
use crate::kernel::prompt_state::PromptStateOwner;
use crate::kernel::session_actor::FocusedAgentProjection;
use crate::local::LocalDaemonResponse;
use crate::provider::ProviderRunOperationLanes;
use crate::session::{PromptCompletion, PromptIdAllocator, PromptQueueItem, PromptStatus};

const AGENT_COMMAND_QUEUE_LIMIT: usize = 128;

#[derive(Debug)]
enum AgentCommand {
    SubmitPrompt {
        request: crate::local::SubmitPromptRequest,
    },
    CompletePrompt {
        request: crate::local::CompletePromptRequest,
        target_agent_id: String,
        next_queued_prompt: Option<PromptQueueItem>,
    },
    CancelActivePrompt {
        request: crate::local::CancelActivePromptRequest,
        target_agent_id: String,
    },
}

#[derive(Debug)]
struct AgentCommandEnvelope {
    command_id: String,
    command_type: String,
    command: AgentCommand,
    result_tx: oneshot::Sender<Result<LocalDaemonResponse, DaemonError>>,
}

#[derive(Clone)]
struct AgentRuntimeCommandExecutor {
    prompt_commands: AgentPromptCommandService,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    prompt_id_allocator: PromptIdAllocator,
}

impl AgentRuntimeCommandExecutor {
    fn new(
        prompt_commands: AgentPromptCommandService,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        prompt_id_allocator: PromptIdAllocator,
    ) -> Self {
        Self {
            prompt_commands,
            session_projection,
            agent_runtime_projection,
            prompt_id_allocator,
        }
    }

    async fn execute(&self, command: AgentCommand) -> Result<LocalDaemonResponse, DaemonError> {
        match command {
            AgentCommand::SubmitPrompt { request } => self.submit_prompt(request).await,
            AgentCommand::CancelActivePrompt {
                request,
                target_agent_id,
            } => self.cancel_active_prompt(request, target_agent_id).await,
            AgentCommand::CompletePrompt {
                request,
                target_agent_id,
                next_queued_prompt,
            } => {
                self.complete_prompt(request, target_agent_id, next_queued_prompt)
                    .await
            }
        }
    }

    async fn submit_prompt(
        &self,
        request: crate::local::SubmitPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let target_agent_id =
            request
                .target_agent_id
                .clone()
                .ok_or_else(|| DaemonError::AgentNotFound {
                    agent_id: "no target agent".to_string(),
                })?;
        let prompt = PromptQueueItem::new(
            self.prompt_id_allocator.next_prompt_id(),
            &request.attachment_id,
            &target_agent_id,
            &request.prompt,
            PromptStatus::Queued,
        )
        .with_attachments(request.attachments);
        let prepared = self
            .prompt_commands
            .submit_prepared_prompt(KernelPreparedPromptSubmission {
                session_id: request.session_id.clone(),
                prompt,
                force_queue: false,
            })
            .await?;
        self.session_projection.update(prepared.session.clone());
        self.agent_runtime_projection
            .update_session(&prepared.session);

        if let Some(dispatch) = prepared.dispatch {
            self.prompt_commands.spawn_prompt_dispatch(dispatch);
        }
        if let Some(dispatch) = prepared.remote_dispatch {
            self.prompt_commands.spawn_remote_prompt_dispatch(dispatch);
        }

        Ok(LocalDaemonResponse::PromptSubmitted {
            outcome: prepared.outcome,
            session: prepared.session,
        })
    }

    async fn cancel_active_prompt(
        &self,
        request: crate::local::CancelActivePromptRequest,
        target_agent_id: String,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let prepared = self
            .prompt_commands
            .cancel_agent_prompt(
                &request.session_id,
                &target_agent_id,
                &request.attachment_id,
            )
            .await?;
        self.session_projection.update(prepared.session.clone());
        self.agent_runtime_projection
            .update_session(&prepared.session);

        if let Some(dispatch) = prepared.dispatch {
            self.prompt_commands.spawn_prompt_abort(dispatch);
        }

        Ok(LocalDaemonResponse::PromptCancelled {
            cancellation: prepared.cancellation,
        })
    }

    async fn complete_prompt(
        &self,
        request: crate::local::CompletePromptRequest,
        target_agent_id: String,
        next_queued_prompt: Option<PromptQueueItem>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let completion = self
            .prompt_commands
            .complete_agent_prompt(
                &request.session_id,
                &target_agent_id,
                next_queued_prompt.clone(),
            )
            .await?;
        let session = self
            .session_projection
            .get(&request.session_id)
            .ok_or_else(|| DaemonError::SessionNotFound {
                session_id: request.session_id.clone(),
            })?;
        self.session_projection.update(session.clone());
        debug_assert!(
            completion_started_next_is_compatible(next_queued_prompt.as_ref(), &completion),
            "agent runtime queue-front preview should match compatibility advancement"
        );
        self.agent_runtime_projection.update_session(&session);

        Ok(LocalDaemonResponse::PromptCompleted { completion })
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct AgentRuntime {
    store: AgentRuntimeStore,
    provider_runtime_lanes: ProviderRunOperationLanes,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    prompt_state_owner: PromptStateOwner,
    prompt_id_allocator: PromptIdAllocator,
    queue_limit: usize,
    lanes: Arc<Mutex<HashMap<String, mpsc::Sender<AgentCommandEnvelope>>>>,
}

impl AgentRuntime {
    pub(crate) fn new(
        app: Arc<Mutex<DaemonApp>>,
        provider_runtime_lanes: ProviderRunOperationLanes,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        prompt_state_owner: PromptStateOwner,
        prompt_id_allocator: PromptIdAllocator,
    ) -> Self {
        Self::with_store(
            AgentRuntimeStore::new(app),
            provider_runtime_lanes,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            prompt_state_owner,
            prompt_id_allocator,
        )
    }

    pub(crate) fn with_store(
        store: AgentRuntimeStore,
        provider_runtime_lanes: ProviderRunOperationLanes,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        prompt_state_owner: PromptStateOwner,
        prompt_id_allocator: PromptIdAllocator,
    ) -> Self {
        Self {
            store,
            provider_runtime_lanes,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            prompt_state_owner,
            prompt_id_allocator,
            queue_limit: AGENT_COMMAND_QUEUE_LIMIT,
            lanes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) async fn dispatch_prompt_submit(
        &self,
        command: &crate::kernel::command::KernelCommand,
        mut request: crate::local::SubmitPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let agent_id = self
            .resolve_submit_agent_id(&request.session_id, request.target_agent_id.as_deref())
            .await?;
        request.target_agent_id = Some(agent_id.clone());
        self.dispatch_to_agent(
            agent_id,
            command.command_id.clone(),
            command.command_type.clone(),
            AgentCommand::SubmitPrompt { request },
        )
        .await
    }

    pub(crate) async fn dispatch_prompt_cancel(
        &self,
        command: &crate::kernel::command::KernelCommand,
        request: crate::local::CancelActivePromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let agent_id = self
            .resolve_active_prompt_agent_id(&request.session_id)
            .await?;
        self.dispatch_to_agent(
            agent_id.clone(),
            command.command_id.clone(),
            command.command_type.clone(),
            AgentCommand::CancelActivePrompt {
                request,
                target_agent_id: agent_id.clone(),
            },
        )
        .await
    }

    pub(crate) async fn dispatch_prompt_complete(
        &self,
        command: &crate::kernel::command::KernelCommand,
        request: crate::local::CompletePromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let agent_id = self
            .resolve_active_prompt_agent_id(&request.session_id)
            .await?;
        let next_queued_prompt = self
            .session_projection
            .get(&request.session_id)
            .and_then(|session| {
                self.prompt_state_owner
                    .peek_next_queued_prompt(&session, &agent_id)
            })
            .or_else(|| {
                self.agent_runtime_projection
                    .next_queued_prompt(&request.session_id, &agent_id)
            });
        self.dispatch_to_agent(
            agent_id.clone(),
            command.command_id.clone(),
            command.command_type.clone(),
            AgentCommand::CompletePrompt {
                request,
                target_agent_id: agent_id.clone(),
                next_queued_prompt,
            },
        )
        .await
    }

    async fn resolve_active_prompt_agent_id(
        &self,
        session_id: &str,
    ) -> Result<String, DaemonError> {
        if let Some(agent_id) = self
            .resolve_projected_active_prompt_agent_id(session_id)
            .await
        {
            return Ok(agent_id);
        }
        if let Some(agent_id) = self
            .session_projection
            .get(session_id)
            .and_then(|session| self.prompt_state_owner.active_prompt_agent_id(&session))
        {
            return Ok(agent_id);
        }
        if self.session_projection.get(session_id).is_some()
            || !self
                .agent_runtime_projection
                .list_for_session(session_id)
                .is_empty()
        {
            return Err(DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            });
        }
        if self.session_projection.has_warmed_list() {
            return Err(DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            });
        }
        self.store
            .active_prompt_agent_id(session_id)
            .await?
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })
    }

    async fn resolve_projected_active_prompt_agent_id(&self, session_id: &str) -> Option<String> {
        if let Some(focused_agent_id) = self.focus_projection.focused_agent_id(session_id).await {
            if self
                .agent_runtime_projection
                .get(&focused_agent_id)
                .is_some_and(|projection| {
                    projection.session_id == session_id && projection.active_prompt.is_some()
                })
            {
                return Some(focused_agent_id);
            }
        }

        let session_focused_agent_id = self
            .session_projection
            .get(session_id)
            .and_then(|session| session.focused_agent_id().map(str::to_string));
        if let Some(focused_agent_id) = session_focused_agent_id.as_deref() {
            if self
                .agent_runtime_projection
                .get(focused_agent_id)
                .is_some_and(|projection| {
                    projection.session_id == session_id && projection.active_prompt.is_some()
                })
            {
                return Some(focused_agent_id.to_string());
            }
        }

        active_prompt_agent_id_from_projections(
            session_focused_agent_id.as_deref(),
            &self.agent_runtime_projection.list_for_session(session_id),
        )
    }

    async fn resolve_submit_agent_id(
        &self,
        session_id: &str,
        target_agent_id: Option<&str>,
    ) -> Result<String, DaemonError> {
        let session_projection = self.session_projection.get(session_id);
        if session_projection.is_none()
            && self.session_projection.has_warmed_list()
            && self
                .agent_runtime_projection
                .list_for_session(session_id)
                .is_empty()
        {
            return Err(DaemonError::SessionNotFound {
                session_id: session_id.to_string(),
            });
        }
        if let Some(agent_id) = target_agent_id {
            if let Some(session) = session_projection.as_ref() {
                if !session.agents().iter().any(|agent| agent.id() == agent_id) {
                    return Err(DaemonError::AgentNotInSession {
                        session_id: session_id.to_string(),
                        agent_id: agent_id.to_string(),
                    });
                }
            }
            return Ok(agent_id.to_string());
        }
        if let Some(agent_id) = self.focus_projection.focused_agent_id(session_id).await {
            return Ok(agent_id);
        }
        if let Some(agent_id) =
            session_projection.and_then(|session| session.focused_agent_id().map(str::to_string))
        {
            return Ok(agent_id);
        }
        if let Some(agent_id) =
            single_agent_projection_id(&self.agent_runtime_projection.list_for_session(session_id))
        {
            return Ok(agent_id);
        }
        self.store
            .focused_agent_id(session_id)
            .await?
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "no focused agent".to_string(),
            })
    }

    async fn dispatch_to_agent(
        &self,
        agent_id: String,
        command_id: String,
        command_type: String,
        command: AgentCommand,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let lane_key = agent_id;
        let lane = self.agent_lane(&lane_key).await;
        let (result_tx, result_rx) = oneshot::channel();
        lane.try_send(AgentCommandEnvelope {
            command_id,
            command_type,
            command,
            result_tx,
        })
        .map_err(|error| DaemonError::LocalTransport {
            operation: "enqueue agent kernel command",
            message: format!("agent command lane overloaded: {error}"),
        })?;
        result_rx
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "await agent kernel command",
                message: error.to_string(),
            })?
    }

    async fn agent_lane(&self, agent_id: &str) -> mpsc::Sender<AgentCommandEnvelope> {
        let mut lanes = self.lanes.lock().await;
        if let Some(lane) = lanes.get(agent_id) {
            return lane.clone();
        }
        let (tx, rx) = mpsc::channel(AGENT_COMMAND_QUEUE_LIMIT);
        lanes.insert(agent_id.to_string(), tx.clone());
        let prompt_commands = self
            .store
            .prompt_command_service(self.provider_runtime_lanes.clone());
        let executor = AgentRuntimeCommandExecutor::new(
            prompt_commands,
            self.session_projection.clone(),
            self.agent_runtime_projection.clone(),
            self.prompt_id_allocator.clone(),
        );
        tokio::spawn(run_agent_command_lane(executor, agent_id.to_string(), rx));
        tx
    }

    #[allow(dead_code)]
    pub(crate) async fn queue_snapshots(&self) -> Vec<ActorQueueSnapshot> {
        let lanes = self.lanes.lock().await;
        let mut snapshots = lanes
            .iter()
            .map(|(agent_id, sender)| {
                ActorQueueSnapshot::new(
                    agent_id.clone(),
                    self.queue_limit,
                    self.queue_limit.saturating_sub(sender.capacity()),
                )
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.lane_id.cmp(&right.lane_id));
        snapshots
    }

    pub(crate) async fn remove_agent_lane(&self, agent_id: &str) {
        self.lanes.lock().await.remove(agent_id);
    }

    pub(crate) async fn remove_agent_lanes<'a>(
        &self,
        agent_ids: impl IntoIterator<Item = &'a str>,
    ) {
        let mut lanes = self.lanes.lock().await;
        for agent_id in agent_ids {
            lanes.remove(agent_id);
        }
    }

    pub(crate) fn remove_session_state(&self, session_id: &str) {
        self.prompt_state_owner.remove_session(session_id);
        self.agent_runtime_projection.remove_session(session_id);
    }
}

#[derive(Clone)]
pub(crate) struct AgentRuntimeStore {
    app: Arc<Mutex<DaemonApp>>,
}

struct AgentRuntimeContext<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> AgentRuntimeContext<'a> {
    fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn active_prompt_agent_id(&mut self, session_id: &str) -> Result<Option<String>, DaemonError> {
        self.app.prompt_owner_active_prompt_agent_id(session_id)
    }

    fn focused_agent_id(&self, session_id: &str) -> Result<Option<String>, DaemonError> {
        Ok(self
            .app
            .sessions()
            .get_session(session_id)?
            .focused_agent_id()
            .map(str::to_string))
    }
}

impl AgentRuntimeStore {
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self { app }
    }

    async fn active_prompt_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        let mut app = self.app.lock().await;
        AgentRuntimeContext::new(&mut app).active_prompt_agent_id(session_id)
    }

    async fn focused_agent_id(&self, session_id: &str) -> Result<Option<String>, DaemonError> {
        let mut app = self.app.lock().await;
        AgentRuntimeContext::new(&mut app).focused_agent_id(session_id)
    }

    fn prompt_command_service(
        &self,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) -> AgentPromptCommandService {
        AgentPromptCommandService::new(Arc::clone(&self.app), provider_runtime_lanes)
    }
}

fn active_prompt_agent_id_from_projections(
    focused_agent_id: Option<&str>,
    projections: &[AgentRuntimeProjection],
) -> Option<String> {
    if let Some(focused_agent_id) = focused_agent_id {
        if projections.iter().any(|projection| {
            projection.agent_id == focused_agent_id && projection.active_prompt.is_some()
        }) {
            return Some(focused_agent_id.to_string());
        }
    }
    let mut active_agents = projections
        .iter()
        .filter(|projection| projection.active_prompt.is_some())
        .map(|projection| projection.agent_id.clone());
    let agent_id = active_agents.next()?;
    if active_agents.next().is_none() {
        Some(agent_id)
    } else {
        None
    }
}

fn single_agent_projection_id(projections: &[AgentRuntimeProjection]) -> Option<String> {
    let mut agent_ids = projections
        .iter()
        .map(|projection| projection.agent_id.clone())
        .collect::<Vec<_>>();
    agent_ids.sort();
    agent_ids.dedup();
    if agent_ids.len() == 1 {
        agent_ids.into_iter().next()
    } else {
        None
    }
}

async fn run_agent_command_lane(
    executor: AgentRuntimeCommandExecutor,
    agent_id: String,
    mut rx: mpsc::Receiver<AgentCommandEnvelope>,
) {
    while let Some(envelope) = rx.recv().await {
        crate::logging::info_with_fields(
            "daemon.kernel_agent_actor",
            "agent kernel command dispatched",
            serde_json::json!({
                "agent_id": agent_id,
                "command_id": envelope.command_id,
                "command_type": envelope.command_type,
            }),
        );
        let result = executor.execute(envelope.command).await;
        let _ = envelope.result_tx.send(result);
    }
}

fn completion_started_next_is_compatible(
    next_queued_prompt: Option<&PromptQueueItem>,
    completion: &PromptCompletion,
) -> bool {
    match (next_queued_prompt, completion.started_next.as_ref()) {
        (Some(expected), Some(started)) => expected.id() == started.id(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::kernel::agent_actor::AgentRuntime;
    use crate::kernel::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
    use crate::kernel::prompt_state::PromptStateOwner;
    use crate::kernel::session_actor::FocusedAgentProjection;
    use crate::local::LocalDaemonResponse;
    use crate::provider::{LaunchProviderRequest, ProviderRunOperationLanes};
    use crate::session::{
        CreateSessionRequest, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
    };
    use crate::DaemonError;
    use crate::{DaemonApp, DaemonConfig};

    fn launch_dev_stub_provider(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
        model: &str,
    ) {
        let provider_run = app
            .launch_provider(
                LaunchProviderRequest::new(session_id, "dev-stub", "claude-code", "default", model)
                    .with_agent_id(agent_id),
            )
            .expect("provider launch should succeed");
        app.update_provider_run_projection(provider_run);
    }

    #[tokio::test]
    async fn submit_agent_resolution_uses_single_agent_projection_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(session.id())
            .expect("session projection fixture should be available");
        let agent_runtime_projection = AgentRuntimeProjectionStore::default();
        agent_runtime_projection.update_session(&session_snapshot);
        let app = Arc::new(Mutex::new(app));
        let runtime = AgentRuntime::new(
            Arc::clone(&app),
            ProviderRunOperationLanes::default(),
            FocusedAgentProjection::default(),
            SessionStateProjectionStore::default(),
            agent_runtime_projection,
            PromptStateOwner::default(),
            crate::session::PromptIdAllocator::default(),
        );

        let _locked_app = app.lock().await;
        let resolved = timeout(
            Duration::from_millis(100),
            runtime.resolve_submit_agent_id(session.id(), None),
        )
        .await
        .expect("projection-backed resolution should not wait for the app lock")
        .expect("single projected agent should resolve");

        assert_eq!(resolved, agent.id());
    }

    #[tokio::test]
    async fn active_prompt_resolution_uses_warmed_projection_for_no_active_prompt() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(session.id())
            .expect("session projection fixture should be available");
        let session_projection = SessionStateProjectionStore::default();
        session_projection.update(session_snapshot);
        let app = Arc::new(Mutex::new(app));
        let runtime = AgentRuntime::new(
            Arc::clone(&app),
            ProviderRunOperationLanes::default(),
            FocusedAgentProjection::default(),
            session_projection,
            AgentRuntimeProjectionStore::default(),
            PromptStateOwner::default(),
            crate::session::PromptIdAllocator::default(),
        );

        let _locked_app = app.lock().await;
        let error = timeout(
            Duration::from_millis(100),
            runtime.resolve_active_prompt_agent_id(session.id()),
        )
        .await
        .expect("projection-backed no-active resolution should not wait for the app lock")
        .expect_err("session has no active prompt");

        match error {
            DaemonError::NoActivePrompt { session_id } => assert_eq!(session_id, session.id()),
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn active_prompt_resolution_uses_prompt_owner_without_app_lock_when_session_mirror_is_stale(
    ) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new(
                "workspace-owner-route",
                "worktree-owner-route",
            ))
            .expect("session should be created");
        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-owner-route",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "owner-backed routing",
            PromptStatus::Queued,
        );
        app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("prompt should submit through owner");
        app.sessions_mut()
            .complete_active_prompt_only(session.id(), agent.id())
            .expect("test should clear only the compatibility mirror");
        let session_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(session.id())
            .expect("session snapshot should still be available");
        assert!(
            session_snapshot
                .active_prompt_for_agent(agent.id())
                .is_none(),
            "compatibility session snapshot is intentionally stale"
        );

        let session_projection = SessionStateProjectionStore::default();
        session_projection.update(session_snapshot);
        let prompt_state_owner = app.prompt_state_owner();
        let app = Arc::new(Mutex::new(app));
        let runtime = AgentRuntime::new(
            Arc::clone(&app),
            ProviderRunOperationLanes::default(),
            FocusedAgentProjection::default(),
            session_projection,
            AgentRuntimeProjectionStore::default(),
            prompt_state_owner,
            crate::session::PromptIdAllocator::default(),
        );

        let _locked_app = app.lock().await;
        let resolved = timeout(
            Duration::from_millis(100),
            runtime.resolve_active_prompt_agent_id(session.id()),
        )
        .await
        .expect("owner-backed active prompt resolution should not wait for the app lock")
        .expect("prompt owner should still know the active agent");

        assert_eq!(resolved, agent.id());
    }

    #[tokio::test]
    async fn submit_agent_resolution_uses_warmed_list_for_missing_session_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let session_projection = SessionStateProjectionStore::default();
        session_projection.update_list(Vec::new());
        let runtime = AgentRuntime::new(
            Arc::clone(&app),
            ProviderRunOperationLanes::default(),
            FocusedAgentProjection::default(),
            session_projection,
            AgentRuntimeProjectionStore::default(),
            PromptStateOwner::default(),
            crate::session::PromptIdAllocator::default(),
        );

        let _locked_app = app.lock().await;
        let error = timeout(
            Duration::from_millis(100),
            runtime.resolve_submit_agent_id("missing-session", None),
        )
        .await
        .expect("warmed missing session should not wait for the app lock")
        .expect_err("missing session should fail");

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn submit_agent_resolution_uses_session_projection_for_invalid_target_without_app_lock() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(session.id())
            .expect("session projection fixture should be available");
        let session_projection = SessionStateProjectionStore::default();
        session_projection.update(session_snapshot);
        let app = Arc::new(Mutex::new(app));
        let runtime = AgentRuntime::new(
            Arc::clone(&app),
            ProviderRunOperationLanes::default(),
            FocusedAgentProjection::default(),
            session_projection,
            AgentRuntimeProjectionStore::default(),
            PromptStateOwner::default(),
            crate::session::PromptIdAllocator::default(),
        );

        let _locked_app = app.lock().await;
        let error = timeout(
            Duration::from_millis(100),
            runtime.resolve_submit_agent_id(session.id(), Some("missing-agent")),
        )
        .await
        .expect("projected invalid target resolution should not wait for the app lock")
        .expect_err("invalid target agent should fail");

        match error {
            DaemonError::AgentNotInSession {
                session_id,
                agent_id,
            } => {
                assert_eq!(session_id, session.id());
                assert_eq!(agent_id, "missing-agent");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn active_prompt_resolution_uses_warmed_list_for_missing_session_without_app_lock() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
        ));
        let session_projection = SessionStateProjectionStore::default();
        session_projection.update_list(Vec::new());
        let runtime = AgentRuntime::new(
            Arc::clone(&app),
            ProviderRunOperationLanes::default(),
            FocusedAgentProjection::default(),
            session_projection,
            AgentRuntimeProjectionStore::default(),
            PromptStateOwner::default(),
            crate::session::PromptIdAllocator::default(),
        );

        let _locked_app = app.lock().await;
        let error = timeout(
            Duration::from_millis(100),
            runtime.resolve_active_prompt_agent_id("missing-session"),
        )
        .await
        .expect("warmed missing active-prompt session should not wait for the app lock")
        .expect_err("missing session should fail");

        match error {
            DaemonError::SessionNotFound { session_id } => {
                assert_eq!(session_id, "missing-session");
            }
            error => panic!("unexpected error: {error}"),
        }
    }

    #[test]
    fn handles_prompt_submit_through_agent_request_surface() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attach should succeed");
        launch_dev_stub_provider(&mut app, session.id(), agent.id(), "sonnet");

        let outcome = crate::app::KernelAgentService::new(&mut app)
            .submit_prompt(
                session.id(),
                attachment.id(),
                Some(agent.id()),
                "hello",
                Vec::new(),
            )
            .expect("prompt submit should succeed");
        let response = LocalDaemonResponse::PromptSubmitted {
            outcome,
            session: crate::app::KernelSessionReadService::new(&app)
                .session_snapshot(session.id())
                .expect("session snapshot should load"),
        };

        match response {
            LocalDaemonResponse::PromptSubmitted {
                outcome,
                session: projected_session,
            } => {
                match outcome {
                    PromptSubmissionOutcome::Started { prompt } => {
                        assert_eq!(prompt.target_agent_id(), agent.id());
                    }
                    _ => panic!("expected prompt to start immediately"),
                }
                assert_eq!(projected_session.id(), session.id());
            }
            _ => panic!("unexpected local response"),
        }
    }

    #[test]
    fn handles_prompt_cancel_through_agent_request_surface() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "cli-1",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attach should succeed");
        launch_dev_stub_provider(&mut app, session.id(), agent.id(), "sonnet");
        crate::app::KernelAgentService::new(&mut app)
            .submit_prompt(
                session.id(),
                attachment.id(),
                Some(agent.id()),
                "hello",
                Vec::new(),
            )
            .expect("prompt submit should succeed");

        let response = LocalDaemonResponse::PromptCancelled {
            cancellation: crate::app::KernelAgentService::new(&mut app)
                .cancel_active_prompt(session.id(), attachment.id())
                .expect("prompt cancel should succeed"),
        };

        match response {
            LocalDaemonResponse::PromptCancelled { cancellation } => {
                assert_eq!(cancellation.prompt.target_agent_id(), agent.id());
                assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
            }
            _ => panic!("unexpected local response"),
        }
    }
}
