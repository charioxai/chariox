use super::*;
use crate::prompt_assembly::PromptManifest;
use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};
use std::collections::BTreeMap;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use tokio_tungstenite::tungstenite::{accept, connect, stream::MaybeTlsStream, Message};

// Only bound missing completion events. The server must remain connected while
// the full parallel suite schedules kernel setup between RPCs.
const FIXTURE_DEADLINE: Duration = Duration::from_secs(30);

struct CloseFixtureSocket(TcpStream);

impl Drop for CloseFixtureSocket {
    fn drop(&mut self) {
        // Unblock the server even if a completion deadline or assertion fails.
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

#[test]
fn cancelled_codex_turn_preserves_thread_when_hidden_context_changes() {
    cancelled_codex_turn_continuity(false, "context after cancellation");
}

#[test]
fn kernel_cancel_active_prompt_preserves_codex_thread_for_follow_up() {
    cancelled_codex_turn_continuity(true, "context after cancellation");
}

// Existing Codex threads receive hidden context as injected developer items
// (release F). An empty context injects nothing; the thread is still kept.
#[test]
fn cancelled_codex_turn_without_hidden_context_keeps_thread_and_injects_nothing() {
    cancelled_codex_turn_continuity(false, "");
}

async fn await_fixture_acknowledgement(
    finished: &mut tokio::sync::oneshot::Receiver<()>,
    deadline: Duration,
) {
    tokio::time::timeout(deadline, finished)
        .await
        .expect("Codex fixture did not acknowledge the follow-up")
        .expect("Codex fixture stopped before acknowledging the follow-up");
}

#[test]
fn missing_fixture_completion_times_out_and_unblocks_the_server() {
    use std::io::Read;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    // Safety bound for a broken shutdown guard; success is driven by EOF.
    peer.set_read_timeout(Some(FIXTURE_DEADLINE)).unwrap();
    let (finished_tx, mut finished_rx) = tokio::sync::oneshot::channel();
    let server = thread::spawn(move || {
        let _hold_completion = finished_tx;
        peer.read(&mut [0])
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _close_fixture_socket = CloseFixtureSocket(client.try_clone().unwrap());
        runtime.block_on(await_fixture_acknowledgement(
            &mut finished_rx,
            Duration::ZERO,
        ));
    }));
    assert!(
        failure.is_err(),
        "a missing completion must fail before join"
    );
    // The original client remains open: EOF proves the unwind guard shut it down.
    assert_eq!(server.join().unwrap().unwrap(), 0);
}

fn cancelled_codex_turn_continuity(through_kernel: bool, next_context: &str) {
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
    let (finished_tx, mut finished_rx) = tokio::sync::oneshot::channel();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = accept(stream).unwrap();
        let mut methods = Vec::new();
        let mut applied_context = serde_json::Value::Null;
        let mut turn_count = 0;
        loop {
            let message = socket
                .read()
                .unwrap_or_else(|error| panic!("Codex fixture read after {methods:?}: {error}"));
            let request: serde_json::Value =
                serde_json::from_str(message.to_text().unwrap()).unwrap();
            let method = request["method"].as_str().unwrap();
            methods.push(method.to_string());
            let result = match method {
                "thread/start" => {
                    applied_context = request["params"]["developerInstructions"].clone();
                    json!({"thread": {"id": if methods.len() == 1 { "thread-original" } else { "thread-new" }}, "model": "gpt-test"})
                }
                "thread/inject_items" => {
                    assert_eq!(request["params"]["threadId"], "thread-original");
                    let item = &request["params"]["items"][0];
                    assert_eq!(item["role"], "developer");
                    applied_context = item["content"][0]["text"].clone();
                    json!({})
                }
                "turn/start" => {
                    turn_count += 1;
                    json!({"turn": {"id": format!("turn-{}", methods.len())}})
                }
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
            if turn_count == 2 {
                let _ = finished_tx.send(());
                break;
            }
        }
        (methods, applied_context)
    });
    let (socket, _) = connect(&endpoint).unwrap();
    let MaybeTlsStream::Plain(stream) = socket.get_ref() else {
        unreachable!("fixture uses a plain local WebSocket");
    };
    let _close_fixture_socket = CloseFixtureSocket(stream.try_clone().unwrap());
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
        let runtime = harness.runtime_state();
        let deadline = tokio::time::Instant::now() + FIXTURE_DEADLINE;
        loop {
            // Capture before pumping/checking: completion can race with either.
            let completion = runtime.provider_run_actor_completion_sequence();
            let wake = runtime.transport_runtime_pump_change_sequence();
            let projection = runtime.session_projection_session_change_sequence(session.id());
            harness.pump_transport_runtime();
            let idle = harness.with_app(|app| {
                app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
                    .unwrap()
                    .is_none()
            });
            if idle {
                break;
            }
            harness.block_on_test_task(async {
                tokio::time::timeout_at(deadline, async {
                    tokio::select! {
                        _ = runtime.wait_for_provider_run_actor_completion_after(completion) => {},
                        _ = runtime.wait_for_transport_runtime_pump_change_after(wake) => {},
                        _ = runtime.wait_for_session_projection_session_change_after(session.id(), projection) => {},
                    }
                })
                .await
                .expect("kernel cancellation did not settle");
            });
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
        harness.block_on_test_task(async {
            tokio::time::timeout(FIXTURE_DEADLINE, async {
                loop {
                    let completion = runtime.provider_run_actor_completion_sequence();
                    let wake = runtime.transport_runtime_pump_change_sequence();
                    let projection = runtime.session_projection_session_change_sequence(session.id());
                    runtime.pump_transport_runtime().await;
                    tokio::select! {
                        finished = &mut finished_rx => {
                            finished.expect("Codex fixture stopped before acknowledging the follow-up");
                            break;
                        },
                        _ = runtime.wait_for_provider_run_actor_completion_after(completion) => {},
                        _ = runtime.wait_for_transport_runtime_pump_change_after(wake) => {},
                        _ = runtime.wait_for_session_projection_session_change_after(session.id(), projection) => {},
                    }
                }
            })
            .await
            .unwrap_or_else(|_| {
                let session = runtime.list_session_snapshots().into_iter().find(|s| s.id() == session.id()).unwrap();
                let active = session.active_prompt_for_agent(agent.id()).map(|p| (p.status(), p.durable_delivery_phase()));
                let queued = session.queued_prompts_for_agent(agent.id()).map(|q| q.len());
                let provider = runtime.provider_runs_for_external_session_attachment().into_iter().find(|p| p.id() == run.id()).map(|p| p.state());
                panic!("follow-up was not dispatched: active={active:?}, queued={queued:?}, provider={provider:?}");
            });
        });
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
                next_context,
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
        // An adapter can buffer an RPC error and return Ok without starting turn two.
        // Await its completion before join so that failure unwinds the socket guard.
        harness.block_on_test_task(await_fixture_acknowledgement(
            &mut finished_rx,
            FIXTURE_DEADLINE,
        ));
    }
    let (methods, applied_context) = server.join().unwrap();
    if !through_kernel && next_context.is_empty() {
        assert_eq!(applied_context, "context before cancellation");
        assert_eq!(
            methods,
            ["thread/start", "turn/start", "turn/interrupt", "turn/start"]
        );
        return;
    }
    assert_ne!(
        applied_context, "context before cancellation",
        "the follow-up must actually use updated provider instructions"
    );
    if !through_kernel {
        assert_eq!(applied_context, next_context);
    }
    assert_eq!(
        methods,
        [
            "thread/start",
            "turn/start",
            "turn/interrupt",
            "thread/inject_items",
            "turn/start"
        ]
    );
}
