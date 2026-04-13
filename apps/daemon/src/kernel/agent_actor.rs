use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel::projection::{ActorQueueSnapshot, SessionStateProjectionStore};
use crate::kernel::session_actor::FocusedAgentProjection;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::ProviderRunOperationLanes;

const AGENT_COMMAND_QUEUE_LIMIT: usize = 128;

#[derive(Debug)]
enum AgentCommand {
    SubmitPrompt(crate::local::SubmitPromptRequest),
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
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
    focus_projection: FocusedAgentProjection,
    session_projection: SessionStateProjectionStore,
    queue_limit: usize,
    lanes: Arc<Mutex<HashMap<String, mpsc::Sender<AgentCommandEnvelope>>>>,
}

impl AgentRuntime {
    pub(crate) fn new(
        app: Arc<Mutex<DaemonApp>>,
        provider_runtime_lanes: ProviderRunOperationLanes,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
    ) -> Self {
        Self {
            app,
            provider_runtime_lanes,
            focus_projection,
            session_projection,
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
        let agent_id = self.resolve_focused_agent_id(&request.session_id).await?;
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

    async fn resolve_focused_agent_id(&self, session_id: &str) -> Result<String, DaemonError> {
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
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
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
}

async fn run_agent_command_lane(
    app: Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: ProviderRunOperationLanes,
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
        let result = execute_agent_command(&app, &provider_runtime_lanes, envelope.command).await;
        let _ = envelope.result_tx.send(result);
    }
}

async fn execute_agent_command(
    app: &Arc<Mutex<DaemonApp>>,
    provider_runtime_lanes: &ProviderRunOperationLanes,
    command: AgentCommand,
) -> Result<LocalDaemonResponse, DaemonError> {
    match command {
        AgentCommand::SubmitPrompt(request) => {
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
    use crate::session::{CreateSessionRequest, PromptStatus, PromptSubmissionOutcome};
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
}
