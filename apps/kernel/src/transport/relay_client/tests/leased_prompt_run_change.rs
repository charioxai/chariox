#![allow(unused_imports)]
use super::remote_agents::wait_for_home_remote_prompt_receipt;
use super::support::*;
use crate::session::PromptSubmissionOutcome;
use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// MP-08 / MP-09 / MP-10 / MP-11 G10: the same home prompt moves from worker run A to run B (a
/// provider relaunch after sign-in) before the home's completion reaches the worker. The worker
/// refuses the completion of run A because the prompt is still running; the home must keep the
/// turn active, and only the completion that names run B settles it, once.
#[test]
fn home_does_not_settle_a_completion_the_worker_refuses_for_a_run_change() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack(
        "home-run-change-completion",
        home_does_not_settle_a_completion_the_worker_refuses_for_a_run_change_async,
    );
}

const RUN_A: &str = "worker-run-a";
const RUN_B: &str = "worker-run-b";
const LEASED_AGENT: &str = "run-change-leased-agent";

async fn home_does_not_settle_a_completion_the_worker_refuses_for_a_run_change_async() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _test_home = RelayTestHome::new();
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("run-change-token".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("relay listener address");
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = server_shutdown_rx.await;
            })
            .await
            .expect("relay server should run");
    });

    let relay_url = format!("ws://{}:{}", addr.ip(), addr.port());
    let mut worker_config = DaemonConfig::for_tests();
    worker_config.daemon_id = "run-change-worker".to_string();
    worker_config.daemon_alias = Some("run-change-worker".to_string());
    worker_config.host_machine_id = "run-change-machine".to_string();
    let worker_private_key = worker_config.relay_private_key.clone();
    let (mut worker_socket, _) = connect_async(&relay_url)
        .await
        .expect("worker fixture should connect to relay");
    worker_socket
        .send(Message::Text(
            serde_json::to_string(&chariox_relay::protocol::RelayEnvelope::DaemonRegister {
                registration: chariox_relay::protocol::DaemonRegistration {
                    auth_token: "run-change-token".to_string(),
                    daemon_id: worker_config.daemon_id.clone(),
                    machine_id: worker_config.host_machine_id.clone(),
                    machine_alias: Some("run-change-machine".to_string()),
                    os_name: Some("Linux".to_string()),
                    kernel_started_at_ms: crate::session::unix_epoch_ms(),
                    daemon_alias: worker_config.daemon_alias.clone(),
                    kernel_alias: Some("run-change-worker".to_string()),
                    public_key: worker_config.relay_public_key.clone(),
                    capabilities: vec!["kernel_ws".to_string()],
                    available_providers: vec!["managed-dev-stub".to_string()],
                    provider_accounts: Vec::new(),
                    accepting_remote_leases: true,
                    leased_agent_count: 1,
                    local_session_count: 0,
                },
            })
            .expect("worker registration should serialize")
            .into(),
        ))
        .await
        .expect("worker registration should send");
    wait_for_daemon_registration(registry, &worker_config.daemon_id).await;

    // The run the worker's prompt is on now, and the completions the worker was asked for.
    let worker_run = Arc::new(std::sync::Mutex::new(RUN_A.to_string()));
    let completion_runs = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let (stop_worker_tx, mut stop_worker_rx) = oneshot::channel::<()>();
    let worker_task = tokio::spawn({
        let worker_run = Arc::clone(&worker_run);
        let completion_runs = Arc::clone(&completion_runs);
        async move {
            let mut home_prompt_id = String::new();
            loop {
                let message = tokio::select! {
                    _ = &mut stop_worker_rx => break,
                    message = worker_socket.next() => message,
                };
                let Some(Ok(Message::Text(text))) = message else {
                    continue;
                };
                let Ok(chariox_relay::protocol::RelayEnvelope::DaemonIncomingPeerRequest {
                    relay_request_id,
                    encrypted_request,
                    ..
                }) = serde_json::from_str::<chariox_relay::protocol::RelayEnvelope>(&text)
                else {
                    continue;
                };
                let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
                    &worker_private_key,
                    &encrypted_request,
                )
                .expect("worker should decrypt the request");
                let request = serde_json::from_slice::<RelayPeerRequest>(&decrypted.plaintext)
                    .expect("peer request should decode");
                let current_run = worker_run.lock().unwrap().clone();
                let response: Result<RelayPeerResponse, chariox_relay::protocol::RelayError> =
                    match request {
                        RelayPeerRequest::SubmitLeasedPrompt { prompt, .. } => {
                            Ok(RelayPeerResponse::LeasedPromptSubmitted {
                                provider_run_id: RUN_A.to_string(),
                                outcome: PromptSubmissionOutcome::Started {
                                    prompt: crate::session::PromptQueueItem::new(
                                        "worker-prompt",
                                        "worker-attachment",
                                        LEASED_AGENT,
                                        prompt,
                                        crate::session::PromptStatus::Running,
                                    ),
                                },
                            })
                        }
                        RelayPeerRequest::GetLeasedPromptReceipt {
                            home_prompt_id: asked,
                            ..
                        } => {
                            home_prompt_id = asked.clone();
                            Ok(RelayPeerResponse::LeasedPromptReceiptQueried {
                                receipt: Some(crate::transport::relay_peer::LeasedPromptReceipt {
                                    home_prompt_id: asked,
                                    worker_provider_run_id: current_run,
                                    phase:
                                        crate::transport::relay_peer::LeasedPromptReceiptPhase::Active,
                                    target_home_prompt_id: None,
                                    execution_lease_id: Some("run-change-lease".to_string()),
                                }),
                            })
                        }
                        RelayPeerRequest::DrainLeasedRuntimeProjection { .. } => {
                            Ok(RelayPeerResponse::LeasedRuntimeProjectionDrained {
                                event: None,
                                projection_sequence: None,
                            })
                        }
                        RelayPeerRequest::CompleteLeasedPrompt {
                            home_prompt_id: completed,
                            worker_provider_run_id,
                            ..
                        } => {
                            completion_runs
                                .lock()
                                .unwrap()
                                .push(worker_provider_run_id.clone());
                            if worker_provider_run_id == current_run {
                                Ok(RelayPeerResponse::LeasedPromptCompleted {
                                    provider_run_id: Some(current_run),
                                    provider_diagnostic: None,
                                    provider_termination: None,
                                    git_observations: Vec::new(),
                                    workspace_live_sync_change: None,
                                    completion: crate::session::PromptCompletion {
                                        completed: crate::session::PromptQueueItem::new(
                                            completed,
                                            "worker-attachment",
                                            LEASED_AGENT,
                                            "remote prompt across a run change",
                                            crate::session::PromptStatus::Completed,
                                        ),
                                        started_next: None,
                                    },
                                })
                            } else {
                                Err(chariox_relay::protocol::RelayError {
                                    code:
                                        crate::transport::relay_peer::LEASED_PROMPT_RUN_CHANGED_CODE
                                            .to_string(),
                                    message: "the prompt is still running on another provider run"
                                        .to_string(),
                                    retryable: false,
                                })
                            }
                        }
                        other => panic!("unexpected worker request: {other:?}"),
                    };
                let (encrypted_response, error) = match response {
                    Ok(response) => (
                        Some(
                            crate::transport::relay_crypto::encrypt_payload_for_peer(
                                &worker_private_key,
                                &decrypted.sender_public_key,
                                &serde_json::to_vec(&response)
                                    .expect("worker response should serialize"),
                            )
                            .expect("worker response should encrypt"),
                        ),
                        None,
                    ),
                    Err(error) => (None, Some(error)),
                };
                worker_socket
                    .send(Message::Text(
                        serde_json::to_string(
                            &chariox_relay::protocol::RelayEnvelope::DaemonIncomingPeerResponse {
                                relay_request_id,
                                encrypted_response,
                                error,
                            },
                        )
                        .expect("worker response envelope should serialize")
                        .into(),
                    ))
                    .await
                    .expect("worker response should send");
            }
            let _ = home_prompt_id;
        }
    });

    let mut home_config = DaemonConfig::for_tests();
    home_config.daemon_id = "run-change-home".to_string();
    home_config.host_machine_id = "run-change-home-machine".to_string();
    home_config.relay_url = Some(relay_url);
    home_config.relay_token = Some("run-change-token".to_string());
    home_config.relay_request_timeout_ms = 2_000;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(home_config).expect("home daemon should bootstrap"),
    ));
    let binding_on = |run: &str| crate::agent::RemoteAgentBinding {
        worker_kernel_id: worker_config.daemon_id.clone(),
        worker_machine_id: worker_config.host_machine_id.clone(),
        execution_lease_id: "run-change-lease".to_string(),
        leased_agent_id: LEASED_AGENT.to_string(),
        active_worker_provider_run_id: Some(run.to_string()),
        relay_url: None,
        relay_token: None,
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    };
    let (session_id, agent_id, attachment_id) = {
        let mut app = app.lock().await;
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-run-change",
                "worktree-run-change",
            ))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "run-change-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home attachment should be created");
        app.agents()
            .bind_remote_execution(agent.id(), binding_on(RUN_A))
            .expect("home agent should bind to worker");
        (
            session.id().to_string(),
            agent.id().to_string(),
            attachment.id().to_string(),
        )
    };

    let router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let prompt_request = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id,
        target_agent_id: Some(agent_id.clone()),
        prompt: "remote prompt across a run change".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("run-change-submit", None, None, &prompt_request);
    let LocalDaemonResponse::PromptSubmitted {
        outcome: PromptSubmissionOutcome::Started { prompt },
        ..
    } = router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("remote prompt should start")
    else {
        panic!("remote prompt submission should start");
    };
    let prompt_id = prompt.id().to_string();
    wait_for_home_remote_prompt_receipt(&app, &session_id, &agent_id, &prompt_id).await;

    // The worker relaunches the provider: the same prompt now runs on run B, which the home has
    // not followed yet.
    *worker_run.lock().unwrap() = RUN_B.to_string();
    let complete = |label: &str| {
        let request = LocalDaemonRequest::CompletePrompt(crate::local::CompletePromptRequest {
            session_id: session_id.clone(),
        });
        let command = KernelCommand::from_local_request(label, None, None, &request);
        (command, request)
    };
    let active_prompt_id = |app: Arc<Mutex<DaemonApp>>| {
        let (session_id, agent_id) = (session_id.clone(), agent_id.clone());
        async move {
            app.lock()
                .await
                .prompt_owner_active_prompt_for_agent_snapshot(&session_id, &agent_id)
                .expect("home active prompt should be readable")
                .map(|prompt| prompt.id().to_string())
        }
    };

    let (command, request) = complete("run-change-complete-stale-run");
    let refused = router.dispatch(command, request).await;
    assert!(
        refused.is_err(),
        "a completion the worker refuses for a run change is not a success: {refused:?}"
    );
    assert_eq!(
        active_prompt_id(Arc::clone(&app)).await.as_deref(),
        Some(prompt_id.as_str()),
        "the home keeps its turn active while the prompt still runs on the worker"
    );
    assert_eq!(*completion_runs.lock().unwrap(), vec![RUN_A.to_string()]);

    // The home follows the successor run; the completion that names it settles the prompt.
    app.lock()
        .await
        .agents()
        .bind_remote_execution(&agent_id, binding_on(RUN_B))
        .expect("home follows the successor run");
    let (command, request) = complete("run-change-complete-successor-run");
    let LocalDaemonResponse::PromptCompleted { completion } = router
        .dispatch(command, request)
        .await
        .expect("the completion naming the successor run settles the prompt")
    else {
        panic!("completion should return the completed prompt");
    };
    assert_eq!(completion.completed.id(), prompt_id);
    assert_eq!(active_prompt_id(Arc::clone(&app)).await, None);
    assert_eq!(
        *completion_runs.lock().unwrap(),
        vec![RUN_A.to_string(), RUN_B.to_string()],
        "the worker saw the stale run once and the successor run once"
    );

    let _ = stop_worker_tx.send(());
    worker_task.await.expect("worker task should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("relay server task should join");
}
