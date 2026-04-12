use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};

pub(crate) struct AgentActor;

impl AgentActor {
    pub(crate) fn handle_interactive_command(
        app: &mut DaemonApp,
        request: LocalDaemonRequest,
    ) -> Option<Result<LocalDaemonResponse, DaemonError>> {
        match request {
            LocalDaemonRequest::SubmitPrompt(request) => Some((|| {
                let outcome = app.submit_prompt(
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
                app.cancel_active_prompt(&request.session_id, &request.attachment_id)
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
