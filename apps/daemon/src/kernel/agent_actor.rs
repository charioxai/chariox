use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel::projection::{
    ActorQueueSnapshot, AgentRuntimeProjection, AgentRuntimeProjectionStore,
    SessionStateProjectionStore,
};
use crate::kernel::session_actor::FocusedAgentProjection;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::ProviderRunOperationLanes;
use crate::session::{
    PromptCancellation, PromptCompletion, PromptQueueItem, PromptSubmissionOutcome, RuntimeSession,
};

const AGENT_COMMAND_QUEUE_LIMIT: usize = 128;

#[derive(Debug)]
enum AgentCommand {
    SubmitPrompt(crate::local::SubmitPromptRequest),
    CompletePrompt {
        request: crate::local::CompletePromptRequest,
        target_agent_id: String,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentRuntimePromptState {
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) active_prompt: Option<PromptQueueItem>,
    pub(crate) next_queued_prompt: Option<PromptQueueItem>,
    pub(crate) queued_prompt_count: usize,
}

#[derive(Clone, Default)]
pub(crate) struct AgentRuntimePromptStateStore {
    agents: Arc<StdMutex<HashMap<String, AgentRuntimePromptState>>>,
}

impl AgentRuntimePromptStateStore {
    pub(crate) fn get(&self, agent_id: &str) -> Option<AgentRuntimePromptState> {
        self.agents
            .lock()
            .expect("agent runtime prompt state lock should not be poisoned")
            .get(agent_id)
            .cloned()
    }

    pub(crate) fn list_for_session(&self, session_id: &str) -> Vec<AgentRuntimePromptState> {
        let mut states = self
            .agents
            .lock()
            .expect("agent runtime prompt state lock should not be poisoned")
            .values()
            .filter(|state| state.session_id == session_id)
            .cloned()
            .collect::<Vec<_>>();
        states.sort_by(|left, right| left.agent_id.cmp(&right.agent_id));
        states
    }

    pub(crate) fn update_session(&self, session: &RuntimeSession) {
        let mut agents = self
            .agents
            .lock()
            .expect("agent runtime prompt state lock should not be poisoned");
        agents.retain(|_, state| state.session_id != session.id());
        for agent in session.agents() {
            let prompt_state = session.prompt_states().get(agent.id());
            agents.insert(
                agent.id().to_string(),
                AgentRuntimePromptState {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                    active_prompt: prompt_state.and_then(|state| state.active_prompt().cloned()),
                    next_queued_prompt: prompt_state
                        .and_then(|state| state.queued_prompts().front().cloned()),
                    queued_prompt_count: prompt_state
                        .map(|state| state.queued_prompts().len())
                        .unwrap_or(0),
                },
            );
        }
        for (agent_id, prompt_state) in session.prompt_states() {
            agents
                .entry(agent_id.clone())
                .or_insert_with(|| AgentRuntimePromptState {
                    session_id: session.id().to_string(),
                    agent_id: agent_id.clone(),
                    active_prompt: prompt_state.active_prompt().cloned(),
                    next_queued_prompt: prompt_state.queued_prompts().front().cloned(),
                    queued_prompt_count: prompt_state.queued_prompts().len(),
                });
        }
    }

    pub(crate) fn update_agent_from_session(&self, session: &RuntimeSession, agent_id: &str) {
        let mut agents = self
            .agents
            .lock()
            .expect("agent runtime prompt state lock should not be poisoned");
        let Some(prompt_state) = agent_prompt_state_from_session(session, agent_id) else {
            agents.remove(agent_id);
            return;
        };
        agents.insert(agent_id.to_string(), prompt_state);
    }

    pub(crate) fn apply_submission_outcome(
        &self,
        session_id: &str,
        agent_id: &str,
        outcome: &PromptSubmissionOutcome,
    ) {
        let mut agents = self
            .agents
            .lock()
            .expect("agent runtime prompt state lock should not be poisoned");
        let state = agents
            .entry(agent_id.to_string())
            .or_insert_with(|| empty_agent_prompt_state(session_id, agent_id));
        match outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                state.active_prompt = Some(prompt.clone());
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                if state.next_queued_prompt.is_none() {
                    state.next_queued_prompt = Some(prompt.clone());
                }
                state.queued_prompt_count = state.queued_prompt_count.saturating_add(1);
            }
        }
    }

    pub(crate) fn apply_cancellation(
        &self,
        session_id: &str,
        agent_id: &str,
        cancellation: &PromptCancellation,
    ) {
        let mut agents = self
            .agents
            .lock()
            .expect("agent runtime prompt state lock should not be poisoned");
        let state = agents
            .entry(agent_id.to_string())
            .or_insert_with(|| empty_agent_prompt_state(session_id, agent_id));
        state.active_prompt = cancellation
            .started_next
            .clone()
            .or_else(|| Some(cancellation.prompt.clone()));
        if cancellation.started_next.is_some() {
            state.queued_prompt_count = state.queued_prompt_count.saturating_sub(1);
            state.next_queued_prompt = None;
        }
    }

    pub(crate) fn apply_completion(
        &self,
        session_id: &str,
        agent_id: &str,
        completion: &PromptCompletion,
    ) {
        let mut agents = self
            .agents
            .lock()
            .expect("agent runtime prompt state lock should not be poisoned");
        let state = agents
            .entry(agent_id.to_string())
            .or_insert_with(|| empty_agent_prompt_state(session_id, agent_id));
        state.active_prompt = completion.started_next.clone();
        if completion.started_next.is_some() {
            state.queued_prompt_count = state.queued_prompt_count.saturating_sub(1);
            state.next_queued_prompt = None;
        }
    }

    pub(crate) fn remove_agent(&self, agent_id: &str) {
        self.agents
            .lock()
            .expect("agent runtime prompt state lock should not be poisoned")
            .remove(agent_id);
    }

    pub(crate) fn remove_session(&self, session_id: &str) {
        self.agents
            .lock()
            .expect("agent runtime prompt state lock should not be poisoned")
            .retain(|_, state| state.session_id != session_id);
    }
}

fn empty_agent_prompt_state(session_id: &str, agent_id: &str) -> AgentRuntimePromptState {
    AgentRuntimePromptState {
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        active_prompt: None,
        next_queued_prompt: None,
        queued_prompt_count: 0,
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct AgentRuntime {
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    prompt_state: AgentRuntimePromptStateStore,
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
    ) -> Self {
        Self {
            app,
            provider_runtime_lanes,
            focus_projection,
            session_projection,
            agent_runtime_projection,
            prompt_state: AgentRuntimePromptStateStore::default(),
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
            AgentCommand::SubmitPrompt(request),
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
        self.dispatch_to_agent(
            agent_id.clone(),
            command.command_id.clone(),
            command.command_type.clone(),
            AgentCommand::CompletePrompt {
                request,
                target_agent_id: agent_id.clone(),
            },
        )
        .await
    }

    async fn resolve_active_prompt_agent_id(
        &self,
        session_id: &str,
    ) -> Result<String, DaemonError> {
        if let Some(agent_id) = self.resolve_state_active_prompt_agent_id(session_id).await {
            return Ok(agent_id);
        }
        if let Some(agent_id) = self
            .resolve_projected_active_prompt_agent_id(session_id)
            .await
        {
            return Ok(agent_id);
        }
        if let Some(agent_id) = self
            .session_projection
            .get(session_id)
            .and_then(|session| active_prompt_agent_id(&session))
        {
            return Ok(agent_id);
        }
        let app = self.app.lock().await;
        active_prompt_agent_id(&app.sessions().get_session(session_id)?).ok_or_else(|| {
            DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            }
        })
    }

    async fn resolve_state_active_prompt_agent_id(&self, session_id: &str) -> Option<String> {
        if let Some(focused_agent_id) = self.focus_projection.focused_agent_id(session_id).await {
            if self
                .prompt_state
                .get(&focused_agent_id)
                .is_some_and(|state| {
                    state.session_id == session_id && state.active_prompt.is_some()
                })
            {
                return Some(focused_agent_id);
            }
        }

        let session_focused_agent_id = self
            .session_projection
            .get(session_id)
            .and_then(|session| session.focused_agent_id().map(str::to_string));
        active_prompt_agent_id_from_state(
            session_focused_agent_id.as_deref(),
            &self.prompt_state.list_for_session(session_id),
        )
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
        if let Some(agent_id) = target_agent_id {
            return Ok(agent_id.to_string());
        }
        if let Some(agent_id) = self.focus_projection.focused_agent_id(session_id).await {
            return Ok(agent_id);
        }
        if let Some(agent_id) = self
            .session_projection
            .get(session_id)
            .and_then(|session| session.focused_agent_id().map(str::to_string))
        {
            return Ok(agent_id);
        }
        let app = self.app.lock().await;
        app.sessions()
            .get_session(session_id)?
            .focused_agent_id()
            .map(str::to_string)
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
        tokio::spawn(run_agent_command_lane(
            Arc::clone(&self.app),
            self.provider_runtime_lanes.clone(),
            self.session_projection.clone(),
            self.agent_runtime_projection.clone(),
            self.prompt_state.clone(),
            agent_id.to_string(),
            rx,
        ));
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
        self.prompt_state.remove_agent(agent_id);
    }

    pub(crate) async fn remove_agent_lanes<'a>(
        &self,
        agent_ids: impl IntoIterator<Item = &'a str>,
    ) {
        let mut lanes = self.lanes.lock().await;
        for agent_id in agent_ids {
            lanes.remove(agent_id);
            self.prompt_state.remove_agent(agent_id);
        }
    }

    pub(crate) fn remove_session_state(&self, session_id: &str) {
        self.prompt_state.remove_session(session_id);
    }

    pub(crate) fn update_prompt_state_from_session(&self, session: &RuntimeSession) {
        self.prompt_state.update_session(session);
    }

    #[cfg(test)]
    pub(crate) fn prompt_state_for_test(&self, agent_id: &str) -> Option<AgentRuntimePromptState> {
        self.prompt_state.get(agent_id)
    }
}

fn agent_prompt_state_from_session(
    session: &RuntimeSession,
    agent_id: &str,
) -> Option<AgentRuntimePromptState> {
    if !session.agents().iter().any(|agent| agent.id() == agent_id)
        && !session.prompt_states().contains_key(agent_id)
    {
        return None;
    }
    let prompt_state = session.prompt_states().get(agent_id);
    Some(AgentRuntimePromptState {
        session_id: session.id().to_string(),
        agent_id: agent_id.to_string(),
        active_prompt: prompt_state.and_then(|state| state.active_prompt().cloned()),
        next_queued_prompt: prompt_state.and_then(|state| state.queued_prompts().front().cloned()),
        queued_prompt_count: prompt_state
            .map(|state| state.queued_prompts().len())
            .unwrap_or(0),
    })
}

fn active_prompt_agent_id(session: &crate::session::RuntimeSession) -> Option<String> {
    if let Some(focused_agent_id) = session.focused_agent_id() {
        if session.active_prompt_for_agent(focused_agent_id).is_some() {
            return Some(focused_agent_id.to_string());
        }
    }
    let mut active_agents = session
        .prompt_states()
        .iter()
        .filter(|(_, state)| state.active_prompt().is_some())
        .map(|(agent_id, _)| agent_id.clone());
    let agent_id = active_agents.next()?;
    if active_agents.next().is_none() {
        Some(agent_id)
    } else {
        None
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

fn active_prompt_agent_id_from_state(
    focused_agent_id: Option<&str>,
    states: &[AgentRuntimePromptState],
) -> Option<String> {
    if let Some(focused_agent_id) = focused_agent_id {
        if states
            .iter()
            .any(|state| state.agent_id == focused_agent_id && state.active_prompt.is_some())
        {
            return Some(focused_agent_id.to_string());
        }
    }
    let mut active_agents = states
        .iter()
        .filter(|state| state.active_prompt.is_some())
        .map(|state| state.agent_id.clone());
    let agent_id = active_agents.next()?;
    if active_agents.next().is_none() {
        Some(agent_id)
    } else {
        None
    }
}

async fn run_agent_command_lane(
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
    session_projection: SessionStateProjectionStore,
    agent_runtime_projection: AgentRuntimeProjectionStore,
    prompt_state: AgentRuntimePromptStateStore,
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
        let result = execute_agent_command(
            &app,
            &provider_runtime_lanes,
            &session_projection,
            &agent_runtime_projection,
            &prompt_state,
            envelope.command,
        )
        .await;
        let _ = envelope.result_tx.send(result);
    }
}

async fn execute_agent_command(
    app: &Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: &ProviderRunOperationLanes,
    session_projection: &SessionStateProjectionStore,
    agent_runtime_projection: &AgentRuntimeProjectionStore,
    prompt_state: &AgentRuntimePromptStateStore,
    command: AgentCommand,
) -> Result<LocalDaemonResponse, DaemonError> {
    match command {
        AgentCommand::SubmitPrompt(request) => {
            let target_agent_id =
                request
                    .target_agent_id
                    .clone()
                    .ok_or_else(|| DaemonError::AgentNotFound {
                        agent_id: "no target agent".to_string(),
                    })?;
            let prepared = {
                let mut app = app.lock().await;
                app.kernel_agents().submit_prompt_for_kernel(
                    &request.session_id,
                    &request.attachment_id,
                    request.target_agent_id.as_deref(),
                    &request.prompt,
                    request.attachments,
                )?
            };
            session_projection.update(prepared.session.clone());
            prompt_state.apply_submission_outcome(
                &request.session_id,
                &target_agent_id,
                &prepared.outcome,
            );
            agent_runtime_projection.update_agent_from_session(&prepared.session, &target_agent_id);
            prompt_state.update_agent_from_session(&prepared.session, &target_agent_id);

            if let Some(dispatch) = prepared.dispatch {
                DaemonApp::spawn_kernel_prompt_dispatch_operation(
                    Arc::clone(app),
                    provider_runtime_lanes.clone(),
                    dispatch,
                );
            }

            Ok(LocalDaemonResponse::PromptSubmitted {
                outcome: prepared.outcome,
                session: prepared.session,
            })
        }
        AgentCommand::CancelActivePrompt {
            request,
            target_agent_id,
        } => {
            let prepared = {
                let mut app = app.lock().await;
                app.kernel_agents().cancel_agent_prompt_for_kernel(
                    &request.session_id,
                    &target_agent_id,
                    &request.attachment_id,
                )?
            };
            session_projection.update(prepared.session.clone());
            prompt_state.apply_cancellation(
                &request.session_id,
                &target_agent_id,
                &prepared.cancellation,
            );
            agent_runtime_projection.update_agent_from_session(&prepared.session, &target_agent_id);
            prompt_state.update_agent_from_session(&prepared.session, &target_agent_id);

            if let Some(dispatch) = prepared.dispatch {
                DaemonApp::spawn_kernel_prompt_abort_operation(
                    Arc::clone(app),
                    provider_runtime_lanes.clone(),
                    dispatch,
                );
            }

            Ok(LocalDaemonResponse::PromptCancelled {
                cancellation: prepared.cancellation,
            })
        }
        AgentCommand::CompletePrompt {
            request,
            target_agent_id,
        } => {
            let (completion, session) = {
                let mut app = app.lock().await;
                let provider_run_id = app
                    .providers()
                    .get_run_for_agent(&request.session_id, &target_agent_id)
                    .map(|run| run.id().to_string());
                let completion = app.complete_active_prompt(
                    &request.session_id,
                    &target_agent_id,
                    provider_run_id.as_deref(),
                )?;
                let session = app.local_api_session_snapshot(&request.session_id)?;
                (completion, session)
            };
            session_projection.update(session.clone());
            prompt_state.apply_completion(&request.session_id, &target_agent_id, &completion);
            agent_runtime_projection.update_agent_from_session(&session, &target_agent_id);
            prompt_state.update_agent_from_session(&session, &target_agent_id);

            Ok(LocalDaemonResponse::PromptCompleted { completion })
        }
    }
}

pub(crate) struct AgentActor;

impl AgentActor {
    pub(crate) fn handle_interactive_command(
        app: &mut DaemonApp,
        request: LocalDaemonRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        match request {
            LocalDaemonRequest::SubmitPrompt(request) => Some((|| {
                let outcome = app.kernel_agents().submit_prompt(
                    &request.session_id,
                    &request.attachment_id,
                    request.target_agent_id.as_deref(),
                    &request.prompt,
                    request.attachments,
                )?;
                let session = app.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::PromptSubmitted { outcome, session })
            })()),
            LocalDaemonRequest::CancelActivePrompt(request) => Some(
                app.kernel_agents()
                    .cancel_active_prompt(&request.session_id, &request.attachment_id)
                    .map(|cancellation| LocalDaemonResponse::PromptCancelled { cancellation }),
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::attachment::ClientCapabilityLevel;
    use crate::kernel::agent_actor::AgentActor;
    use crate::local::{
        AttachToSessionRequest, CancelActivePromptRequest, LaunchProviderRunRequest,
        LocalDaemonRequest, LocalDaemonResponse, SubmitPromptRequest,
    };
    use crate::session::{
        CreateSessionRequest, PromptCancellation, PromptCompletion, PromptQueueItem, PromptStatus,
        PromptSubmissionOutcome,
    };
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn handles_prompt_submit_through_agent_actor_surface() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "cli-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };
        let _provider_run = match app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(agent.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                    variant: None,
                },
            ))
            .expect("provider launch should succeed")
        {
            LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
            _ => panic!("unexpected local response"),
        };

        let response = AgentActor::handle_interactive_command(
            &mut app,
            LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: Some(agent.id().to_string()),
                prompt: "hello".to_string(),
                attachments: Vec::new(),
            }),
        )
        .expect("actor should handle prompt submit")
        .expect("prompt submit should succeed");

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
    fn handles_prompt_cancel_through_agent_actor_surface() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "cli-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };
        let _provider_run = match app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(agent.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                    variant: None,
                },
            ))
            .expect("provider launch should succeed")
        {
            LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
            _ => panic!("unexpected local response"),
        };
        AgentActor::handle_interactive_command(
            &mut app,
            LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: Some(agent.id().to_string()),
                prompt: "hello".to_string(),
                attachments: Vec::new(),
            }),
        )
        .expect("actor should handle prompt submit")
        .expect("prompt submit should succeed");

        let response = AgentActor::handle_interactive_command(
            &mut app,
            LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            }),
        )
        .expect("actor should handle prompt cancel")
        .expect("prompt cancel should succeed");

        match response {
            LocalDaemonResponse::PromptCancelled { cancellation } => {
                assert_eq!(cancellation.prompt.target_agent_id(), agent.id());
                assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
            }
            _ => panic!("unexpected local response"),
        }
    }

    #[test]
    fn prompt_state_store_applies_mailbox_lifecycle_facts() {
        let store = super::AgentRuntimePromptStateStore::default();
        let session_id = "session-1";
        let agent_id = "agent-1";
        let active = prompt_item("prompt-1", agent_id, "active");
        let queued = prompt_item("prompt-2", agent_id, "queued");

        store.apply_submission_outcome(
            session_id,
            agent_id,
            &PromptSubmissionOutcome::Started {
                prompt: active.clone(),
            },
        );
        let state = store
            .get(agent_id)
            .expect("started prompt should create runtime state");
        assert_eq!(
            state.active_prompt.as_ref().map(|prompt| prompt.id()),
            Some("prompt-1")
        );
        assert_eq!(state.queued_prompt_count, 0);

        store.apply_submission_outcome(
            session_id,
            agent_id,
            &PromptSubmissionOutcome::Queued {
                prompt: queued.clone(),
            },
        );
        let state = store
            .get(agent_id)
            .expect("queued prompt should update runtime state");
        assert_eq!(
            state.next_queued_prompt.as_ref().map(|prompt| prompt.id()),
            Some("prompt-2")
        );
        assert_eq!(state.queued_prompt_count, 1);

        store.apply_cancellation(
            session_id,
            agent_id,
            &PromptCancellation {
                prompt: active,
                started_next: None,
            },
        );
        let state = store
            .get(agent_id)
            .expect("cancellation should keep runtime state");
        assert_eq!(
            state.active_prompt.as_ref().map(|prompt| prompt.id()),
            Some("prompt-1")
        );

        store.apply_completion(
            session_id,
            agent_id,
            &PromptCompletion {
                completed: prompt_item("prompt-1", agent_id, "active"),
                started_next: Some(queued),
            },
        );
        let state = store
            .get(agent_id)
            .expect("completion should keep runtime state");
        assert_eq!(
            state.active_prompt.as_ref().map(|prompt| prompt.id()),
            Some("prompt-2")
        );
        assert_eq!(state.queued_prompt_count, 0);
    }

    fn prompt_item(id: &str, agent_id: &str, prompt: &str) -> PromptQueueItem {
        PromptQueueItem::new(
            id.to_string(),
            "attachment-1".to_string(),
            agent_id.to_string(),
            prompt.to_string(),
            PromptStatus::Queued,
        )
    }
}
