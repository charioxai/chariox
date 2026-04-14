use super::DaemonApp;
use crate::agent::CreateAgentRequest;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::provider::ProviderRunState;
use crate::session::{PromptAttachment, PromptQueueItem, PromptStatus, PromptSubmissionOutcome};
use crate::transport::flow_control;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

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

    pub(crate) fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        self.app
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let session_before = self.app.sessions.get_session(session_id)?;

        let target_agent_id = target_agent_id
            .or_else(|| session_before.focused_agent_id())
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "no focused agent".to_string(),
            })?
            .to_string();
        let target_agent = self.app.agents.get_agent(&target_agent_id)?;
        let remote_execution = target_agent.remote_execution().cloned();
        let queued_while_active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, &target_agent_id)?
            .is_some();
        let provider_run_id = if remote_execution.is_some() {
            None
        } else if queued_while_active {
            self.app
                .providers
                .get_run_for_agent(session_id, &target_agent_id)
                .map(|run| run.id().to_string())
        } else {
            Some(
                self.app
                    .ensure_prompt_provider_run_for_agent(session_id, &target_agent_id)?,
            )
        };
        let provider_run_is_starting = provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.app.providers.get_run(provider_run_id).ok())
            .is_some_and(|run| run.state() == ProviderRunState::Starting);

        self.app.append_user_prompt_history(
            session_id,
            attachment_id,
            &target_agent_id,
            prompt,
            &attachments,
        );

        let prepared_prompt = PromptQueueItem::new(
            self.app.sessions_mut().reserve_prompt_id(),
            attachment_id,
            &target_agent_id,
            prompt,
            PromptStatus::Queued,
        )
        .with_attachments(attachments.clone());
        let outcome = self.app.prompt_owner_submit_prepared_prompt(
            session_id,
            prepared_prompt,
            provider_run_is_starting,
        )?;

        match &outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                if let Some(remote_execution) = remote_execution.as_ref() {
                    let response =
                        self.app
                            .block_on_relay_future(send_peer_request_via_temporary_connection(
                                self.app.config(),
                                ClientTarget {
                                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::SubmitLeasedPrompt {
                                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                                    prompt: prompt.prompt().to_string(),
                                    attachments: self.app.serialize_remote_prompt_attachments(
                                        prompt.attachments(),
                                    )?,
                                    workflow_context: None,
                                },
                            ));
                    let remote_provider_run_id = match response {
                        Ok(RelayPeerResponse::LeasedPromptSubmitted {
                            provider_run_id, ..
                        }) => provider_run_id,
                        Ok(other) => {
                            let _ = self.app.prompt_owner_cancel_active_prompt_only(
                                session_id,
                                &target_agent_id,
                            );
                            return Err(DaemonError::LocalTransport {
                                operation: "submit remote prompt",
                                message: format!("unexpected remote prompt response: {other:?}"),
                            });
                        }
                        Err(error) => {
                            let _ = self.app.prompt_owner_cancel_active_prompt_only(
                                session_id,
                                &target_agent_id,
                            );
                            return Err(error);
                        }
                    };
                    self.app.echo_prompt_to_other_attachments(
                        session_id,
                        &remote_provider_run_id,
                        prompt.source_attachment_id(),
                        prompt.prompt(),
                        prompt.attachments(),
                    );
                    return Ok(outcome);
                }
                let provider_run_id =
                    provider_run_id
                        .as_deref()
                        .ok_or_else(|| DaemonError::NoActiveProviderRun {
                            session_id: session_id.to_string(),
                        })?;
                self.app.echo_prompt_to_other_attachments(
                    session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                if let Err(error) = self.acquire_provider_prompt_claim(
                    session_id,
                    provider_run_id,
                    &target_agent_id,
                    prompt.source_attachment_id(),
                ) {
                    self.cancel_active_after_prompt_start_failure(
                        session_id,
                        &target_agent_id,
                        provider_run_id,
                    );
                    return Err(error);
                }
                if let Err(error) = self.app.dispatch_prompt_to_provider(
                    session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                ) {
                    self.cancel_active_after_prompt_start_failure(
                        session_id,
                        &target_agent_id,
                        provider_run_id,
                    );
                    return Err(error);
                }
                flow_control::note_prompt_started(self.app, provider_run_id);
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                let queue_depth = self
                    .app
                    .prompt_owner_queued_prompt_count_for_agent(session_id, &target_agent_id)
                    .unwrap_or(0);
                if let Some(provider_run_id) = provider_run_id.as_deref() {
                    self.app.echo_prompt_to_other_attachments(
                        session_id,
                        provider_run_id,
                        prompt.source_attachment_id(),
                        prompt.prompt(),
                        prompt.attachments(),
                    );
                }
                self.app.record_notice(
                    session_id,
                    provider_run_id.as_deref(),
                    self.app.other_attachment_ids(session_id, attachment_id),
                    format!(
                        "A queued message from attachment `{}` was added to agent `{}` in session `{}` as `{}`. Queue depth is now {}.",
                        attachment_id,
                        target_agent_id,
                        session_id,
                        prompt.id(),
                        queue_depth
                    ),
                );
            }
        }

        self.app.publish_session_projection(session_id)?;
        Ok(outcome)
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
