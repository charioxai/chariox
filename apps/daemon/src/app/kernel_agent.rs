use super::DaemonApp;
use crate::agent::CreateAgentRequest;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::session::PromptQueueItem;

mod prompt_commands;

pub(crate) struct KernelAgentService<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> KernelAgentService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn execute_request(
        &mut self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::SubmitPrompt(request) => {
                let outcome = self.submit_prompt(
                    &request.session_id,
                    &request.attachment_id,
                    request.target_agent_id.as_deref(),
                    &request.prompt,
                    request.attachments,
                )?;
                let session = self.app.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::PromptSubmitted { outcome, session })
            }
            LocalDaemonRequest::CompletePrompt(request) => {
                let agent_id = self
                    .app
                    .sessions()
                    .get_session(&request.session_id)?
                    .active_prompt_agent_id()
                    .ok_or_else(|| DaemonError::NoActivePrompt {
                        session_id: request.session_id.clone(),
                    })?;
                let provider_run_id = self
                    .app
                    .providers()
                    .get_run_for_agent(&request.session_id, &agent_id)
                    .map(|run| run.id().to_string());
                let completion = self.complete_active_prompt(
                    &request.session_id,
                    &agent_id,
                    provider_run_id.as_deref(),
                )?;
                Ok(LocalDaemonResponse::PromptCompleted { completion })
            }
            LocalDaemonRequest::CancelActivePrompt(request) => {
                let cancellation =
                    self.cancel_active_prompt(&request.session_id, &request.attachment_id)?;
                Ok(LocalDaemonResponse::PromptCancelled { cancellation })
            }
            LocalDaemonRequest::SpawnAgent(request) => {
                let create_request =
                    CreateAgentRequest::new(&request.session_id, &request.provider);
                let create_request = if let Some(alias) = request.alias {
                    create_request.with_alias(alias)
                } else {
                    create_request
                };
                let create_request = if let Some(model) = request.model {
                    create_request.with_model(model)
                } else {
                    create_request
                };
                let create_request = if let Some(effort) = request.effort {
                    create_request.with_effort(effort)
                } else {
                    create_request
                };
                let create_request = if let Some(worktree_id) = request.worktree_id {
                    create_request.with_worktree(worktree_id)
                } else {
                    create_request
                };
                let create_request = if let Some(machine_ref) = request.machine_ref {
                    create_request.with_machine(machine_ref)
                } else {
                    create_request
                };
                let agent = self.app.spawn_agent(create_request)?;
                let _ = self.app.local_api_session_snapshot(agent.session_id())?;
                Ok(LocalDaemonResponse::AgentSpawned { agent })
            }
            LocalDaemonRequest::DestroyAgent(request) => {
                let agent = self.app.destroy_agent(&request.agent_id)?;
                let _ = self.app.local_api_session_snapshot(agent.session_id())?;
                Ok(LocalDaemonResponse::AgentDestroyed { agent })
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "execute agent request",
                message: "request is not handled by the agent runtime".to_string(),
            }),
        }
    }
}

fn select_next_queued_prompt_candidate(
    expected_next: Option<&PromptQueueItem>,
    fallback_next: Option<PromptQueueItem>,
) -> Option<PromptQueueItem> {
    expected_next.cloned().or(fallback_next)
}

#[cfg(test)]
mod tests {
    use super::select_next_queued_prompt_candidate;
    use crate::agent::RemoteAgentBinding;
    use crate::app::KernelPreparedPromptSubmission;
    use crate::attachment::ClientCapabilityLevel;
    use crate::local::{
        AttachToSessionRequest, LaunchProviderRunRequest, LocalDaemonRequest, LocalDaemonResponse,
    };
    use crate::session::{
        CreateSessionRequest, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
    };
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn queue_candidate_selection_prefers_runtime_expected_prompt() {
        let runtime_expected = prompt_item("prompt-runtime");
        let stale_fallback = prompt_item("prompt-fallback");

        let selected =
            select_next_queued_prompt_candidate(Some(&runtime_expected), Some(stale_fallback))
                .expect("candidate should be selected");

        assert_eq!(selected.id(), "prompt-runtime");
    }

    #[test]
    fn prepared_remote_submit_returns_dispatch_without_relay_io() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "cli-remote-submit".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };
        app.agents
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                },
            )
            .expect("agent should bind to remote execution");
        let prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "remote prompt should dispatch after ack",
            PromptStatus::Queued,
        );

        let prepared = app
            .kernel_agents()
            .submit_prepared_prompt_for_kernel(KernelPreparedPromptSubmission {
                session_id: session.id().to_string(),
                prompt,
                force_queue: false,
            })
            .expect("prepared remote submit should not require relay I/O");

        assert!(prepared.dispatch.is_none());
        let remote_dispatch = prepared
            .remote_dispatch
            .expect("started remote prompt should return deferred relay dispatch");
        assert_eq!(remote_dispatch.worker_kernel_id, "worker-kernel-1");
        assert_eq!(remote_dispatch.leased_agent_id, "leased-agent-1");
        match prepared.outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                assert_eq!(prompt.prompt(), "remote prompt should dispatch after ack");
            }
            PromptSubmissionOutcome::Queued { .. } => panic!("remote prompt should start"),
        }
        assert!(
            app.prompt_owner_active_prompt_for_agent(session.id(), agent.id())
                .expect("prompt owner should resolve")
                .is_some(),
            "remote relay dispatch is now a deferred side effect; prompt ownership is already recorded"
        );
    }

    #[test]
    fn completion_uses_prompt_owner_when_session_mirror_is_stale() {
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
        let provider_run = match app
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

        let outcome = app
            .submit_prompt(
                session.id(),
                attachment.id(),
                Some(agent.id()),
                "hello",
                Vec::new(),
            )
            .expect("prompt submit should succeed");
        let prompt_id = match outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            _ => panic!("prompt should start"),
        };

        app.sessions_mut()
            .cancel_active_prompt(session.id(), agent.id())
            .expect("test should be able to corrupt only the compatibility mirror");
        assert!(
            app.sessions()
                .get_session(session.id())
                .expect("session mirror should exist")
                .active_prompt_for_agent(agent.id())
                .is_none(),
            "compatibility mirror is intentionally stale"
        );

        let completion = app
            .complete_active_prompt(session.id(), agent.id(), Some(provider_run.id()))
            .expect("prompt owner should still complete active prompt");

        assert_eq!(completion.completed.id(), prompt_id);
        assert!(
            app.sessions()
                .get_session(session.id())
                .expect("session mirror should exist")
                .active_prompt_for_agent(agent.id())
                .is_none(),
            "owner completion should remirror the idle state"
        );
    }

    fn prompt_item(id: &str) -> PromptQueueItem {
        PromptQueueItem::new(
            id.to_string(),
            "attachment-1",
            "agent-1",
            "prompt",
            PromptStatus::Queued,
        )
    }
}
