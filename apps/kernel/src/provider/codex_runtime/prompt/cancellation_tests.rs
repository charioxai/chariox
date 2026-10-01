use super::*;
use crate::prompt_assembly::PromptManifest;
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::thread;
use tokio_tungstenite::tungstenite::{accept, connect, Message};

#[test]
fn cancelled_codex_turn_preserves_thread_when_hidden_context_changes() {
    cancelled_codex_turn_continuity(false);
}

#[test]
fn kernel_cancel_active_prompt_preserves_codex_thread_for_follow_up() {
    cancelled_codex_turn_continuity(true);
}

fn cancelled_codex_turn_continuity(through_kernel: bool) {
    use crate::local::*;
    let worktree = crate::test_support::TestWorktree::new("codex-cancel-continuity");
    let harness = crate::local::test_support::LocalRouterTestHarness::new();
    harness.with_app(|app| {
        let registry = app.provider_account_profile_registry();
        for profile in registry.list_all().unwrap() {
            crate::test_support::authenticate_provider_account(
                &registry,
                &profile.owner_user_id,
                &profile.provider,
                &profile.profile_id,
            )
            .unwrap();
        }
    });
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected response {other:?}"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "cancel-continuity".to_string(),
                capability_level: crate::attachment::ClientCapabilityLevel::FullTerminal,
            },
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected response {other:?}"),
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut socket = accept(stream).unwrap();
        let mut methods = Vec::new();
        for _ in 0..5 {
            let message = socket.read().unwrap();
            let request: serde_json::Value =
                serde_json::from_str(message.to_text().unwrap()).unwrap();
            let method = request["method"].as_str().unwrap();
            methods.push(method.to_string());
            let result = match method {
                "thread/start" => {
                    json!({"thread": {"id": if methods.len() == 1 { "thread-original" } else { "thread-new" }}, "model": "gpt-test"})
                }
                "thread/resume" => {
                    assert_eq!(request["params"]["threadId"], "thread-original");
                    assert_ne!(
                        request["params"]["developerInstructions"],
                        "context before cancellation"
                    );
                    json!({"thread": {"id": "thread-original"}, "model": "gpt-test"})
                }
                "turn/start" => json!({"turn": {"id": format!("turn-{}", methods.len())}}),
                "turn/interrupt" => json!({}),
                other => panic!("unexpected RPC {other}"),
            };
            socket
                .send(Message::Text(
                    json!({"id": request["id"], "result": result})
                        .to_string()
                        .into(),
                ))
                .unwrap();
        }
        methods
    });
    let (socket, _) = connect(&endpoint).unwrap();
    let mut run = RuntimeProviderRun::new(
        "provider-run-cancel-continuity",
        &LaunchProviderRequest::new(session.id(), "codex", "codex", "default", "default")
            .with_agent_id(agent.id()),
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "codex-fixture".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    let mut state = super::super::state::CodexRuntimeState::pending(endpoint, None, socket, 1);
    super::submit_codex_prompt(
        &run,
        &mut state,
        &PromptEnvelope::new(
            "remember cancelled context",
            "context before cancellation",
            Vec::new(),
            PromptManifest::current(),
        ),
    )
    .unwrap();
    if through_kernel {
        run.mark_running();
        harness.with_app_mut(|app| {
            app.agents_mut()
                .set_agent_runtime_profile_with_account_profile(
                    agent.id(),
                    "codex",
                    Some("default".to_string()),
                    None,
                    Some("default".to_string()),
                    crate::provider::ProviderResumeState::from_codex_thread_id("thread-original"),
                )
                .unwrap();
            app.providers_mut().insert_run_for_test(run.clone());
            app.providers_mut()
                .apply_runtime_binding(
                    run.id(),
                    crate::provider::ProviderRuntimeBinding::Codex(
                        super::super::CodexRuntimeBinding {
                            state,
                            selection: crate::provider::CodexRunSelection {
                                model: None,
                                variant: None,
                            },
                        },
                    ),
                )
                .unwrap();
            app.update_provider_run_projection(run.clone());
            app.sessions_mut()
                .set_active_provider_run(session.id(), Some(run.id().to_string()))
                .unwrap();
            let prompt = crate::session::PromptQueueItem::new(
                app.sessions_mut().reserve_prompt_id(),
                attachment.id(),
                agent.id(),
                "remember cancelled context",
                crate::session::PromptStatus::Queued,
            );
            app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
                .unwrap();
        });
        harness
            .dispatch(LocalDaemonRequest::CancelActivePrompt(
                CancelActivePromptRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    target_agent_id: Some(agent.id().to_string()),
                },
            ))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            harness.pump_transport_runtime();
            let idle = harness.with_app(|app| {
                app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                    .unwrap()
                    .is_none()
            });
            if idle {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "kernel cancellation did not settle"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        harness
            .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: Some(agent.id().to_string()),
                prompt: "continue".to_string(),
                attachments: Vec::new(),
            }))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !server.is_finished() {
            harness.pump_transport_runtime();
            assert!(
                std::time::Instant::now() < deadline,
                "follow-up was not dispatched"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        harness.pump_transport_runtime();
        harness.with_app(|app| {
            assert_eq!(
                app.agents()
                    .get_agent(agent.id())
                    .unwrap()
                    .provider_resume_state()
                    .codex_thread_id(),
                Some("thread-original")
            )
        });
    } else {
        super::abort_codex_turn(run.id(), &mut state).unwrap();
        super::submit_codex_prompt(
            &run,
            &mut state,
            &PromptEnvelope::new(
                "continue",
                "context after cancellation",
                Vec::new(),
                PromptManifest::current(),
            ),
        )
        .unwrap();
        assert_eq!(
            state.thread_id(),
            "thread-original",
            "user cancellation must preserve the conversation"
        );
    }
    let methods = server.join().unwrap();
    assert_eq!(
        methods,
        [
            "thread/start",
            "turn/start",
            "turn/interrupt",
            "thread/resume",
            "turn/start"
        ]
    );
}
