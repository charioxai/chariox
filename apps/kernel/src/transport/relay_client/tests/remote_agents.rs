#![allow(unused_imports)]
use super::support::*;
use crate::session::PromptSubmissionOutcome;
use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[test]
fn public_remote_completion_preserves_worker_termination() {
    run_async_with_large_test_stack(
        "public-remote-completion-worker-termination",
        public_remote_completion_preserves_worker_termination_async,
    );
}

async fn public_remote_completion_preserves_worker_termination_async() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _test_home = RelayTestHome::new();
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("termination-test-token".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener
        .local_addr()
        .expect("relay listener should have address");
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
    worker_config.daemon_id = "termination-worker".to_string();
    worker_config.daemon_alias = Some("termination-worker".to_string());
    worker_config.host_machine_id = "termination-machine".to_string();
    let worker_private_key = worker_config.relay_private_key.clone();
    let worker_public_key = worker_config.relay_public_key.clone();
    let (mut worker_socket, _) = connect_async(&relay_url)
        .await
        .expect("worker fixture should connect to relay");
    worker_socket
        .send(Message::Text(
            serde_json::to_string(&chariox_relay::protocol::RelayEnvelope::DaemonRegister {
                registration: chariox_relay::protocol::DaemonRegistration {
                    auth_token: "termination-test-token".to_string(),
                    daemon_id: worker_config.daemon_id.clone(),
                    machine_id: worker_config.host_machine_id.clone(),
                    machine_alias: Some("termination-machine".to_string()),
                    os_name: Some("Linux".to_string()),
                    kernel_started_at_ms: crate::session::unix_epoch_ms(),
                    daemon_alias: worker_config.daemon_alias.clone(),
                    kernel_alias: Some("termination-worker".to_string()),
                    public_key: worker_public_key.clone(),
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

    let (active_prompt_tx, active_prompt_rx) =
        oneshot::channel::<crate::session::PromptQueueItem>();
    let worker_response = tokio::spawn(async move {
        let (submit_relay_request_id, submit_home_public_key) = loop {
            let message = timeout(Duration::from_secs(2), worker_socket.next())
                .await
                .expect("worker should receive prompt submission request before timeout")
                .expect("worker relay socket should remain open")
                .expect("worker relay frame should decode");
            let Message::Text(text) = message else {
                continue;
            };
            let envelope = serde_json::from_str::<chariox_relay::protocol::RelayEnvelope>(&text)
                .expect("worker relay envelope should decode");
            let chariox_relay::protocol::RelayEnvelope::DaemonIncomingPeerRequest {
                relay_request_id,
                encrypted_request,
                ..
            } = envelope
            else {
                continue;
            };
            let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
                &worker_private_key,
                &encrypted_request,
            )
            .expect("worker should decrypt prompt submission request");
            let request = serde_json::from_slice::<RelayPeerRequest>(&decrypted.plaintext)
                .expect("prompt submission request should decode");
            assert!(matches!(
                request,
                RelayPeerRequest::SubmitLeasedPrompt { leased_agent_id, .. }
                    if leased_agent_id == "termination-leased-agent"
            ));
            break (relay_request_id, decrypted.sender_public_key);
        };
        let submission = RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id: "worker-provider-run".to_string(),
            outcome: PromptSubmissionOutcome::Started {
                prompt: crate::session::PromptQueueItem::new(
                    "worker-prompt",
                    "worker-attachment",
                    "termination-leased-agent",
                    "remote prompt with worker termination",
                    crate::session::PromptStatus::Running,
                ),
            },
        };
        let encrypted_submission = crate::transport::relay_crypto::encrypt_payload_for_peer(
            &worker_private_key,
            &submit_home_public_key,
            &serde_json::to_vec(&submission).expect("worker submission should serialize"),
        )
        .expect("worker submission should encrypt");
        worker_socket
            .send(Message::Text(
                serde_json::to_string(
                    &chariox_relay::protocol::RelayEnvelope::DaemonIncomingPeerResponse {
                        relay_request_id: submit_relay_request_id,
                        encrypted_response: Some(encrypted_submission),
                        error: None,
                    },
                )
                .expect("worker submission envelope should serialize")
                .into(),
            ))
            .await
            .expect("worker submission should send");

        let active_prompt = active_prompt_rx
            .await
            .expect("home should provide active prompt after submission");
        let termination = crate::provider::ProviderRunTermination::process_exit(23, 17_600);
        let (relay_request_id, home_public_key) = loop {
            let message = timeout(Duration::from_secs(2), worker_socket.next())
                .await
                .expect("worker should receive completion request before timeout")
                .expect("worker relay socket should remain open")
                .expect("worker relay frame should decode");
            let Message::Text(text) = message else {
                continue;
            };
            let envelope = serde_json::from_str::<chariox_relay::protocol::RelayEnvelope>(&text)
                .expect("worker relay envelope should decode");
            let chariox_relay::protocol::RelayEnvelope::DaemonIncomingPeerRequest {
                relay_request_id,
                encrypted_request,
                ..
            } = envelope
            else {
                continue;
            };
            let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
                &worker_private_key,
                &encrypted_request,
            )
            .expect("worker should decrypt completion request");
            let request = serde_json::from_slice::<RelayPeerRequest>(&decrypted.plaintext)
                .expect("completion request should decode");
            match request {
                RelayPeerRequest::GetLeasedPromptReceipt {
                    leased_agent_id,
                    home_prompt_id,
                } => {
                    assert_eq!(leased_agent_id, "termination-leased-agent");
                    assert_eq!(home_prompt_id, active_prompt.id());
                    let response = RelayPeerResponse::LeasedPromptReceiptQueried {
                        receipt: Some(crate::transport::relay_peer::LeasedPromptReceipt {
                            home_prompt_id,
                            worker_provider_run_id: "worker-provider-run".to_string(),
                            phase: crate::transport::relay_peer::LeasedPromptReceiptPhase::Active,
                            target_home_prompt_id: None,
                            execution_lease_id: Some("termination-lease".to_string()),
                        }),
                    };
                    let encrypted_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
                        &worker_private_key,
                        &decrypted.sender_public_key,
                        &serde_json::to_vec(&response)
                            .expect("worker receipt response should serialize"),
                    )
                    .expect("worker receipt response should encrypt");
                    worker_socket
                        .send(Message::Text(
                            serde_json::to_string(
                                &chariox_relay::protocol::RelayEnvelope::DaemonIncomingPeerResponse {
                                    relay_request_id,
                                    encrypted_response: Some(encrypted_response),
                                    error: None,
                                },
                            )
                            .expect("worker receipt envelope should serialize")
                            .into(),
                        ))
                        .await
                        .expect("worker receipt response should send");
                }
                RelayPeerRequest::DrainLeasedRuntimeProjection {
                    leased_agent_id,
                    provider_run_id,
                    ..
                } => {
                    assert_eq!(leased_agent_id, "termination-leased-agent");
                    assert_eq!(provider_run_id, "worker-provider-run");
                    let response = RelayPeerResponse::LeasedRuntimeProjectionDrained {
                        event: None,
                    };
                    let encrypted_response =
                        crate::transport::relay_crypto::encrypt_payload_for_peer(
                            &worker_private_key,
                            &decrypted.sender_public_key,
                            &serde_json::to_vec(&response)
                                .expect("empty projection response should serialize"),
                        )
                        .expect("empty projection response should encrypt");
                    worker_socket
                        .send(Message::Text(
                            serde_json::to_string(
                                &chariox_relay::protocol::RelayEnvelope::DaemonIncomingPeerResponse {
                                    relay_request_id,
                                    encrypted_response: Some(encrypted_response),
                                    error: None,
                                },
                            )
                            .expect("empty projection envelope should serialize")
                            .into(),
                        ))
                        .await
                        .expect("empty projection response should send");
                }
                RelayPeerRequest::CompleteLeasedPrompt { leased_agent_id } => {
                    assert_eq!(leased_agent_id, "termination-leased-agent");
                    break (relay_request_id, decrypted.sender_public_key);
                }
                other => panic!("unexpected remote completion request: {other:?}"),
            }
        };
        let response = RelayPeerResponse::LeasedPromptCompleted {
            provider_run_id: Some("worker-provider-run".to_string()),
            provider_diagnostic: None,
            provider_termination: Some(termination),
            git_observations: Vec::new(),
            workspace_live_sync_change: None,
            completion: crate::session::PromptCompletion {
                completed: active_prompt,
                started_next: None,
            },
        };
        let encrypted_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
            &worker_private_key,
            &home_public_key,
            &serde_json::to_vec(&response).expect("worker response should serialize"),
        )
        .expect("worker response should encrypt");
        worker_socket
            .send(Message::Text(
                serde_json::to_string(
                    &chariox_relay::protocol::RelayEnvelope::DaemonIncomingPeerResponse {
                        relay_request_id,
                        encrypted_response: Some(encrypted_response),
                        error: None,
                    },
                )
                .expect("worker response envelope should serialize")
                .into(),
            ))
            .await
            .expect("worker response should send");
        let _ = worker_socket.close(None).await;
    });
    tokio::task::yield_now().await;

    let mut home_config = DaemonConfig::for_tests();
    home_config.daemon_id = "termination-home".to_string();
    home_config.host_machine_id = "termination-home-machine".to_string();
    home_config.relay_url = Some(relay_url);
    home_config.relay_token = Some("termination-test-token".to_string());
    home_config.relay_request_timeout_ms = 2_000;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(home_config).expect("home daemon should bootstrap"),
    ));
    let (session_id, agent_id, attachment_id) = {
        let mut app = app.lock().await;
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                "workspace-remote-termination",
                "worktree-remote-termination",
            ))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "remote-termination-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home attachment should be created");
        app.agents()
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: worker_config.daemon_id.clone(),
                    worker_machine_id: worker_config.host_machine_id.clone(),
                    execution_lease_id: "termination-lease".to_string(),
                    leased_agent_id: "termination-leased-agent".to_string(),
                    active_worker_provider_run_id: Some("worker-provider-run".to_string()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("home agent should bind to worker");
        (
            session.id().to_string(),
            agent.id().to_string(),
            attachment.id().to_string(),
        )
    };

    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::clone(&app),
        1,
    );
    let prompt_request = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(agent_id.clone()),
        prompt: "remote prompt with worker termination".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command = KernelCommand::from_local_request(
        "public-remote-completion-worker-termination-submit",
        None,
        None,
        &prompt_request,
    );
    let LocalDaemonResponse::PromptSubmitted {
        outcome: PromptSubmissionOutcome::Started { prompt },
        ..
    } = router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("remote prompt should start through the public kernel runtime")
    else {
        panic!("public remote prompt submission should start");
    };
    let prompt_id = prompt.id().to_string();
    let active_prompt = wait_for_home_remote_prompt_receipt(
        &app,
        &session_id,
        &agent_id,
        &prompt_id,
    )
    .await;

    active_prompt_tx
        .send(active_prompt)
        .expect("worker should wait for the home prompt before completion");

    let completion_request = LocalDaemonRequest::CompletePrompt(crate::local::CompletePromptRequest {
        session_id: session_id.clone(),
    });
    let completion_command = KernelCommand::from_local_request(
        "public-remote-completion-worker-termination-complete",
        None,
        None,
        &completion_request,
    );
    let LocalDaemonResponse::PromptCompleted { completion } = router
        .dispatch(completion_command, completion_request)
        .await
        .expect("public remote completion should succeed")
    else {
        panic!("public remote completion should return a completed prompt");
    };
    worker_response
        .await
        .expect("worker response task should join");
    assert_eq!(completion.completed.id(), prompt_id);
    let projected = app
        .lock()
        .await
        .completed_git_turn_snapshot_store()
        .latest_projection_for_agent(&session_id, &agent_id)
        .expect("remote completion should be projected");
    assert_eq!(
        projected.provider_termination,
        Some(crate::provider::ProviderRunTermination::process_exit(
            23, 17_600
        )),
        "normal public completion must retain the worker-reported termination"
    );
    assert_eq!(
        projected.settlement_status,
        crate::git_observer::CompletedTurnSettlementStatus::Failed,
        "worker-reported termination must not project a successful turn"
    );
    assert_eq!(
        app.lock()
            .await
            .agents()
            .get_agent(&agent_id)
            .expect("remote agent should remain available")
            .state(),
        crate::agent::AgentState::Error,
    );

    let _ = server_shutdown_tx.send(());
    server_task.await.expect("relay server task should join");
}

async fn wait_for_home_remote_prompt_receipt(
    app: &Arc<Mutex<DaemonApp>>,
    session_id: &str,
    agent_id: &str,
    prompt_id: &str,
) -> crate::session::PromptQueueItem {
    for _ in 0..400 {
        let delivered_prompt = {
            let app = app.lock().await;
            let active_prompt = app
                .prompt_owner_active_prompt_for_agent_snapshot(session_id, agent_id)
                .expect("home active remote prompt should be readable");
            let active_worker_provider_run_id = app
                .agents()
                .get_agent(agent_id)
                .expect("home remote agent should remain available")
                .remote_execution()
                .and_then(|binding| binding.active_worker_provider_run_id.clone());
            active_prompt.filter(|prompt| {
                prompt.id() == prompt_id
                    && prompt.durable_delivery_phase()
                        == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                    && prompt.durable_delivery_provider_run_id()
                        == active_worker_provider_run_id.as_deref()
                    && active_worker_provider_run_id.is_some()
            })
        };
        if let Some(prompt) = delivered_prompt {
            return prompt;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "home prompt `{prompt_id}` did not receive its exact durable worker-run receipt for agent `{agent_id}`"
    );
}

#[test]
fn agents_can_be_spawned_on_a_remote_machine_and_cleaned_up() {
    run_async_with_large_test_stack(
        "remote-agents-spawn-resize-cleanup",
        agents_can_be_spawned_on_a_remote_machine_and_cleaned_up_async,
    );
}

async fn agents_can_be_spawned_on_a_remote_machine_and_cleaned_up_async() {
    let _relay_test_guard = relay_client_test_guard().await;
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let server = Arc::new(RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    }));
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_home = DaemonConfig::for_tests();
    config_home.daemon_id = "daemon-home".to_string();
    config_home.daemon_alias = Some("home".to_string());
    config_home.host_machine_id = "machine-home".to_string();
    config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;
    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
    ));
    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_home),
        Arc::clone(&state_home),
        shutdown_home_rx,
    ));
    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-worker".to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.accept_remote_leases = true;
    config_worker.provider_runtime_init_delay_ms = 1_000;
    let app_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_worker),
        Arc::clone(&state_worker),
        shutdown_worker_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let worker_kernels =
        relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
            .await
            .expect("worker kernels should be discoverable");
    let provider = worker_kernels
        .first()
        .and_then(|kernel| {
            kernel
                .available_providers
                .iter()
                .find(|provider| provider.as_str() == "managed-dev-stub")
        })
        .cloned()
        .expect("worker should advertise managed-dev-stub");
    refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
        .await
        .expect("home remote inventory should refresh");

    let session_id = {
        let mut app = app_home.lock().await;
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        session.id().to_string()
    };

    let remote_agent = {
        let mut app = app_home.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("remote-reviewer")
                    .with_model("default")
                    .with_effort("medium")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn")
    };

    let remote_execution = remote_agent
        .remote_execution()
        .cloned()
        .expect("remote binding should be present");
    assert_eq!(remote_execution.worker_kernel_id, config_worker.daemon_id);
    assert_eq!(
        remote_execution.worker_machine_id,
        config_worker.host_machine_id
    );

    {
        let mut app = app_worker.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 1);
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 1);
        let worker_agents = app.agents().list_agents();
        assert_eq!(
            worker_agents
                .iter()
                .filter(|agent| agent.is_metaagent())
                .count(),
            0,
            "remote agents start regular until /meta activates temporary meta mode"
        );
    }

    let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
        &config_home,
        ClientTarget {
            daemon_id: Some(config_worker.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::UpdateLeasedAgentMetaMode {
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            active: true,
        },
    )
    .await
    .expect("remote meta mode activation should be sent");
    assert!(matches!(
        response,
        RelayPeerResponse::LeasedAgentMetaModeUpdated { .. }
    ));

    {
        let app = app_worker.lock().await;
        let worker_agents = app.agents().list_agents();
        assert_eq!(
            worker_agents
                .iter()
                .filter(|agent| agent.is_metaagent())
                .count(),
            1,
            "remote meta mode update should activate the backing agent"
        );
    }

    let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
        &config_home,
        ClientTarget {
            daemon_id: Some(config_worker.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::UpdateLeasedAgentMetaMode {
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            active: false,
        },
    )
    .await
    .expect("remote meta mode deactivation should be sent");
    assert!(matches!(
        response,
        RelayPeerResponse::LeasedAgentMetaModeUpdated { .. }
    ));

    {
        let app = app_worker.lock().await;
        let worker_agents = app.agents().list_agents();
        assert_eq!(
            worker_agents
                .iter()
                .filter(|agent| agent.is_metaagent())
                .count(),
            0,
            "remote meta mode deactivation should restore the backing agent"
        );
    }

    Box::pin(assert_remote_native_terminal_resize(
        &app_home,
        &app_worker,
        &session_id,
        &provider,
        &remote_agent,
    ))
    .await;

    {
        let mut app = app_home.lock().await;
        let destroyed = crate::app::KernelSessionService::new(&mut app)
            .destroy_agent(remote_agent.id())
            .expect("remote agent should destroy");
        assert_eq!(destroyed.id(), remote_agent.id());
    }

    {
        let mut app = app_worker.lock().await;
        assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 0);
        assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 0);
    }

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

async fn assert_remote_native_terminal_resize(
    app_home: &Arc<Mutex<DaemonApp>>,
    app_worker: &Arc<Mutex<DaemonApp>>,
    session_id: &str,
    provider: &str,
    remote_agent: &crate::agent::AgentInstance,
) {
    let home_router =
        crate::runtime::router::CommandRouter::with_interactive_capacity(Arc::clone(app_home), 8);
    let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
        session_id: session_id.to_string(),
        agent_id: Some(remote_agent.id().to_string()),
        adapter_key: crate::provider::adapter_key_for_provider(provider).to_string(),
        provider: provider.to_string(),
        account_profile: "default".to_string(),
        model: "default".to_string(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: true,
    });
    let launch_command = KernelCommand::from_local_request(
        "launch-remote-native-resize",
        None,
        None,
        &launch_request,
    );
    let home_provider_run = match home_router
        .dispatch(launch_command, launch_request)
        .await
        .expect("remote native provider should launch")
    {
        LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
        other => panic!("unexpected provider launch response: {other:?}"),
    };
    let worker_provider_run_id = {
        let app = app_home.lock().await;
        app.agents()
            .get_agent(remote_agent.id())
            .expect("remote agent should remain available")
            .remote_execution()
            .and_then(|binding| binding.active_worker_provider_run_id.clone())
            .expect("worker provider run should be projected")
    };
    {
        let app = app_worker.lock().await;
        let worker_run = app
            .providers()
            .get_run(&worker_provider_run_id)
            .expect("worker provider run should exist");
        assert_eq!(
            worker_run.state(),
            crate::provider::ProviderRunState::Starting,
            "remote native launch must return before slow provider initialization completes"
        );
    }
    let resize_request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
        session_id: session_id.to_string(),
        provider_run_id: Some(home_provider_run.id().to_string()),
        cols: 83,
        rows: 27,
    });
    let resize_command =
        KernelCommand::from_local_request("resize-remote-native", None, None, &resize_request);
    assert!(matches!(
        home_router
            .dispatch(resize_command, resize_request)
            .await
            .expect("remote native provider terminal should resize"),
        LocalDaemonResponse::TerminalResized {
            cols: 83,
            rows: 27,
            ..
        }
    ));
    let app = app_worker.lock().await;
    assert_eq!(app.pty().size(&worker_provider_run_id), Some((83, 27)));
}

#[test]
fn remote_machine_agents_execute_prompts_through_the_home_session() {
    run_async_with_large_test_stack("remote-agents-execute-prompts", || {
        remote_machine_agents_execute_prompts_through_the_home_session_async(
            false, false, false, false, false, false,
        )
    });
}

#[test]
fn remote_agent_message_steers_live_worker_without_a_user_queue() {
    // Fixed worker IDs must not retain the previous fixture's generated trust key.
    for _ in 0..2 {
        run_async_with_large_test_stack("remote-agent-direct-message", || {
            remote_machine_agents_execute_prompts_through_the_home_session_async(
                true, false, false, false, false, false,
            )
        });
    }
}

#[test]
fn remote_agent_message_steer_keeps_home_app_lock_available_while_worker_reply_waits() {
    run_async_with_large_test_stack("remote-agent-message-home-lock", || {
        remote_machine_agents_execute_prompts_through_the_home_session_async(
            true, true, false, false, false, false,
        )
    });
}

#[test]
fn remote_agent_message_reply_after_target_completion_does_not_commit_stale_steer() {
    run_async_with_large_test_stack("remote-agent-message-stale-reply", || {
        remote_machine_agents_execute_prompts_through_the_home_session_async(
            true, true, false, true, false, false,
        )
    });
}

#[test]
fn remote_queued_prompt_steer_reserves_queue_head_and_keeps_home_lock_available_while_worker_reply_waits(
) {
    run_async_with_large_test_stack("remote-queued-steer-home-lock", || {
        remote_machine_agents_execute_prompts_through_the_home_session_async(
            false, false, true, false, false, false,
        )
    });
}

#[test]
fn failed_remote_queued_prompt_steer_advances_after_concurrent_completion() {
    run_async_with_large_test_stack("remote-queued-steer-failed-advance", || {
        remote_machine_agents_execute_prompts_through_the_home_session_async(
            false, false, true, false, true, false,
        )
    });
}

#[test]
fn ambiguous_remote_queued_steer_reply_reconciles_exact_receipt_without_replay() {
    run_async_with_large_test_stack("remote-queued-steer-uncertain", || {
        remote_machine_agents_execute_prompts_through_the_home_session_async(
            false, false, true, false, false, true,
        )
    });
}

async fn remote_machine_agents_execute_prompts_through_the_home_session_async(
    direct_message_only: bool,
    hold_agent_message_reply: bool,
    hold_queued_prompt_reply: bool,
    finish_target_before_direct_reply: bool,
    fail_queued_prompt_reply: bool,
    lose_queued_prompt_reply: bool,
) {
    let _relay_test_guard = relay_client_test_guard().await;
    let _test_home = RelayTestHome::new();
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let server = Arc::new(RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    }));
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_home = DaemonConfig::for_tests();
    config_home.daemon_id = if direct_message_only {
        "daemon-home-direct-message"
    } else {
        "daemon-home"
    }
    .to_string();
    config_home.daemon_alias = Some("home".to_string());
    config_home.host_machine_id = "machine-home".to_string();
    config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;
    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = if direct_message_only {
        "daemon-worker-direct-message"
    } else {
        "daemon-worker"
    }
    .to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.accept_remote_leases = true;
    let app_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_worker),
        Arc::clone(&state_worker),
        shutdown_worker_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
        .await
        .expect("worker kernels should be discoverable")
        .first()
        .and_then(|kernel| {
            kernel
                .available_providers
                .iter()
                .find(|provider| provider.as_str() == "managed-dev-stub")
        })
        .cloned()
        .expect("worker should advertise managed-dev-stub");

    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
    ));
    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_home),
        Arc::clone(&state_home),
        shutdown_home_rx,
    ));
    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
        .await
        .expect("home remote inventory should refresh");

    let (session_id, attachment_id, steering_attachment_id, local_provider_run_id) = {
        let mut app_home = app_home.lock().await;
        let (session, local_agent) = crate::app::KernelSessionService::new(&mut app_home)
            .create_session(CreateSessionRequest::new(
                "workspace-home",
                std::env::temp_dir().to_string_lossy(),
            ))
            .expect("home session should be created");
        let local_provider_run = app_home
            .launch_provider(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "native-tui-idle",
                )
                .with_agent_id(local_agent.id().to_string()),
            )
            .expect("home provider should launch to force a cross-kernel run-id collision");
        let attachment = crate::app::KernelSessionService::new(&mut app_home)
            .attach(AttachRequest::new(
                session.id(),
                "home-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home attachment should attach");
        let steering_attachment = crate::app::KernelSessionService::new(&mut app_home)
            .attach(AttachRequest::new(
                session.id(),
                "home-steering-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home steering attachment should attach");
        (
            session.id().to_string(),
            attachment.id().to_string(),
            steering_attachment.id().to_string(),
            local_provider_run.id().to_string(),
        )
    };

    let remote_agent_id = {
        let mut app_home = app_home.lock().await;
        crate::app::KernelSessionService::new(&mut app_home)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("remote-reviewer")
                    .with_model("native-tui-idle")
                    .with_effort("medium")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn")
            .id()
            .to_string()
    };

    let leased_agent_id = app_home
        .lock()
        .await
        .agents()
        .get_agent(&remote_agent_id)
        .expect("remote agent should still exist")
        .remote_execution()
        .expect("remote binding should still exist")
        .leased_agent_id
        .clone();
    assert_eq!(
        state_worker
            .read()
            .await
            .peer_public_key(&config_home.daemon_id)
            .as_deref(),
        Some(config_home.relay_public_key.as_str()),
        "worker should retain the authenticated home key for projection events"
    );

    let _ = server_shutdown_tx.send(());
    server_task.await.expect("relay accept loop should stop");

    let router = Arc::new(
        crate::runtime::router::CommandRouter::with_interactive_capacity(Arc::clone(&app_home), 1),
    );
    let prompt_request = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(remote_agent_id.clone()),
        prompt: "remote prompt over home session\n".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command = KernelCommand::from_local_request(
        "command-remote-persistent-prompt",
        None,
        None,
        &prompt_request,
    );
    let prompt_response = router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("remote prompt should submit while new relay connections are unavailable");
    assert!(matches!(
        prompt_response,
        LocalDaemonResponse::PromptSubmitted {
            outcome: crate::session::PromptSubmissionOutcome::Started { .. },
            ..
        }
    ));

    let mut worker_received_prompt = false;
    for _ in 0..1200 {
        worker_received_prompt = {
            let mut app = app_worker.lock().await;
            let leased_agent = RemoteLeaseRuntime::new(&mut app)
                .leased_agent_snapshot_for_test(&leased_agent_id)
                .expect("worker leased agent should remain available");
            leased_agent.active_home_prompt_id.as_deref() == Some("prompt-1")
                && app
                    .prompt_owner_queued_prompt_count_for_agent(
                        &leased_agent.backing_session_id,
                        &leased_agent.backing_agent_id,
                    )
                    .expect("worker queue count should load")
                    == 0
        };
        if worker_received_prompt {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }

    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should restart");
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should resume accepting connections");
        })
    };
    assert!(
        worker_received_prompt,
        "remote prompt must reuse the persistent relay lane when new relay connections are unavailable"
    );

    let mut home_provider_running = false;
    for _ in 0..400 {
        home_provider_running = {
            let app = app_home.lock().await;
            let worker_acknowledged = app
                .agents()
                .get_agent(&remote_agent_id)
                .expect("remote agent should remain available")
                .remote_execution()
                .and_then(|remote| remote.active_worker_provider_run_id.as_deref())
                .is_some();
            let projected_running = app
                .provider_run_projection_store()
                .get_for_agent(&session_id, &remote_agent_id)
                .is_some_and(|run| run.state() == crate::provider::ProviderRunState::Running);
            worker_acknowledged && projected_running
        };
        if home_provider_running {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }
    let home_projection_debug = {
        let app = app_home.lock().await;
        (
            app.agents()
                .get_agent(&remote_agent_id)
                .ok()
                .and_then(|agent| {
                    agent
                        .remote_execution()
                        .and_then(|remote| remote.active_worker_provider_run_id.clone())
                }),
            app.provider_run_projection_store()
                .get_for_agent(&session_id, &remote_agent_id)
                .map(|run| (run.id().to_string(), run.state())),
        )
    };
    assert!(
        home_provider_running,
        "home agent must project the running worker provider run before follow-up steering: {home_projection_debug:?}"
    );
    let (worker_provider_run_id, projected_provider_run_id) = {
        let app = app_home.lock().await;
        let worker_provider_run_id = app
            .agents()
            .get_agent(&remote_agent_id)
            .expect("remote agent should remain available")
            .remote_execution()
            .and_then(|remote| remote.active_worker_provider_run_id.clone())
            .expect("worker provider run should remain projected");
        let projected_provider_run_id = app
            .provider_run_projection_store()
            .get_for_agent(&session_id, &remote_agent_id)
            .expect("remote projected provider run should exist")
            .id()
            .to_string();
        (worker_provider_run_id, projected_provider_run_id)
    };
    assert_eq!(
        worker_provider_run_id, local_provider_run_id,
        "the regression requires raw provider-run IDs to collide across kernels"
    );
    assert_ne!(projected_provider_run_id, local_provider_run_id);

    let mut held_reply_observation = None;
    if direct_message_only {
        let sender_prompt_id = {
            let mut app = app_home.lock().await;
            let sender_id = app
                .providers()
                .get_run(&local_provider_run_id)
                .expect("home sender run should remain available")
                .agent_instance_id()
                .expect("home sender run should have an agent")
                .to_string();
            let sender_prompt = crate::session::PromptQueueItem::new(
                app.sessions_mut().reserve_prompt_id(),
                &attachment_id,
                &sender_id,
                "sender task",
                crate::session::PromptStatus::Queued,
            );
            let crate::session::PromptSubmissionOutcome::Started { prompt } = app
                .prompt_owner_submit_prepared_prompt(&session_id, sender_prompt, false)
                .expect("home sender turn should start")
            else {
                panic!("home sender turn must start");
            };
            prompt.id().to_string()
        };
        let sender_token = app_home
            .lock()
            .await
            .providers()
            .get_run(&local_provider_run_id)
            .expect("home sender provider run should remain available")
            .runtime_mcp_auth_token()
            .expect("home sender should have runtime MCP auth")
            .to_string();
        let direct_message_args = serde_json::json!({
            "agent": remote_agent_id,
            "message": "REMOTE_AGENT_MESSAGE_DIRECT",
            "origin_prompt_id": sender_prompt_id,
            "idempotency_key": "remote-direct-message",
        });
        let direct_message = if hold_agent_message_reply {
            let pending_before = registry.read().await.pending_request_count();
            // Keep the worker from producing a reply until the relay has recorded the steer.
            let worker_app_guard = app_worker.lock().await;
            let dispatch_router = Arc::clone(&router);
            let dispatch_sender_token = sender_token.clone();
            let dispatch_args = direct_message_args.clone();
            let dispatch = tokio::spawn(async move {
                dispatch_router
                    .runtime_state()
                    .dispatch_authenticated_runtime_tool_call(
                        &dispatch_sender_token,
                        crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
                        dispatch_args,
                    )
                    .await
            });
            let relay_reply_is_waiting = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if registry.read().await.pending_request_count() > pending_before {
                        break;
                    }
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .is_ok();
            let mut home_app_lock_available = false;
            for _ in 0..20 {
                if let Ok(guard) = app_home.try_lock() {
                    home_app_lock_available = true;
                    drop(guard);
                }
                sleep(Duration::from_millis(10)).await;
            }
            if finish_target_before_direct_reply {
                let mut home = app_home
                    .try_lock()
                    .expect("the home lock should be free while the worker reply is held");
                home.prompt_owner_complete_active_prompt_only(&session_id, &remote_agent_id)
                    .expect("target turn should complete before the steer reply");
                assert!(home
                    .prompt_owner_active_prompt_for_agent(&session_id, &remote_agent_id)
                    .expect("home active prompt should load")
                    .is_none());
            }
            drop(worker_app_guard);
            let direct_message_result = dispatch
                .await
                .expect("agent message dispatch task should join");
            held_reply_observation = Some((relay_reply_is_waiting, home_app_lock_available));
            if finish_target_before_direct_reply {
                assert!(
                    !direct_message_result.as_ref().is_ok_and(|result| result.ok),
                    "a reply for a completed target turn must not commit as a current steer"
                );
                let home = app_home.lock().await;
                assert!(!home.terminal().output_records().iter().any(|record| {
                    record.kind == crate::terminal::TerminalOutputKind::PromptEcho
                        && String::from_utf8_lossy(&record.bytes)
                            .contains("REMOTE_AGENT_MESSAGE_DIRECT")
                }));
                assert!(!home
                    .operational_history_store()
                    .load_session_events(&session_id, Some(&remote_agent_id))
                    .expect("home history should load")
                    .iter()
                    .any(|event| event.content.as_deref().is_some_and(|content| {
                        content.contains("REMOTE_AGENT_MESSAGE_DIRECT")
                    })));
                drop(home);
                let _ = shutdown_home_tx.send(true);
                let _ = shutdown_worker_tx.send(true);
                connector_home.await.expect("home connector should join");
                connector_worker
                    .await
                    .expect("worker connector should join");
                let _ = server_shutdown_tx.send(());
                server_task.await.expect("server task should join");
                return;
            }
            direct_message_result.expect("agent message should steer to the leased worker")
        } else {
            router
                .runtime_state()
                .dispatch_authenticated_runtime_tool_call(
                    &sender_token,
                    crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
                    direct_message_args.clone(),
                )
                .await
                .expect("agent message should steer to the leased worker")
        };
        assert!(direct_message.ok, "{:?}", direct_message.payload);
        assert_eq!(direct_message.payload["status"], "steered");
        assert_eq!(
            app_home
                .lock()
                .await
                .prompt_owner_queued_prompt_count_for_agent(&session_id, &remote_agent_id)
                .expect("home prompt queue should load"),
            0,
            "remote agent messages must not become queued user prompts"
        );
        let repeated_message = router
            .runtime_state()
            .dispatch_authenticated_runtime_tool_call(
                &sender_token,
                crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
                direct_message_args,
            )
            .await
            .expect("idempotent agent message retry should replay");
        assert_eq!(
            repeated_message.payload["prompt_id"],
            direct_message.payload["prompt_id"]
        );
        let mut direct_delivery_count = 0;
        for _ in 0..80 {
            direct_delivery_count = app_worker
                .lock()
                .await
                .terminal()
                .input_records()
                .iter()
                .filter(|record| {
                    String::from_utf8_lossy(&record.bytes).contains("REMOTE_AGENT_MESSAGE_DIRECT")
                })
                .count();
            if direct_delivery_count > 0 {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(
            direct_delivery_count, 1,
            "leased worker must receive one direct message"
        );
        let direct_echoes = app_home
            .lock()
            .await
            .terminal()
            .output_records()
            .iter()
            .filter(|record| {
                record.kind == crate::terminal::TerminalOutputKind::PromptEcho
                    && String::from_utf8_lossy(&record.bytes)
                        .contains("REMOTE_AGENT_MESSAGE_DIRECT")
            })
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(direct_echoes.len(), 1, "direct message should echo once");
        assert_eq!(
            direct_echoes[0].provider_run_id, projected_provider_run_id,
            "the echo must identify the home-projected leased run, not the colliding local run"
        );
        assert_eq!(
            direct_echoes[0].agent_id.as_deref(),
            Some(remote_agent_id.as_str())
        );
        let _ = shutdown_home_tx.send(true);
        let _ = shutdown_worker_tx.send(true);
        connector_home.await.expect("home connector should join");
        connector_worker
            .await
            .expect("worker connector should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
        return;
    }

    {
        let mut app = app_worker.lock().await;
        let leased_agent = RemoteLeaseRuntime::new(&mut app)
            .leased_agent_snapshot_for_test(&leased_agent_id)
            .expect("worker leased agent should exist");
        let backing_active = app
            .prompt_owner_active_prompt_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("worker active prompt should load");
        assert_eq!(
            leased_agent.active_home_prompt_id.as_deref(),
            Some("prompt-1"),
            "worker backing active prompt: {backing_active:?}"
        );
        assert_eq!(
            app.prompt_owner_queued_prompt_count_for_agent(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
            )
            .expect("worker queue count should load"),
            0,
            "provider launch promotion must not leave a duplicate queued prompt"
        );
    }

    let queued_prompt_id = match app_home
        .lock()
        .await
        .submit_prompt(
            &session_id,
            &attachment_id,
            Some(&remote_agent_id),
            "REMOTE_QUEUE_STEER_DELIVERY\n",
            Vec::new(),
        )
        .expect("second remote prompt should queue")
    {
        crate::session::PromptSubmissionOutcome::Queued { prompt, .. } => prompt.id().to_string(),
        other => panic!("unexpected queued prompt outcome: {other:?}"),
    };
    let request = LocalDaemonRequest::SteerQueuedPrompt(crate::local::SteerQueuedPromptRequest {
        session_id: session_id.clone(),
        attachment_id: steering_attachment_id,
        target_agent_id: remote_agent_id.clone(),
        prompt_id: queued_prompt_id.clone(),
    });
    let command =
        KernelCommand::from_local_request("command-remote-queued-steer", None, None, &request);
    let response = if hold_queued_prompt_reply {
        let pending_before = registry.read().await.pending_request_count();
        let mut worker_app_guard = app_worker.lock().await;
        if lose_queued_prompt_reply {
            state_worker
                .write()
                .await
                .test_lose_next_peer_response_payload();
        }
        let dispatch_router = Arc::clone(&router);
        let dispatch =
            tokio::spawn(async move { dispatch_router.dispatch(command, request).await });
        let relay_reply_is_waiting = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if registry.read().await.pending_request_count() > pending_before {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok();
        let mut home_app_lock_available = false;
        for _ in 0..20 {
            if let Ok(guard) = app_home.try_lock() {
                home_app_lock_available = true;
                drop(guard);
            }
            sleep(Duration::from_millis(10)).await;
        }
        let mut queue_advance_deferred = false;
        if relay_reply_is_waiting && home_app_lock_available {
            let mut app = app_home
                .try_lock()
                .expect("home app lock should remain available during held worker reply");
            let queue_head = app
                .prompt_owner_peek_next_queued_prompt(&session_id, &remote_agent_id)
                .expect("home queue head should load");
            assert!(
                queue_head.is_none(),
                "ordinary queue peeks must hide the reserved head while its relay reply is pending"
            );
            assert_eq!(
                app.prompt_owner_queued_prompt_count_for_agent(&session_id, &remote_agent_id)
                    .expect("home queued prompt count should load"),
                1,
                "the reserved prompt must remain queued until the relay response commits"
            );
            app.prompt_owner_complete_active_prompt_only(&session_id, &remote_agent_id)
                .expect("the active turn should be able to complete while steer reply waits");
            let completion_path_activation = app
                .prompt_owner_activate_next_queued_prompt_with_prompt_id(
                    &session_id,
                    &remote_agent_id,
                    Some(&queued_prompt_id),
                    "prompt-should-remain-queued".to_string(),
                )
                .expect("remote completion activation should inspect the reserved queue head");
            queue_advance_deferred = completion_path_activation.is_none();
            let next = crate::app::KernelAgentService::new(&mut app)
                .admit_next_queued_remote_prompt(&session_id, &remote_agent_id, None)
                .expect("ordinary completion advancement should inspect the queue");
            queue_advance_deferred &= next.is_none();
            if lose_queued_prompt_reply {
                let snapshot = app
                    .sessions()
                    .get_session(&session_id)
                    .expect("home session should snapshot while worker receipt is pending");
                let queued = snapshot
                    .queued_prompts_for_agent(&remote_agent_id)
                    .and_then(|prompts| prompts.front())
                    .expect("uncertain steer must remain in the home queue");
                let uncertainty = queued
                    .remote_steer_outcome_uncertainty()
                    .expect("send intent must be uncertain before the relay response arrives");
                assert_eq!(queued.id(), queued_prompt_id);
                assert_eq!(uncertainty.0, "prompt-1");
                assert_eq!(
                    uncertainty.1.as_deref(),
                    Some(worker_provider_run_id.as_str())
                );

                let durable = crate::durable_prompt_state::DurablePromptStateEventPayload::capture(
                    &snapshot,
                    &remote_agent_id,
                );
                let encoded = serde_json::to_vec(&durable)
                    .expect("pre-send uncertainty should serialize into durable prompt state");
                let mut restored = serde_json::from_slice::<
                    crate::durable_prompt_state::DurablePromptStateEventPayload,
                >(&encoded)
                .expect("pre-send durable prompt state should deserialize");
                restored.restore_private_states();
                let restored_prompt = restored
                    .queued_prompts
                    .front()
                    .expect("pre-send durable state must retain the exact queued prompt");
                assert_eq!(restored_prompt.id(), queued_prompt_id);
                assert_eq!(
                    restored_prompt.remote_steer_outcome_uncertainty(),
                    Some(uncertainty),
                    "durable state must identify the worker run before any reply can be lost"
                );
                let public_session = serde_json::to_vec(&snapshot)
                    .expect("home prompt state should serialize for restart");
                let mut restarted_session =
                    serde_json::from_slice::<crate::session::RuntimeSession>(&public_session)
                        .expect("home prompt state should deserialize for restart");
                restarted_session.restore_durable_prompt_private_states(&durable.private_states);
                let restarted_prompt_owner =
                    crate::runtime::prompt_state::PromptStateOwner::default();
                assert!(
                    restarted_prompt_owner
                        .peek_next_queued_prompt(&restarted_session, &remote_agent_id)
                        .is_none(),
                    "restart must not promote an uncertain queued steer"
                );
                assert!(restarted_prompt_owner
                    .activate_next_queued_prompt_with_prompt_id(
                        &restarted_session,
                        &remote_agent_id,
                        Some(&queued_prompt_id),
                        "restart-must-not-promote-uncertain-steer".to_string(),
                    )
                    .expect("restored uncertain queue should remain fail closed")
                    .is_none());
            }
        }
        if fail_queued_prompt_reply {
            assert!(
                relay_reply_is_waiting,
                "the worker request must be held before injecting its rejection"
            );
            let leased_agent = RemoteLeaseRuntime::new(&mut worker_app_guard)
                .leased_agent_snapshot_for_test(&leased_agent_id)
                .expect("worker leased agent should remain available");
            worker_app_guard
                .providers_mut()
                .terminate_run_provider_only(
                    &leased_agent.backing_session_id,
                    &worker_provider_run_id,
                )
                .expect("the held worker run should terminate for explicit rejection");
            assert_eq!(
                worker_app_guard
                    .providers()
                    .get_run(&worker_provider_run_id)
                    .expect("terminated worker run should remain recorded")
                    .state(),
                crate::provider::ProviderRunState::Ended
            );
        }
        drop(worker_app_guard);
        let dispatch_result = dispatch
            .await
            .expect("queued steer dispatch task should join");
        assert!(
            relay_reply_is_waiting,
            "worker reply should be held by its app lock"
        );
        assert!(
            home_app_lock_available,
            "home app lock must remain available during the relay round trip"
        );
        assert!(
            queue_advance_deferred,
            "ordinary completion must leave the reserved queued prompt for the steer reply"
        );
        if fail_queued_prompt_reply {
            let rejection = match dispatch_result {
                Err(DaemonError::RelayTransport {
                    operation: "read temporary relay peer response",
                    code,
                    retryable: false,
                    ..
                }) => code,
                other => panic!("expected a terminal explicit worker rejection, got {other:?}"),
            };
            assert_eq!(
                rejection, "no_active_provider_run",
                "the worker must return its validated terminal rejection code"
            );
            let mut worker_queue_count = 0;
            for _ in 0..80 {
                worker_queue_count = {
                    let mut worker = app_worker.lock().await;
                    let leased_agent = RemoteLeaseRuntime::new(&mut worker)
                        .leased_agent_snapshot_for_test(&leased_agent_id)
                        .expect("worker leased agent should remain available");
                    worker
                        .prompt_owner_queued_prompt_count_for_agent(
                            &leased_agent.backing_session_id,
                            &leased_agent.backing_agent_id,
                        )
                        .expect("worker prompt queue should load")
                };
                if worker_queue_count == 1 {
                    break;
                }
                sleep(Duration::from_millis(25)).await;
            }
            assert_eq!(
                worker_queue_count, 1,
                "the rejected steer must be admitted once as an ordinary worker prompt"
            );
            let (active_prompt, queued_count) = {
                let mut app = app_home.lock().await;
                let active = app
                    .prompt_owner_active_prompt_for_agent(&session_id, &remote_agent_id)
                    .expect("home active prompt should load");
                let queued = app
                    .prompt_owner_queued_prompt_count_for_agent(&session_id, &remote_agent_id)
                    .expect("home prompt queue should load");
                (active, queued)
            };
            let active_prompt = active_prompt.expect("ordinary queue advancement should start");
            assert_eq!(active_prompt.prompt(), "REMOTE_QUEUE_STEER_DELIVERY\n");
            assert_ne!(
                active_prompt.id(),
                queued_prompt_id,
                "promotion must allocate one new active-turn identity"
            );
            assert_eq!(active_prompt.pending_prompt_id(), None);
            assert_eq!(
                queued_count, 0,
                "the head must be removed from the queue once"
            );
            let steer_deliveries = app_worker
                .lock()
                .await
                .terminal()
                .input_records()
                .into_iter()
                .filter(|record| {
                    String::from_utf8_lossy(&record.bytes).contains("REMOTE_QUEUE_STEER_DELIVERY")
                })
                .count();
            assert_eq!(
                steer_deliveries, 0,
                "a rejected steer must not also be recorded as a steer delivery"
            );
            let _ = shutdown_home_tx.send(true);
            let _ = shutdown_worker_tx.send(true);
            connector_home.await.expect("home connector should join");
            connector_worker
                .await
                .expect("worker connector should join");
            let _ = server_shutdown_tx.send(());
            server_task.await.expect("server task should join");
            return;
        }
        if lose_queued_prompt_reply {
            let uncertain_error = match dispatch_result {
                Err(DaemonError::LocalTransport { message, .. }) => message,
                other => panic!("expected a diagnostic for the lost worker receipt, got {other:?}"),
            };
            assert!(
                uncertain_error.contains("remote steer outcome is uncertain")
                    && uncertain_error.contains(queued_prompt_id.as_str()),
                "lost reply should explain the exact queued prompt is held pending a worker receipt: {uncertain_error}"
            );

            let mut worker_deliveries = 0;
            for _ in 0..80 {
                worker_deliveries = app_worker
                    .lock()
                    .await
                    .terminal()
                    .input_records()
                    .into_iter()
                    .filter(|record| {
                        String::from_utf8_lossy(&record.bytes)
                            .contains("REMOTE_QUEUE_STEER_DELIVERY")
                    })
                    .count();
                if worker_deliveries == 1 {
                    break;
                }
                sleep(Duration::from_millis(25)).await;
            }
            assert_eq!(
                worker_deliveries, 1,
                "the worker must accept the steer before its response payload is lost"
            );

            let steering_merge_key = crate::history::steering_prompt_merge_key(&queued_prompt_id);
            let mut reconciled = false;
            for _ in 0..240 {
                let (queued, active, matching_history) = {
                    let mut home = app_home.lock().await;
                    let snapshot = home
                        .sessions()
                        .get_session(&session_id)
                        .expect("home session should remain available");
                    let queued = snapshot
                        .queued_prompts_for_agent(&remote_agent_id)
                        .is_some_and(|prompts| {
                            prompts.iter().any(|prompt| prompt.id() == queued_prompt_id)
                        });
                    let active = snapshot.active_prompt_for_agent(&remote_agent_id).is_some();
                    let matching_history = home
                        .operational_history_store()
                        .load_session_events(&session_id, Some(&remote_agent_id))
                        .expect("home history should load")
                        .into_iter()
                        .filter(|event| {
                            event
                                .metadata
                                .get("merge_key")
                                .and_then(serde_json::Value::as_str)
                                == Some(steering_merge_key.as_str())
                        })
                        .count();
                    (queued, active, matching_history)
                };
                if !queued && !active && matching_history == 1 {
                    reconciled = true;
                    break;
                }
                sleep(Duration::from_millis(25)).await;
            }
            assert!(
                reconciled,
                "the exact accepted worker receipt should remove only its queue item and merge one history event"
            );
            let (queued, active, history) = {
                let mut home = app_home.lock().await;
                let snapshot = home
                    .sessions()
                    .get_session(&session_id)
                    .expect("reconciled home session should snapshot");
                let queued = snapshot
                    .queued_prompts_for_agent(&remote_agent_id)
                    .is_some_and(|prompts| {
                        prompts.iter().any(|prompt| prompt.id() == queued_prompt_id)
                    });
                let active = snapshot.active_prompt_for_agent(&remote_agent_id).is_some();
                let history = home
                    .operational_history_store()
                    .load_session_events(&session_id, Some(&remote_agent_id))
                    .expect("reconciled history should load")
                    .into_iter()
                    .filter(|event| {
                        event
                            .metadata
                            .get("merge_key")
                            .and_then(serde_json::Value::as_str)
                            == Some(steering_merge_key.as_str())
                    })
                    .collect::<Vec<_>>();
                (queued, active, history)
            };
            assert!(
                !queued,
                "receipt reconciliation must settle the exact queued ID"
            );
            assert!(
                !active,
                "accepted steer reconciliation must not promote a second prompt"
            );
            assert_eq!(
                history.len(),
                1,
                "history reconciliation must be idempotent"
            );
            assert_eq!(
                history[0].provider_run_id.as_deref(),
                Some(projected_provider_run_id.as_str()),
                "reconciled history must identify the exact worker run projection"
            );
            let worker_steer_deliveries = app_worker
                .lock()
                .await
                .terminal()
                .input_records()
                .into_iter()
                .filter(|record| {
                    String::from_utf8_lossy(&record.bytes).contains("REMOTE_QUEUE_STEER_DELIVERY")
                })
                .count();
            assert_eq!(
                worker_steer_deliveries, 1,
                "receipt recovery must never resend the steer"
            );
            let _ = shutdown_home_tx.send(true);
            let _ = shutdown_worker_tx.send(true);
            connector_home.await.expect("home connector should join");
            connector_worker
                .await
                .expect("worker connector should join");
            let _ = server_shutdown_tx.send(());
            server_task.await.expect("server task should join");
            return;
        }
        dispatch_result.expect("remote queued prompt should steer through the worker")
    } else {
        router
            .dispatch(command, request)
            .await
            .expect("remote queued prompt should steer through the worker")
    };
    let LocalDaemonResponse::QueuedPromptSteered {
        prompt, session, ..
    } = response
    else {
        panic!("unexpected remote queued prompt steer response");
    };
    assert_eq!(prompt.id(), queued_prompt_id);
    assert!(session
        .queued_prompts_for_agent(&remote_agent_id)
        .is_none_or(|prompts| prompts.is_empty()));
    if hold_queued_prompt_reply {
        assert!(
            session.active_prompt_for_agent(&remote_agent_id).is_none(),
            "completion should remain settled while the exact queued prompt is steered"
        );
        let worker_steer_deliveries = app_worker
            .lock()
            .await
            .terminal()
            .input_records()
            .into_iter()
            .filter(|record| {
                String::from_utf8_lossy(&record.bytes).contains("REMOTE_QUEUE_STEER_DELIVERY")
            })
            .count();
        assert_eq!(worker_steer_deliveries, 1);
        let _ = shutdown_home_tx.send(true);
        let _ = shutdown_worker_tx.send(true);
        connector_home.await.expect("home connector should join");
        connector_worker
            .await
            .expect("worker connector should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
        return;
    }
    assert_eq!(
        session
            .active_prompt_for_agent(&remote_agent_id)
            .map(|prompt| prompt.prompt()),
        Some("remote prompt over home session\n")
    );
    let worker_steer_deliveries = app_worker
        .lock()
        .await
        .terminal()
        .input_records()
        .into_iter()
        .filter(|record| {
            String::from_utf8_lossy(&record.bytes).contains("REMOTE_QUEUE_STEER_DELIVERY")
        })
        .count();
    assert_eq!(worker_steer_deliveries, 1);
    let browser_steering_echo = app_home
        .lock()
        .await
        .terminal_mut()
        .drain_output_records(&session_id, &attachment_id)
        .into_iter()
        .find(|record| {
            String::from_utf8_lossy(&record.bytes).contains("REMOTE_QUEUE_STEER_DELIVERY")
        })
        .expect("queued steering should echo to the prompt source attachment");
    assert_eq!(
        browser_steering_echo.agent_id.as_deref(),
        Some(remote_agent_id.as_str()),
        "remote steering echoes must stay scoped to the target agent"
    );
    let steering_merge_key = crate::history::steering_prompt_merge_key(&queued_prompt_id);
    let steering_history = app_home
        .lock()
        .await
        .operational_history_store()
        .load_session_events(&session_id, Some(&remote_agent_id))
        .expect("remote steering history should load")
        .into_iter()
        .filter(|event| {
            event
                .metadata
                .get("merge_key")
                .and_then(serde_json::Value::as_str)
                == Some(steering_merge_key.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        steering_history.len(),
        1,
        "successful remote steering must persist one canonical history event"
    );
    assert_eq!(
        steering_history[0].provider_run_id.as_deref(),
        Some(projected_provider_run_id.as_str()),
        "home history must reference the namespaced remote projection, not a colliding local run"
    );
    assert!(steering_history[0]
        .content
        .as_deref()
        .is_some_and(|content| content.contains("REMOTE_QUEUE_STEER_DELIVERY")));

    let queued_request = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(remote_agent_id.clone()),
        prompt: "REMOTE_QUEUE_COMPLETION_DELIVERY\n".to_string(),
        attachments: Vec::new(),
    });
    let queued_command = KernelCommand::from_local_request(
        "command-remote-completion-queue",
        None,
        None,
        &queued_request,
    );
    assert!(matches!(
        router
            .dispatch(queued_command, queued_request)
            .await
            .unwrap(),
        LocalDaemonResponse::PromptSubmitted {
            outcome: crate::session::PromptSubmissionOutcome::Queued { .. },
            ..
        }
    ));
    let complete_request =
        LocalDaemonRequest::CompletePrompt(crate::local::CompletePromptRequest {
            session_id: session_id.clone(),
        });
    let complete_command = KernelCommand::from_local_request(
        "command-remote-completion-promote",
        None,
        None,
        &complete_request,
    );
    let response = router
        .dispatch(complete_command, complete_request)
        .await
        .expect("remote completion must promote the queued prompt through the runtime");
    let LocalDaemonResponse::PromptCompleted { completion, .. } = response else {
        panic!("unexpected remote completion response");
    };
    assert_eq!(completion.completed.target_agent_id(), remote_agent_id);
    assert_eq!(
        completion.completed.prompt(),
        "remote prompt over home session\n"
    );
    let promoted = completion
        .started_next
        .expect("queued prompt must be promoted");
    assert_eq!(promoted.prompt(), "REMOTE_QUEUE_COMPLETION_DELIVERY\n");
    let promoted_echoes = app_home
        .lock()
        .await
        .terminal()
        .output_records()
        .iter()
        .filter(|record| {
            record.kind == crate::terminal::TerminalOutputKind::PromptEcho
                && String::from_utf8_lossy(&record.bytes)
                    .contains("REMOTE_QUEUE_COMPLETION_DELIVERY")
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(promoted_echoes.len(), 1, "promoted prompt should echo once");
    assert_eq!(
        promoted_echoes[0].provider_run_id, projected_provider_run_id,
        "the promoted echo must identify the home-projected leased run"
    );
    let mut home = app_home.lock().await;
    let confirmed_run = home
        .agents()
        .get_agent(&remote_agent_id)
        .unwrap()
        .remote_execution()
        .unwrap()
        .active_worker_provider_run_id
        .clone();
    assert_eq!(
        confirmed_run.as_deref(),
        Some(worker_provider_run_id.as_str()),
        "queue promotion must retain the acknowledged worker run so managed output is accepted"
    );
    let active = home
        .prompt_owner_active_prompt_for_agent(&session_id, &remote_agent_id)
        .unwrap()
        .expect("promoted prompt must remain active");
    assert_eq!(active.id(), promoted.id());
    assert_eq!(
        active.durable_delivery_phase(),
        Some(crate::session::DurablePromptDeliveryPhase::Delivered),
        "acknowledged queue promotion must be durable as delivered"
    );
    drop(home);

    let worker_run = app_worker
        .lock()
        .await
        .providers()
        .get_run(&worker_provider_run_id)
        .expect("acknowledged worker run must exist");
    let event = RelayPeerEvent::LeasedRuntimeProjection {
        home_session_id: session_id.clone(),
        home_agent_id: remote_agent_id.clone(),
        provider_run_id: worker_provider_run_id.clone(),
        provider_run: Some(worker_run),
        prompts: Vec::new(),
        output_chunks: vec![crate::transport::relay_peer::RelayProjectedOutputChunk {
            kind: crate::terminal::TerminalOutputKind::ProviderOutput,
            merge_key: Some("promoted-worker-result".into()),
            bytes: b"PROMOTED_WORKER_RESULT".to_vec(),
        }],
        notices: Vec::new(),
        completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
            message_id: "promoted-worker-completion".into(),
            completed_at_ms: crate::session::unix_epoch_ms(),
            home_prompt_id: Some(promoted.id().to_string()),
            provider_termination: None,
        }],
    };
    let encrypted = relay_crypto::encrypt_payload_for_peer(
        &config_worker.relay_private_key,
        &config_home.relay_public_key,
        &serde_json::to_vec(&event).unwrap(),
    )
    .unwrap();
    handle_daemon_peer_event(
        &router,
        &state_home,
        &config_worker.daemon_id,
        None,
        encrypted,
    )
    .await
    .expect("acknowledged managed worker output must project to the home");
    let mut home = app_home.lock().await;
    let output = home
        .terminal_mut()
        .drain_output_records(&session_id, &attachment_id);
    assert!(
        output
            .iter()
            .any(|record| record.bytes == b"PROMOTED_WORKER_RESULT"
                && record.agent_id.as_deref() == Some(remote_agent_id.as_str())),
        "promoted managed output must reach the home attachment"
    );
    assert!(
        home.prompt_owner_active_prompt_for_agent(&session_id, &remote_agent_id)
            .unwrap()
            .is_none(),
        "promoted prompt must settle from its worker completion"
    );
    drop(home);

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
    if let Some((relay_reply_is_waiting, home_app_lock_available)) = held_reply_observation {
        assert!(
            relay_reply_is_waiting,
            "fake relay should have a routed agent-message request pending while the worker is paused"
        );
        assert!(
            home_app_lock_available,
            "home DaemonApp lock must remain available while the remote worker reply is pending"
        );
    }
}
#[test]
fn remote_machine_agents_materialize_file_attachments_on_the_worker() {
    run_async_with_large_test_stack(
        "remote-machine-agent-attachment-materialization",
        remote_machine_agents_materialize_file_attachments_on_the_worker_async,
    );
}

async fn remote_machine_agents_materialize_file_attachments_on_the_worker_async() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _test_home = RelayTestHome::new();
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let server = Arc::new(RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    }));
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_home = DaemonConfig::for_tests();
    config_home.daemon_id = "daemon-home".to_string();
    config_home.daemon_alias = Some("home".to_string());
    config_home.host_machine_id = "machine-home".to_string();
    config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;
    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-worker".to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.accept_remote_leases = true;
    let app_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_worker),
        Arc::clone(&state_worker),
        shutdown_worker_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
        .await
        .expect("worker kernels should be discoverable")
        .first()
        .and_then(|kernel| {
            kernel
                .available_providers
                .iter()
                .find(|provider| provider.as_str() == "managed-dev-stub")
        })
        .cloned()
        .expect("worker should advertise managed-dev-stub");

    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
    ));
    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_home),
        Arc::clone(&state_home),
        shutdown_home_rx,
    ));
    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
        .await
        .expect("home remote inventory should refresh");
    let (session_id, attachment_id, remote_agent_id, remote_leased_agent_id) = {
        let mut app_home = app_home.lock().await;
        let (session, _) = crate::app::KernelSessionService::new(&mut app_home)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app_home)
            .attach(AttachRequest::new(
                session.id(),
                "home-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home attachment should attach");
        let remote_agent = crate::app::KernelSessionService::new(&mut app_home)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), &provider)
                    .with_alias("remote-reviewer")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn");
        let leased_agent_id = remote_agent
            .remote_execution()
            .expect("remote binding should exist")
            .leased_agent_id
            .clone();
        (
            session.id().to_string(),
            attachment.id().to_string(),
            remote_agent.id().to_string(),
            leased_agent_id,
        )
    };

    let source_path = std::env::temp_dir().join(format!(
        "chariox-remote-attachment-{}.txt",
        crate::session::unix_epoch_ms()
    ));
    std::fs::write(&source_path, b"remote attachment body")
        .expect("source attachment should be written");

    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::clone(&app_home),
        1,
    );
    let submit_request = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(remote_agent_id.clone()),
        prompt: "prompt with attachment\n".to_string(),
        attachments: vec![crate::session::PromptAttachment::new(
            format!("file://{}", source_path.display()),
            "text/plain",
            Some("note.txt".to_string()),
        )],
    });
    let submit_command = KernelCommand::from_local_request(
        "remote-machine-agent-attachment-submit",
        None,
        None,
        &submit_request,
    );
    let LocalDaemonResponse::PromptSubmitted { outcome, .. } = router
        .dispatch(submit_command, submit_request)
        .await
        .expect("remote prompt should submit through the public kernel runtime")
    else {
        panic!("public remote attachment submission should return prompt state");
    };
    assert!(matches!(
        &outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let crate::session::PromptSubmissionOutcome::Started { prompt } = &outcome else {
        panic!("remote attachment prompt should start");
    };
    let _delivered_prompt = wait_for_home_remote_prompt_receipt(
        &app_home,
        &session_id,
        &remote_agent_id,
        prompt.id(),
    )
    .await;

    let worker_attachments = wait_for_leased_agent_active_prompt_attachments(
        app_worker.clone(),
        &remote_leased_agent_id,
    )
    .await;
    assert_eq!(worker_attachments.len(), 1);
    let materialized = &worker_attachments[0];
    assert_eq!(materialized.filename(), Some("note.txt"));
    assert_eq!(materialized.mime(), "text/plain");
    assert!(materialized.url().starts_with("file://"));
    assert_ne!(
        materialized.url(),
        format!("file://{}", source_path.display())
    );
    let worker_path = materialized.url().trim_start_matches("file://");
    let worker_bytes = std::fs::read(worker_path).expect("worker attachment should exist");
    assert_eq!(worker_bytes, b"remote attachment body");

    let _ = std::fs::remove_file(&source_path);
    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

async fn wait_for_leased_agent_active_prompt_attachments(
    app: Arc<Mutex<DaemonApp>>,
    leased_agent_id: &str,
) -> Vec<crate::session::PromptAttachment> {
    for _ in 0..400 {
        let attachments = {
            let mut app = app.lock().await;
            RemoteLeaseRuntime::new(&mut app)
                .leased_agent_active_prompt_attachments(leased_agent_id)
                .expect("worker prompt attachments should be available")
        };
        if !attachments.is_empty() {
            return attachments;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "worker prompt attachments did not become available for leased agent `{leased_agent_id}`"
    );
}

#[test]
fn remote_machine_agents_cancel_prompts_through_the_home_session() {
    run_async_with_large_test_stack(
        "remote-machine-agent-home-session-cancellation",
        remote_machine_agents_cancel_prompts_through_the_home_session_async,
    );
}

async fn remote_machine_agents_cancel_prompts_through_the_home_session_async() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _test_home = RelayTestHome::new();
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let server = Arc::new(RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    }));
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };

    let mut config_home = DaemonConfig::for_tests();
    config_home.daemon_id = "daemon-home".to_string();
    config_home.daemon_alias = Some("home".to_string());
    config_home.host_machine_id = "machine-home".to_string();
    config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_home.relay_token = Some("secret".to_string());
    config_home.relay_heartbeat_ms = 50;
    let mut config_worker = DaemonConfig::for_tests();
    config_worker.daemon_id = "daemon-worker".to_string();
    config_worker.daemon_alias = Some("worker".to_string());
    config_worker.host_machine_id = "machine-worker".to_string();
    config_worker.host_machine_alias = Some("builder-west".to_string());
    config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config_worker.relay_token = Some("secret".to_string());
    config_worker.relay_heartbeat_ms = 50;
    config_worker.accept_remote_leases = true;
    let app_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
    ));
    let state_worker = {
        let app = app_worker.lock().await;
        app.relay_client_state()
    };
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_worker),
        Arc::clone(&state_worker),
        shutdown_worker_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

    let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
        .await
        .expect("worker kernels should be discoverable")
        .first()
        .and_then(|kernel| {
            kernel
                .available_providers
                .iter()
                .find(|provider| provider.as_str() == "managed-dev-stub")
        })
        .cloned()
        .expect("worker should advertise managed-dev-stub");

    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
    ));
    let state_home = {
        let app = app_home.lock().await;
        app.relay_client_state()
    };
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_home),
        Arc::clone(&state_home),
        shutdown_home_rx,
    ));
    wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
    refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
        .await
        .expect("home remote inventory should refresh");
    let (session_id, attachment_id) = {
        let mut app_home = app_home.lock().await;
        let (session, _) = crate::app::KernelSessionService::new(&mut app_home)
            .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app_home)
            .attach(AttachRequest::new(
                session.id(),
                "home-client",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("home attachment should attach");
        (session.id().to_string(), attachment.id().to_string())
    };
    let remote_agent_id = {
        let mut app_home = app_home.lock().await;
        crate::app::KernelSessionService::new(&mut app_home)
            .spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("remote-reviewer")
                    .with_model("native-tui-idle")
                    .with_kernel(&config_worker.daemon_id),
            )
            .expect("remote agent should spawn")
            .id()
            .to_string()
    };

    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::clone(&app_home),
        1,
    );
    let submit_request = LocalDaemonRequest::SubmitPrompt(crate::local::SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(remote_agent_id.clone()),
        prompt: "cancel this remote prompt\n".to_string(),
        attachments: Vec::new(),
    });
    let submit_command = KernelCommand::from_local_request(
        "remote-machine-agent-cancel-submit",
        None,
        None,
        &submit_request,
    );
    let LocalDaemonResponse::PromptSubmitted { outcome, .. } = router
        .dispatch(submit_command, submit_request)
        .await
        .expect("remote prompt should submit through the public kernel runtime")
    else {
        panic!("public remote prompt submission should return prompt state");
    };
    let crate::session::PromptSubmissionOutcome::Started { prompt } = &outcome else {
        panic!("remote prompt should start before cancellation");
    };
    let _delivered_prompt = wait_for_home_remote_prompt_receipt(
        &app_home,
        &session_id,
        &remote_agent_id,
        prompt.id(),
    )
    .await;

    let cancel_request = || {
        LocalDaemonRequest::CancelActivePrompt(crate::local::CancelActivePromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment_id.clone(),
            target_agent_id: None,
        })
    };
    let cancellation_request = cancel_request();
    let cancellation_command = KernelCommand::from_local_request(
        "remote-machine-agent-cancel-first",
        None,
        None,
        &cancellation_request,
    );
    let LocalDaemonResponse::PromptCancelled { cancellation } = router
        .dispatch(cancellation_command, cancellation_request)
        .await
        .expect("remote prompt should cancel through the public kernel runtime")
    else {
        panic!("public remote cancellation should return prompt state");
    };
    let forced_cancellation_request = cancel_request();
    let forced_cancellation_command = KernelCommand::from_local_request(
        "remote-machine-agent-cancel-repeat",
        None,
        None,
        &forced_cancellation_request,
    );
    let LocalDaemonResponse::PromptCancelled {
        cancellation: forced_cancellation,
    } = router
        .dispatch(forced_cancellation_command, forced_cancellation_request)
        .await
        .expect("a repeated remote cancellation should force settlement")
    else {
        panic!("repeated remote cancellation should return prompt state");
    };
    assert!(matches!(
        outcome,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));

    assert_eq!(cancellation.prompt.target_agent_id(), remote_agent_id);
    assert_eq!(
        cancellation.prompt.status(),
        crate::session::PromptStatus::Cancelling
    );

    assert_eq!(
        forced_cancellation.prompt.status(),
        crate::session::PromptStatus::Cancelled
    );
    assert!(app_home
        .lock()
        .await
        .prompt_owner_active_prompt_for_agent(&session_id, &remote_agent_id)
        .expect("home prompt state should remain readable")
        .is_none());

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_worker_tx.send(true);
    connector_home.await.expect("home connector should join");
    connector_worker
        .await
        .expect("worker connector should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
