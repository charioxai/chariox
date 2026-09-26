use super::support::*;

fn leased_prompt_submission_request(
    leased_agent_id: &str,
    expected_profile: crate::transport::relay_peer::RelayAgentExecutionProfile,
    home_session_id: &str,
    home_agent_id: &str,
    home_prompt_id: &str,
    prompt: &str,
) -> RelayPeerRequest {
    RelayPeerRequest::SubmitLeasedPrompt {
        leased_agent_id: leased_agent_id.to_string(),
        expected_profile,
        prompt: prompt.to_string(),
        hidden_system_context: String::new(),
        attachments: Vec::new(),
        workflow_context: None,
        git_context: Some(crate::transport::relay_peer::RemoteGitTurnContext {
            home_session_id: home_session_id.to_string(),
            home_agent_id: home_agent_id.to_string(),
            home_prompt_id: home_prompt_id.to_string(),
            home_turn_id: format!("home-turn-{home_prompt_id}"),
            source_attachment_id: None,
            workspace_live_sync_mode: None,
            prompt_origin: None,
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            prompt_summary: "worker receipt restart fixture".to_string(),
        }),
        required_mcps: Vec::new(),
        required_skills: None,
        remote_extension_manifest: Default::default(),
        provider_launch_credential: None,
    }
}

fn isolate_test_config(
    mut config: DaemonConfig,
    root: &std::path::Path,
    name: &str,
) -> DaemonConfig {
    let state_root = root.join(name);
    std::fs::create_dir_all(&state_root).expect("isolated daemon state root should be created");
    config.user_config_path = state_root.join("config.toml");
    config.local_socket_path = state_root.join("daemon.sock");
    config.user_config.state.path = Some(state_root.join("state.db").display().to_string());
    config.user_config.history.operational.path = Some(
        state_root
            .join("operational-history.db")
            .display()
            .to_string(),
    );
    config.user_config.artifacts.operational.root =
        Some(state_root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(state_root.join("artifact-index.db").display().to_string());
    config.with_session_history_root(state_root.join("sessions"))
}

fn assert_unauthorized(result: Result<RelayPeerResponse, crate::error::DaemonError>) {
    match result {
        Err(crate::error::DaemonError::RelayTransport { code, .. }) => {
            assert_eq!(code, "unauthorized");
        }
        other => panic!("different authenticated home must be denied, got {other:?}"),
    }
}

#[test]
fn accepted_queued_steer_receipt_reconciles_after_worker_restart_without_replay() {
    run_async_with_large_test_stack(
        "accepted-queued-steer-receipt-restart",
        accepted_queued_steer_receipt_reconciles_after_worker_restart_without_replay_async,
    );
}

async fn accepted_queued_steer_receipt_reconciles_after_worker_restart_without_replay_async() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _test_home = RelayTestHome::new();
    let test_root = std::env::temp_dir().join(format!(
        "chariox-worker-steer-restart-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms(),
    ));
    std::fs::create_dir(&test_root).expect("isolated restart test root should be created");
    let listener = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    })
    .bind_listener()
    .await
    .expect("relay listener should bind");
    let addr = listener.local_addr().expect("relay listener address");
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

    let mut home_config = isolate_test_config(DaemonConfig::for_tests(), &test_root, "home");
    home_config.daemon_id = "steer-restart-home".to_string();
    home_config.host_machine_id = "steer-restart-home-machine".to_string();
    home_config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    home_config.relay_token = Some("secret".to_string());
    home_config.relay_heartbeat_ms = 50;
    let app_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(home_config.clone()).expect("home should bootstrap"),
    ));
    let state_home = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
    let connector_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_home),
        Arc::clone(&state_home),
        shutdown_home_rx,
    ));

    let mut worker_config = isolate_test_config(DaemonConfig::for_tests(), &test_root, "worker");
    worker_config.daemon_id = "steer-restart-worker".to_string();
    worker_config.host_machine_id = "steer-restart-worker-machine".to_string();
    worker_config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    worker_config.relay_token = Some("secret".to_string());
    worker_config.relay_heartbeat_ms = 50;
    worker_config.accept_remote_leases = true;
    let app_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(worker_config.clone()).expect("worker should bootstrap"),
    ));
    let state_worker = app_worker.lock().await.relay_client_state();
    let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
    let connector_worker = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_worker),
        Arc::clone(&state_worker),
        shutdown_worker_rx,
    ));
    wait_for_daemon_registration(registry.clone(), &home_config.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &worker_config.daemon_id).await;

    let (home_session_id, home_agent_id) = {
        let mut app = app_home.lock().await;
        create_test_session_with_alias(&mut app, "workspace-steer-restart", "worktree", "home")
    };
    let lease = match send_peer_request_via_relay(
        &app_home,
        &state_home,
        ClientTarget {
            daemon_id: Some(worker_config.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::CreateExecutionLease {
            home_kernel_id: home_config.daemon_id.clone(),
            home_session_id: home_session_id.clone(),
            home_agent_id,
            home_agent_metaagent: false,
            owner_user_id: "user-home".to_string(),
        },
    )
    .await
    .expect("worker should create the authenticated execution lease")
    {
        RelayPeerResponse::ExecutionLeaseCreated { lease, .. } => lease,
        other => panic!("unexpected lease response: {other:?}"),
    };
    let leased_agent = match send_peer_request_via_relay(
        &app_home,
        &state_home,
        ClientTarget {
            daemon_id: Some(worker_config.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::SpawnLeasedAgent {
            lease_id: lease.id.clone(),
            provider: "managed-dev-stub".to_string(),
            account_profile: "default".to_string(),
            model: Some("native-tui-idle".to_string()),
            effort: None,
            execution_mode: None,
            permission_level: None,
            workspace_live_sync_mode: None,
            worktree_id: None,
            worktree_placement: None,
        },
    )
    .await
    .expect("worker should spawn the leased agent")
    {
        RelayPeerResponse::LeasedAgentSpawned { leased_agent } => leased_agent,
        other => panic!("unexpected leased-agent response: {other:?}"),
    };

    let worker_profile = crate::transport::relay_peer::RelayAgentExecutionProfile {
        provider: leased_agent.provider.clone(),
        account_profile: leased_agent.account_profile.clone(),
        model: leased_agent.model.clone(),
        effort: leased_agent.effort.clone(),
    };
    let target_home_prompt_id = "home-prompt-before-worker-restart";
    state_worker
        .write()
        .await
        .test_lose_next_peer_response_payload();
    assert!(
        send_peer_request_via_relay(
            &app_home,
            &state_home,
            ClientTarget {
                daemon_id: Some(worker_config.daemon_id.clone()),
                daemon_alias: None,
            },
            leased_prompt_submission_request(
                &leased_agent.id,
                worker_profile.clone(),
                &home_session_id,
                &lease.home_agent_id,
                target_home_prompt_id,
                "initial worker prompt\n",
            ),
        )
        .await
        .is_err(),
        "the accepted prompt response should be lost on the relay path"
    );
    let accepted = send_peer_request_via_relay(
        &app_home,
        &state_home,
        ClientTarget {
            daemon_id: Some(worker_config.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::GetLeasedPromptReceipt {
            leased_agent_id: leased_agent.id.clone(),
            home_prompt_id: target_home_prompt_id.to_string(),
        },
    )
    .await
    .expect("worker should report the accepted prompt after a lost reply");
    let accepted = match accepted {
        RelayPeerResponse::LeasedPromptReceiptQueried {
            receipt: Some(receipt),
        } => receipt,
        other => panic!("unexpected accepted prompt receipt: {other:?}"),
    };
    assert_eq!(accepted.home_prompt_id, target_home_prompt_id);
    assert_eq!(
        accepted.execution_lease_id.as_deref(),
        Some(lease.id.as_str())
    );
    assert_eq!(
        accepted.phase,
        crate::transport::relay_peer::LeasedPromptReceiptPhase::Active
    );
    let worker_provider_run_id = accepted.worker_provider_run_id.clone();

    let steer_id = "home-queued-steer-before-worker-restart";
    state_worker
        .write()
        .await
        .test_lose_next_peer_response_payload();
    assert!(
        send_peer_request_via_relay(
            &app_home,
            &state_home,
            ClientTarget {
                daemon_id: Some(worker_config.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::SteerLeasedPrompt {
                leased_agent_id: leased_agent.id.clone(),
                steer_id: steer_id.to_string(),
                target_home_prompt_id: target_home_prompt_id.to_string(),
                prompt: "accepted queued steer\n".to_string(),
                hidden_system_context: String::new(),
                attachments: Vec::new(),
                required_skills: None,
            },
        )
        .await
        .is_err(),
        "the accepted worker response should be lost on the relay path"
    );
    {
        let mut worker = app_worker.lock().await;
        let recorded = RemoteLeaseRuntime::new(&mut worker)
            .leased_agent_snapshot_for_test(&leased_agent.id)
            .expect("worker lease should remain live before restart");
        assert!(
            recorded.home_steer_receipts.iter().any(|receipt| {
                receipt.steer_id == steer_id
                    && receipt.target_home_prompt_id == target_home_prompt_id
                    && receipt.worker_provider_run_id == worker_provider_run_id
                    && receipt.execution_lease_id == lease.id
                    && receipt.phase
                        == crate::execution_lease::LeasedPromptSteerReceiptPhase::Accepted
            }),
            "worker must accept and record the steer before its reply is lost"
        );
    }

    let rejected_prompt_id = "home-prompt-rejected-while-active";
    let changed_profile = crate::transport::relay_peer::RelayAgentExecutionProfile {
        model: Some("native-tui-idle-after-rejection".to_string()),
        ..worker_profile
    };
    let rejected_submission = leased_prompt_submission_request(
        &leased_agent.id,
        changed_profile.clone(),
        &home_session_id,
        &lease.home_agent_id,
        rejected_prompt_id,
        "prompt rejected while the prior turn is active\n",
    );
    assert!(
        send_peer_request_via_relay(
            &app_home,
            &state_home,
            ClientTarget {
                daemon_id: Some(worker_config.daemon_id.clone()),
                daemon_alias: None,
            },
            rejected_submission,
        )
        .await
        .is_err(),
        "a profile change while the prior turn is active must be rejected"
    );
    let rejection = send_peer_request_via_relay(
        &app_home,
        &state_home,
        ClientTarget {
            daemon_id: Some(worker_config.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::GetLeasedPromptReceipt {
            leased_agent_id: leased_agent.id.clone(),
            home_prompt_id: rejected_prompt_id.to_string(),
        },
    )
    .await
    .expect("worker should report definite non-admission");
    let rejection = match rejection {
        RelayPeerResponse::LeasedPromptReceiptQueried {
            receipt: Some(receipt),
        } => receipt,
        other => panic!("unexpected rejected prompt receipt: {other:?}"),
    };
    assert_eq!(rejection.home_prompt_id, rejected_prompt_id);
    assert_eq!(rejection.worker_provider_run_id, "");
    assert_eq!(
        rejection.execution_lease_id.as_deref(),
        Some(lease.id.as_str())
    );
    assert!(rejection.target_home_prompt_id.is_none());
    assert_eq!(
        rejection.phase,
        crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerRejected
    );
    assert!(matches!(
        send_peer_request_via_relay(
            &app_home,
            &state_home,
            ClientTarget {
                daemon_id: Some(worker_config.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::CompleteLeasedPrompt {
                leased_agent_id: leased_agent.id.clone(),
            },
        )
        .await
        .expect("worker should complete the original prompt"),
        RelayPeerResponse::LeasedPromptCompleted { .. }
    ));
    assert!(
        send_peer_request_via_relay(
            &app_home,
            &state_home,
            ClientTarget {
                daemon_id: Some(worker_config.daemon_id.clone()),
                daemon_alias: None,
            },
            leased_prompt_submission_request(
                &leased_agent.id,
                changed_profile,
                &home_session_id,
                &lease.home_agent_id,
                rejected_prompt_id,
                "prompt rejected while the prior turn is active\n",
            ),
        )
        .await
        .is_err(),
        "the rejection tombstone must fence a delayed duplicate after completion"
    );

    let _ = shutdown_worker_tx.send(true);
    connector_worker
        .await
        .expect("old worker connector should stop");
    app_worker
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true)
        .expect("old worker provider process should stop before restart");
    drop(state_worker);
    drop(app_worker);
    for _ in 0..80 {
        if registry
            .read()
            .await
            .daemon(&worker_config.daemon_id)
            .is_none()
        {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }
    assert!(
        registry
            .read()
            .await
            .daemon(&worker_config.daemon_id)
            .is_none(),
        "old worker registration must be gone before restoring its state"
    );

    let restarted_worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(worker_config.clone()).expect("worker should restore same state"),
    ));
    let restarted_state = restarted_worker.lock().await.relay_client_state();
    let (shutdown_restarted_tx, shutdown_restarted_rx) = watch::channel(false);
    let restarted_connector = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&restarted_worker),
        Arc::clone(&restarted_state),
        shutdown_restarted_rx,
    ));
    wait_for_daemon_registration(registry.clone(), &worker_config.daemon_id).await;

    let mut other_home_config =
        isolate_test_config(DaemonConfig::for_tests(), &test_root, "other-home");
    other_home_config.daemon_id = "steer-restart-other-home".to_string();
    other_home_config.host_machine_id = "steer-restart-other-machine".to_string();
    other_home_config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    other_home_config.relay_token = Some("secret".to_string());
    other_home_config.relay_heartbeat_ms = 50;
    assert_ne!(other_home_config.daemon_id, home_config.daemon_id);
    assert_ne!(
        other_home_config.host_machine_id,
        home_config.host_machine_id
    );
    assert_ne!(
        other_home_config.relay_public_key,
        home_config.relay_public_key
    );
    let app_other_home = Arc::new(Mutex::new(
        DaemonApp::bootstrap(other_home_config.clone()).expect("other home should bootstrap"),
    ));
    let state_other_home = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_other_home_tx, shutdown_other_home_rx) = watch::channel(false);
    let connector_other_home = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app_other_home),
        Arc::clone(&state_other_home),
        shutdown_other_home_rx,
    ));
    wait_for_daemon_registration(registry.clone(), &other_home_config.daemon_id).await;

    let other_target = ClientTarget {
        daemon_id: Some(worker_config.daemon_id.clone()),
        daemon_alias: None,
    };
    assert_unauthorized(
        send_peer_request_via_relay(
            &app_other_home,
            &state_other_home,
            other_target.clone(),
            RelayPeerRequest::GetLeasedPromptReceipt {
                leased_agent_id: leased_agent.id.clone(),
                home_prompt_id: steer_id.to_string(),
            },
        )
        .await,
    );
    assert_unauthorized(
        send_peer_request_via_relay(
            &app_other_home,
            &state_other_home,
            other_target,
            RelayPeerRequest::ReconcileLeasedPromptSteerReceipt {
                leased_agent_id: leased_agent.id.clone(),
                steer_id: steer_id.to_string(),
                target_home_prompt_id: target_home_prompt_id.to_string(),
                worker_provider_run_id: worker_provider_run_id.clone(),
                execution_lease_id: lease.id.clone(),
            },
        )
        .await,
    );

    for (requested_run, requested_lease) in [
        ("wrong-worker-run", lease.id.as_str()),
        (worker_provider_run_id.as_str(), "wrong-execution-lease"),
    ] {
        assert!(
            send_peer_request_via_relay(
                &app_home,
                &state_home,
                ClientTarget {
                    daemon_id: Some(worker_config.daemon_id.clone()),
                    daemon_alias: None,
                },
                RelayPeerRequest::ReconcileLeasedPromptSteerReceipt {
                    leased_agent_id: leased_agent.id.clone(),
                    steer_id: steer_id.to_string(),
                    target_home_prompt_id: target_home_prompt_id.to_string(),
                    worker_provider_run_id: requested_run.to_string(),
                    execution_lease_id: requested_lease.to_string(),
                },
            )
            .await
            .is_err(),
            "a mismatched worker run or lease must not read or replace the exact receipt"
        );
    }

    let recovered = send_peer_request_via_relay(
        &app_home,
        &state_home,
        ClientTarget {
            daemon_id: Some(worker_config.daemon_id.clone()),
            daemon_alias: None,
        },
        RelayPeerRequest::ReconcileLeasedPromptSteerReceipt {
            leased_agent_id: leased_agent.id.clone(),
            steer_id: steer_id.to_string(),
            target_home_prompt_id: target_home_prompt_id.to_string(),
            worker_provider_run_id: worker_provider_run_id.clone(),
            execution_lease_id: lease.id.clone(),
        },
    )
    .await
    .expect("exact old binding should retrieve the accepted receipt after restart");
    let recovered = match recovered {
        RelayPeerResponse::LeasedPromptReceiptQueried {
            receipt: Some(receipt),
        } => receipt,
        other => panic!("unexpected recovered receipt response: {other:?}"),
    };
    assert_eq!(recovered.home_prompt_id, steer_id);
    assert_eq!(
        recovered.target_home_prompt_id.as_deref(),
        Some(target_home_prompt_id)
    );
    assert_eq!(
        recovered.execution_lease_id.as_deref(),
        Some(lease.id.as_str())
    );
    assert_eq!(
        recovered.phase,
        crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerAccepted
    );
    for (home_prompt_id, expected_phase, expected_run) in [
        (
            target_home_prompt_id,
            crate::transport::relay_peer::LeasedPromptReceiptPhase::Active,
            worker_provider_run_id.as_str(),
        ),
        (
            rejected_prompt_id,
            crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerRejected,
            "",
        ),
    ] {
        let result = send_peer_request_via_relay(
            &app_home,
            &state_home,
            ClientTarget {
                daemon_id: Some(worker_config.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::GetLeasedPromptReceipt {
                leased_agent_id: leased_agent.id.clone(),
                home_prompt_id: home_prompt_id.to_string(),
            },
        )
        .await
        .expect("original-prompt receipt should survive worker restart");
        let receipt = match result {
            RelayPeerResponse::LeasedPromptReceiptQueried {
                receipt: Some(receipt),
            } => receipt,
            other => panic!("unexpected restored prompt receipt: {other:?}"),
        };
        assert_eq!(receipt.home_prompt_id, home_prompt_id);
        assert_eq!(receipt.worker_provider_run_id, expected_run);
        assert_eq!(
            receipt.execution_lease_id.as_deref(),
            Some(lease.id.as_str())
        );
        assert_eq!(receipt.phase, expected_phase);
        assert!(receipt.target_home_prompt_id.is_none());
    }
    assert!(
        restarted_worker
            .lock()
            .await
            .providers()
            .list_runs()
            .is_empty(),
        "receipt recovery must not launch or replay a provider run on the restarted worker"
    );

    let _ = shutdown_home_tx.send(true);
    let _ = shutdown_restarted_tx.send(true);
    let _ = shutdown_other_home_tx.send(true);
    connector_home.await.expect("home connector should stop");
    restarted_connector
        .await
        .expect("restarted worker connector should stop");
    connector_other_home
        .await
        .expect("other home connector should stop");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("relay server should stop");
    drop(state_home);
    drop(restarted_state);
    drop(app_home);
    drop(restarted_worker);
    drop(state_other_home);
    drop(app_other_home);
    std::fs::remove_dir_all(test_root).expect("isolated restart test artifacts should clean up");
}
