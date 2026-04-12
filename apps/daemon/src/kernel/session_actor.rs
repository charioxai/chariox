use crate::app::DaemonApp;
use crate::attachment::AttachRequest;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};

pub(crate) struct SessionActor;

impl SessionActor {
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
