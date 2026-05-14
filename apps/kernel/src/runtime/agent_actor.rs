use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::app::KernelPreparedPromptSubmission;
use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::provider::ProviderRunOperationLanes;
use crate::runtime::agent_prompt_service::AgentPromptCommandService;
use crate::runtime::projection::{
    ActorQueueSnapshot, AgentRuntimeProjection, AgentRuntimeProjectionStore,
    SessionStateProjectionStore,
};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::runtime::session_actor::FocusedAgentProjection;
use crate::runtime::state::KernelRuntimeState;
use crate::session::{
    PromptCompletion, PromptIdAllocator, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
    DEFAULT_LOCAL_USER_ID,
};

const AGENT_COMMAND_QUEUE_LIMIT: usize = 128;

mod prompt_attachment_materialization;

use prompt_attachment_materialization::materialize_inline_prompt_attachments;

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
        .with_attachments(materialize_inline_prompt_attachments(
            &request.session_id,
            &target_agent_id,
            request.attachments,
        )?);
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

        if let (PromptSubmissionOutcome::Started { prompt }, Some(dispatch)) =
            (&prepared.outcome, prepared.dispatch.as_ref())
        {
            self.prompt_commands.start_active_turn(
                &dispatch.session_id,
                &dispatch.agent_id,
                prompt.id(),
                &dispatch.provider_run_id,
            );
        }
        let agent_activity = self
            .prompt_commands
            .agent_activity_for_session(&prepared.session);

        if let Some(dispatch) = prepared.dispatch {
            self.prompt_commands.spawn_prompt_dispatch(dispatch);
        }
        if let Some(dispatch) = prepared.remote_dispatch {
            self.prompt_commands.spawn_remote_prompt_dispatch(dispatch);
        }

        Ok(LocalDaemonResponse::PromptSubmitted {
            outcome: prepared.outcome,
            session: prepared.session,
            agent_activity,
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
            .prompt_commands
            .session_snapshot(&request.session_id)
            .await?;
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
        state: KernelRuntimeState,
        provider_runtime_lanes: ProviderRunOperationLanes,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
        prompt_state_owner: PromptStateOwner,
        prompt_id_allocator: PromptIdAllocator,
    ) -> Self {
        Self::with_store(
            AgentRuntimeStore::new(state),
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
        command: &crate::runtime::command::KernelCommand,
        mut request: crate::local::SubmitPromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_agent_actor_user_id(command);
        let agent_id = self
            .resolve_submit_agent_id(&request.session_id, request.target_agent_id.as_deref())
            .await?;
        self.store
            .ensure_agent_owner(&agent_id, &caller_user_id, "submit prompt")
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
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::CancelActivePromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_agent_actor_user_id(command);
        let agent_id = self
            .resolve_active_prompt_agent_id(&request.session_id)
            .await?;
        self.store
            .ensure_agent_owner(&agent_id, &caller_user_id, "cancel active prompt")
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
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::CompletePromptRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_user_id = command_agent_actor_user_id(command);
        let agent_id = self
            .resolve_active_prompt_agent_id(&request.session_id)
            .await?;
        self.store
            .ensure_agent_owner(&agent_id, &caller_user_id, "complete prompt")
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
    state: KernelRuntimeState,
}

impl AgentRuntimeStore {
    pub(crate) fn new(state: KernelRuntimeState) -> Self {
        Self { state }
    }

    async fn active_prompt_agent_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, DaemonError> {
        self.state.active_prompt_agent_id(session_id).await
    }

    async fn focused_agent_id(&self, session_id: &str) -> Result<Option<String>, DaemonError> {
        self.state.focused_agent_id(session_id).await
    }

    async fn ensure_agent_owner(
        &self,
        agent_id: &str,
        caller_user_id: &str,
        operation: &'static str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.state
            .ensure_agent_owner(agent_id, caller_user_id, operation)
            .await
    }

    fn prompt_command_service(
        &self,
        provider_runtime_lanes: ProviderRunOperationLanes,
    ) -> AgentPromptCommandService {
        AgentPromptCommandService::new(self.state.clone(), provider_runtime_lanes)
    }
}

fn command_agent_actor_user_id(command: &crate::runtime::command::KernelCommand) -> String {
    command
        .caller
        .user_id
        .clone()
        .unwrap_or_else(|| DEFAULT_LOCAL_USER_ID.to_string())
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
    use std::fs;
    use std::sync::Arc;

    use base64::Engine;
    use tokio::sync::Mutex;
    use tokio::time::{timeout, Duration};

    use crate::agent::CreateAgentRequest;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::local::{
        CancelActivePromptRequest, CompletePromptRequest, LocalDaemonRequest, LocalDaemonResponse,
        SubmitPromptRequest,
    };
    use crate::provider::{LaunchProviderRequest, ProviderRunOperationLanes};
    use crate::runtime::agent_actor::prompt_attachment_materialization::{
        materialize_inline_prompt_attachments, INLINE_PROMPT_ATTACHMENT_DIR,
    };
    use crate::runtime::agent_actor::AgentRuntime;
    use crate::runtime::projection::{AgentRuntimeProjectionStore, SessionStateProjectionStore};
    use crate::runtime::prompt_state::PromptStateOwner;
    use crate::runtime::session_actor::FocusedAgentProjection;
    use crate::runtime::state::KernelRuntimeState;
    use crate::session::{
        CreateSessionRequest, PromptAttachment, PromptQueueItem, PromptStatus,
        PromptSubmissionOutcome,
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
        app.update_provider_run_projection(provider_run.clone());
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

    mod agent_resolution;
    mod prompt_attachment_materialization;
    mod prompt_command_execution;
    mod request_surface;
}
