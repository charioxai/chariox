use super::DaemonApp;
use crate::session::PromptQueueItem;

mod prompt_commands;

pub(crate) struct KernelAgentService<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> KernelAgentService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
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
        test_support::LocalRouterTestHarness, AttachToSessionRequest, LocalDaemonRequest,
        LocalDaemonResponse,
    };
    use crate::provider::LaunchProviderRequest;
    use crate::session::{
        CreateSessionRequest, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
    };

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
        let harness = LocalRouterTestHarness::new();
        let (session, agent) = harness.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created")
        });
        let attachment = match harness
            .dispatch(LocalDaemonRequest::AttachToSession(
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
        let prepared = harness.with_app_mut(|app| {
            app.agents
                .bind_remote_execution(
                    agent.id(),
                    RemoteAgentBinding {
                        worker_kernel_id: "worker-kernel-1".to_string(),
                        worker_machine_id: "worker-machine-1".to_string(),
                        execution_lease_id: "lease-1".to_string(),
                        leased_agent_id: "leased-agent-1".to_string(),
                        active_worker_provider_run_id: None,
                        relay_url: None,
                        relay_token: None,
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

            crate::app::KernelAgentService::new(app)
                .submit_prepared_prompt_for_kernel(KernelPreparedPromptSubmission {
                    session_id: session.id().to_string(),
                    prompt,
                    force_queue: false,
                })
                .expect("prepared remote submit should not require relay I/O")
        });

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
            harness.with_app_mut(|app| {
                app.prompt_owner_active_prompt_for_agent(session.id(), agent.id())
                    .expect("prompt owner should resolve")
                    .is_some()
            }),
            "remote relay dispatch is now a deferred side effect; prompt ownership is already recorded"
        );
    }

    #[test]
    fn completion_uses_prompt_owner_when_session_mirror_is_stale() {
        let harness = LocalRouterTestHarness::new();
        let (session, agent) = harness.with_app_mut(|app| {
            crate::app::KernelSessionService::new(app)
                .create_session(CreateSessionRequest::new("workspace", "worktree"))
                .expect("session should be created")
        });
        let attachment = match harness
            .dispatch(LocalDaemonRequest::AttachToSession(
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
        let provider_run = harness.with_app_mut(|app| {
            app.launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider launch should succeed")
        });

        let outcome = harness.with_app_mut(|app| {
            app.submit_prompt(
                session.id(),
                attachment.id(),
                Some(agent.id()),
                "hello",
                Vec::new(),
            )
            .expect("prompt submit should succeed")
        });
        let prompt_id = match outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            _ => panic!("prompt should start"),
        };

        harness.with_app_mut(|app| {
            app.sessions_mut()
                .cancel_active_prompt(session.id(), agent.id())
                .expect("test should be able to corrupt only the compatibility mirror");
        });
        assert!(
            harness.with_app(|app| {
                app.sessions()
                    .get_session(session.id())
                    .expect("session mirror should exist")
                    .active_prompt_for_agent(agent.id())
                    .is_none()
            }),
            "compatibility mirror is intentionally stale"
        );

        let completion = harness.with_app_mut(|app| {
            app.complete_active_prompt(session.id(), agent.id(), Some(provider_run.id()))
                .expect("prompt owner should still complete active prompt")
        });

        assert_eq!(completion.completed.id(), prompt_id);
        assert!(
            harness.with_app(|app| {
                app.sessions()
                    .get_session(session.id())
                    .expect("session mirror should exist")
                    .active_prompt_for_agent(agent.id())
                    .is_none()
            }),
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
