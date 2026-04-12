use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::app::DaemonApp;
use crate::attachment::AttachRequest;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};

const SESSION_COMMAND_QUEUE_LIMIT: usize = 128;

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
    lanes: Arc<Mutex<HashMap<String, mpsc::Sender<SessionCommandEnvelope>>>>,
}

impl SessionRuntime {
    pub(crate) fn new(app: Arc<Mutex<DaemonApp>>) -> Self {
        Self::with_queue_limit(app, SESSION_COMMAND_QUEUE_LIMIT)
    }

    pub(crate) fn with_queue_limit(app: Arc<Mutex<DaemonApp>>, queue_limit: usize) -> Self {
        Self {
            app,
            queue_limit,
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
            LocalDaemonRequest::AttachToSession(request) => Ok(request.session_id.clone()),
            LocalDaemonRequest::FocusAgent(request) => Ok(request.session_id.clone()),
            LocalDaemonRequest::CycleAgentFocus(request) => Ok(request.session_id.clone()),
            LocalDaemonRequest::ResizeTerminal(request) => Ok(request.session_id.clone()),
            LocalDaemonRequest::EndSession(request) => Ok(request.session_id.clone()),
            LocalDaemonRequest::DeleteSession(request) => {
                let app = self.app.lock().await;
                Ok(app
                    .resolve_session_ref(&request.session_ref, request.workspace_id.as_deref())?
                    .id()
                    .to_string())
            }
            LocalDaemonRequest::DetachFromSession(request) => {
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

    async fn session_lane(&self, session_id: &str) -> mpsc::Sender<SessionCommandEnvelope> {
        let mut lanes = self.lanes.lock().await;
        if let Some(lane) = lanes.get(session_id) {
            return lane.clone();
        }
        let (tx, rx) = mpsc::channel(self.queue_limit);
        lanes.insert(session_id.to_string(), tx.clone());
        tokio::spawn(run_session_command_lane(
            Arc::clone(&self.app),
            session_id.to_string(),
            rx,
        ));
        tx
    }

    async fn remove_session_lane(&self, session_id: &str) {
        let mut lanes = self.lanes.lock().await;
        lanes.remove(session_id);
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
    session_id: String,
    mut rx: mpsc::Receiver<SessionCommandEnvelope>,
) {
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
        let result = {
            let mut app = app.lock().await;
            SessionActor::handle_interactive_command(&mut app, envelope.request).unwrap_or_else(
                || {
                    Err(DaemonError::LocalTransport {
                        operation: "execute session kernel command",
                        message: "request is not handled by the session runtime".to_string(),
                    })
                },
            )
        };
        let _ = envelope.result_tx.send(result);
    }
}

pub(crate) struct SessionActor;

impl SessionActor {
    pub(crate) fn is_session_interactive_command(request: &LocalDaemonRequest) -> bool {
        matches!(
            request,
            LocalDaemonRequest::AttachToSession(_)
                | LocalDaemonRequest::DetachFromSession(_)
                | LocalDaemonRequest::FocusAgent(_)
                | LocalDaemonRequest::CycleAgentFocus(_)
                | LocalDaemonRequest::ResizeTerminal(_)
                | LocalDaemonRequest::EndSession(_)
                | LocalDaemonRequest::DeleteSession(_)
        )
    }

    pub(crate) fn handle_interactive_command(
        app: &mut DaemonApp,
        request: LocalDaemonRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        let mut sessions = app.kernel_sessions();
        match request {
            LocalDaemonRequest::AttachToSession(request) => Some(
                sessions
                    .attach(AttachRequest::new(
                        request.session_id,
                        request.client_id,
                        request.capability_level,
                    ))
                    .map(|attachment| LocalDaemonResponse::SessionAttached { attachment }),
            ),
            LocalDaemonRequest::DetachFromSession(request) => Some(
                sessions
                    .detach(&request.attachment_id)
                    .map(|attachment| LocalDaemonResponse::SessionDetached { attachment }),
            ),
            LocalDaemonRequest::FocusAgent(request) => Some(
                sessions
                    .focus_agent(&request.session_id, &request.agent_id)
                    .map(|agent| LocalDaemonResponse::AgentFocused { agent }),
            ),
            LocalDaemonRequest::CycleAgentFocus(request) => Some(
                sessions
                    .cycle_agent_focus(&request.session_id)
                    .map(|agent| LocalDaemonResponse::AgentFocusCycled { agent }),
            ),
            LocalDaemonRequest::ResizeTerminal(request) => {
                let session_id = request.session_id;
                let cols = request.cols;
                let rows = request.rows;
                Some(sessions.resize_terminal(&session_id, cols, rows).map(|()| {
                    LocalDaemonResponse::TerminalResized {
                        session_id,
                        cols,
                        rows,
                    }
                }))
            }
            LocalDaemonRequest::EndSession(request) => Some(
                sessions
                    .end_session(&request.session_id)
                    .map(|session| LocalDaemonResponse::SessionEnded { session }),
            ),
            LocalDaemonRequest::DeleteSession(request) => Some(
                sessions
                    .delete_session_ref(&request.session_ref, request.workspace_id.as_deref())
                    .map(|session| LocalDaemonResponse::SessionDeleted { session }),
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::kernel::session_actor::SessionActor;
    use crate::local::{
        AttachToSessionRequest, FocusAgentRequest, LaunchProviderRunRequest, LocalDaemonRequest,
        LocalDaemonResponse, SpawnAgentRequest, SubmitPromptRequest,
    };
    use crate::session::{CreateSessionRequest, PromptSubmissionOutcome};
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn handles_attach_through_session_actor_surface() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let response = SessionActor::handle_interactive_command(
            &mut app,
            LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "cli-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            }),
        )
        .expect("actor should handle attach")
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

        SessionActor::handle_interactive_command(
            &mut app,
            LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: session.id().to_string(),
                agent_id: default_agent.id().to_string(),
            }),
        )
        .expect("actor should handle focus")
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

        let response = SessionActor::handle_interactive_command(
            &mut app,
            LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: session.id().to_string(),
                agent_id: second_agent.id().to_string(),
            }),
        )
        .expect("actor should handle focus")
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
