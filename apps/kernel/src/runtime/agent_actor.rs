use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::provider::ProviderRunOperationLanes;
use crate::runtime::agent_prompt_service::AgentPromptCommandService;
use crate::runtime::projection::{
    ActorQueueSnapshot, AgentRuntimeProjectionStore, SessionStateProjectionStore,
};
use crate::runtime::prompt_state::PromptStateOwner;
use crate::runtime::session_actor::FocusedAgentProjection;
use crate::runtime::state::KernelRuntimeState;
use crate::session::{PromptIdAllocator, PromptQueueItem, DEFAULT_LOCAL_USER_ID};

const AGENT_COMMAND_QUEUE_LIMIT: usize = 128;

mod agent_resolution;
mod command_executor;
mod prompt_attachment_materialization;

use command_executor::AgentRuntimeCommandExecutor;

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
