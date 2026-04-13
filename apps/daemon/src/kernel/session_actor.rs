use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::app::DaemonApp;
use crate::attachment::AttachRequest;
use crate::error::DaemonError;
use crate::kernel::projection::{
    ActorQueueSnapshot, AgentRuntimeProjectionStore, SessionStateProjectionStore,
};
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};

pub(crate) const SESSION_COMMAND_QUEUE_LIMIT: usize = 128;

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
    lanes: Arc<Mutex<HashMap<String, mpsc::Sender<SessionCommandEnvelope>>>>,
}

impl SessionRuntime {
    pub(crate) fn with_queue_limit_and_focus_projection(
        app: Arc<Mutex<DaemonApp>>,
        queue_limit: usize,
        focus_projection: FocusedAgentProjection,
        session_projection: SessionStateProjectionStore,
        agent_runtime_projection: AgentRuntimeProjectionStore,
    ) -> Self {
        Self {
            app,
            queue_limit,
            focus_projection,
            session_projection,
            agent_runtime_projection,
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
            LocalDaemonRequest::UpdateSessionConfig(request) => Ok(request.session_id.clone()),
            LocalDaemonRequest::AliasSession(request) => Ok(request.session_id.clone()),
            LocalDaemonRequest::EndSession(request) => Ok(request.session_id.clone()),
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
        let (result, projected_session) = {
            let mut app = app.lock().await;
            let result = SessionActor::handle_interactive_command(&mut app, envelope.request)
                .unwrap_or_else(|| {
                    Err(DaemonError::LocalTransport {
                        operation: "execute session kernel command",
                        message: "request is not handled by the session runtime".to_string(),
                    })
                });
            let projected_session = if result.is_ok() {
                session_id_for_projection_refresh(&result)
                    .and_then(|session_id| app.local_api_session_snapshot(&session_id).ok())
            } else {
                None
            };
            (result, projected_session)
        };
        if let Some(session) = projected_session.as_ref() {
            agent_runtime_projection.update_session(session);
            session_projection.update(session.clone());
        }
        update_focus_projection_after_session_command(
            &focus_projection,
            &session_id,
            &result,
            projected_session
                .as_ref()
                .and_then(|session| session.focused_agent_id()),
        )
        .await;
        let _ = envelope.result_tx.send(result);
    }
}

async fn update_focus_projection_after_session_command(
    focus_projection: &FocusedAgentProjection,
    session_id: &str,
    result: &Result<LocalDaemonResponse, DaemonError>,
    focused_agent_id: Option<&str>,
) {
    match result {
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

fn session_id_for_projection_refresh(
    result: &Result<LocalDaemonResponse, DaemonError>,
) -> Option<String> {
    match result {
        Ok(LocalDaemonResponse::SessionAttached { attachment })
        | Ok(LocalDaemonResponse::SessionDetached { attachment }) => {
            Some(attachment.session_id().to_string())
        }
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
            LocalDaemonRequest::AttachToSession(_)
                | LocalDaemonRequest::DetachFromSession(_)
                | LocalDaemonRequest::FocusAgent(_)
                | LocalDaemonRequest::CycleAgentFocus(_)
                | LocalDaemonRequest::ResizeTerminal(_)
                | LocalDaemonRequest::UpdateSessionConfig(_)
                | LocalDaemonRequest::AliasSession(_)
                | LocalDaemonRequest::EndSession(_)
                | LocalDaemonRequest::DeleteSession(_)
        )
    }

    pub(crate) fn handle_interactive_command(
        app: &mut DaemonApp,
        request: LocalDaemonRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        if let LocalDaemonRequest::UpdateSessionConfig(request) = request {
            let session_id = request.session_id.clone();
            return Some(
                app.update_session_config(
                    &request.session_id,
                    &request.attachment_id,
                    request.values,
                    request.requires_idle,
                )
                .and_then(|config| {
                    app.local_api_session_snapshot(&session_id).map(|session| {
                        LocalDaemonResponse::SessionConfigUpdated { config, session }
                    })
                }),
            );
        }
        if let LocalDaemonRequest::AliasSession(request) = request {
            return Some(
                app.sessions_mut()
                    .assign_session_alias(&request.session_id, request.alias)
                    .and_then(|_| app.local_api_session_snapshot(&request.session_id))
                    .map(|session| LocalDaemonResponse::SessionAliased { session }),
            );
        }

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
