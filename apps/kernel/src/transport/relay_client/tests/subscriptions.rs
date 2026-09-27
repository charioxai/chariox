#![allow(unused_imports)]
use super::support::*;
use crate::transport::relay_client::subscriptions::run_relay_subscription_loop;

struct RelayEventStoreTestRoot(std::path::PathBuf);

impl RelayEventStoreTestRoot {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-{label}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms(),
        ));
        std::fs::create_dir_all(&path).expect("relay event test root should be created");
        Self(path)
    }

    fn join(&self, path: &str) -> std::path::PathBuf {
        self.0.join(path)
    }
}

impl Drop for RelayEventStoreTestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn relay_event_store_recovers_both_subscription_scopes_after_append_failure() {
    let root = std::env::temp_dir().join(format!(
        "chariox-relay-event-store-recovery-test-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms(),
    ));
    let counter_path = root.join("event-counter.json");
    let events_path = root.join("relay-events.jsonl");
    let runtime = RelayEventRuntime::new(&counter_path)
        .expect("persistent relay event runtime should initialize");
    let session_stream = subscription_event_stream_id("session-a", "attachment-a");

    std::fs::create_dir_all(&events_path).expect("event store path should become unwritable");
    runtime
        .event_log
        .append(
            session_stream.clone(),
            KernelEvent::Heartbeat {
                session_id: "session-a".to_string(),
            },
        )
        .await
        .expect_err("relay append failure must be visible before delivery");

    std::fs::remove_dir(&events_path).expect("event store path should become writable again");
    let session_first = runtime
        .event_log
        .append(
            session_stream.clone(),
            KernelEvent::Heartbeat {
                session_id: "session-a".to_string(),
            },
        )
        .await
        .expect("session events should recover without a runtime restart");
    let waiting_first = runtime
        .event_log
        .append(
            WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
            KernelEvent::Heartbeat {
                session_id: WAITING_ROOM_INVENTORY_SENTINEL_ID.to_string(),
            },
        )
        .await
        .expect("waiting-room events should recover without a runtime restart");
    let session_second = runtime
        .event_log
        .append(
            session_stream.clone(),
            KernelEvent::Heartbeat {
                session_id: "session-a".to_string(),
            },
        )
        .await
        .expect("later session events should preserve order");
    let waiting_second = runtime
        .event_log
        .append(
            WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
            KernelEvent::Heartbeat {
                session_id: WAITING_ROOM_INVENTORY_SENTINEL_ID.to_string(),
            },
        )
        .await
        .expect("later waiting-room events should preserve order");
    assert!(session_first.event_id < waiting_first.event_id);
    assert!(waiting_first.event_id < session_second.event_id);
    assert!(session_second.event_id < waiting_second.event_id);
    drop(runtime);

    let restarted = RelayEventRuntime::new(&counter_path)
        .expect("recovered relay event runtime should restart");
    match restarted
        .event_log
        .replay_after(&session_stream, session_first.event_id)
        .await
    {
        ReplayOutcome::Replayed(events) => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event_id, session_second.event_id);
            assert!(matches!(
                &events[0].event,
                KernelEvent::Heartbeat { session_id } if session_id == "session-a"
            ));
        }
        ReplayOutcome::Gap(gap) => panic!("session events should replay in order: {gap:?}"),
    }
    match restarted
        .event_log
        .replay_after(
            WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
            waiting_first.event_id,
        )
        .await
    {
        ReplayOutcome::Replayed(events) => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].event_id, waiting_second.event_id);
            assert!(matches!(
                &events[0].event,
                KernelEvent::Heartbeat { session_id }
                    if session_id == WAITING_ROOM_INVENTORY_SENTINEL_ID
            ));
        }
        ReplayOutcome::Gap(gap) => {
            panic!("waiting-room events should replay in order: {gap:?}")
        }
    }
    drop(restarted);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_waiting_room_subscription_sends_baseline_after_reload_and_observes_mutations() {
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

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app),
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector_with_router(
        Arc::clone(&router),
        Arc::clone(&state),
        shutdown_rx,
    ));
    wait_for_daemon_registration(registry, &config.daemon_id).await;

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut client_socket, _) = connect_async(&url)
        .await
        .expect("client should connect to relay");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let _daemon_public_key = expect_client_connected(&mut client_socket).await;
    let subscription_private_key = relay_crypto::generate_private_key_base64();
    let subscription_public_key =
        relay_crypto::public_key_from_private_key_base64(&subscription_private_key)
            .expect("subscription public key should derive");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientSubscribe {
            request_id: "waiting-room-subscribe".to_string(),
            subscription_id: "waiting-room-subscription".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
            session_id: WAITING_ROOM_INVENTORY_SENTINEL_ID.to_string(),
            attachment_id: WAITING_ROOM_INVENTORY_SENTINEL_ID.to_string(),
            client_public_key: subscription_public_key,
            subscription_scope: Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE.to_string()),
            resume_from_event_id: None,
        },
    )
    .await;
    let _ = expect_json_client_response(
        &mut client_socket,
        "waiting-room-subscribe",
        &subscription_private_key,
    )
    .await;
    let first_inventory = expect_named_client_event(
        &mut client_socket,
        &subscription_private_key,
        "waiting_room_rows_changed",
    )
    .await
    .1;

    let (mut reloaded_client_socket, _) = connect_async(&url)
        .await
        .expect("reloaded waiting-room client should connect to relay");
    send_client_envelope(
        &mut reloaded_client_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let _reloaded_daemon_public_key = expect_client_connected(&mut reloaded_client_socket).await;
    let reloaded_subscription_private_key = relay_crypto::generate_private_key_base64();
    let reloaded_subscription_public_key =
        relay_crypto::public_key_from_private_key_base64(&reloaded_subscription_private_key)
            .expect("reloaded subscription public key should derive");
    send_client_envelope(
        &mut reloaded_client_socket,
        &RelayEnvelope::ClientSubscribe {
            request_id: "waiting-room-reload-subscribe".to_string(),
            subscription_id: "waiting-room-reload-subscription".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
            session_id: WAITING_ROOM_INVENTORY_SENTINEL_ID.to_string(),
            attachment_id: WAITING_ROOM_INVENTORY_SENTINEL_ID.to_string(),
            client_public_key: reloaded_subscription_public_key,
            subscription_scope: Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE.to_string()),
            resume_from_event_id: None,
        },
    )
    .await;
    let _ = expect_json_client_response(
        &mut reloaded_client_socket,
        "waiting-room-reload-subscribe",
        &reloaded_subscription_private_key,
    )
    .await;
    let reloaded_inventory = tokio::time::timeout(
        Duration::from_secs(2),
        expect_named_client_event(
            &mut reloaded_client_socket,
            &reloaded_subscription_private_key,
            "waiting_room_rows_changed",
        ),
    )
    .await
    .expect("reloaded waiting-room subscriber should receive its own unchanged baseline")
    .1;
    assert_eq!(
        reloaded_inventory["inventory_version"], first_inventory["inventory_version"],
        "each null-cursor subscriber should receive the same unchanged inventory baseline",
    );

    let request = LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("workspace-relay-inventory", "worktree-relay-inventory")
            .with_alias("relay-visible-session"),
    );
    let command =
        KernelCommand::from_local_request("create-relay-visible-session", None, None, &request);
    let response = router
        .dispatch(command, request)
        .await
        .expect("session creation should succeed");
    assert!(matches!(
        response,
        LocalDaemonResponse::SessionCreated { .. }
    ));

    let event = tokio::time::timeout(
        Duration::from_secs(2),
        expect_named_client_event(
            &mut client_socket,
            &subscription_private_key,
            "waiting_room_rows_changed",
        ),
    )
    .await
    .expect("relay waiting-room subscription should observe the shared router mutation")
    .1;
    assert_eq!(
        event["sessions"][0]["alias"],
        serde_json::json!("relay-visible-session")
    );
    let reloaded_event = tokio::time::timeout(
        Duration::from_secs(2),
        expect_named_client_event(
            &mut reloaded_client_socket,
            &reloaded_subscription_private_key,
            "waiting_room_rows_changed",
        ),
    )
    .await
    .expect("every concurrent relay waiting-room subscriber should observe the mutation")
    .1;
    assert_eq!(
        reloaded_event["sessions"][0]["alias"],
        serde_json::json!("relay-visible-session")
    );

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn proxied_session_subscriptions_share_one_attachment_without_replacement() {
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

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    let created_session_id = {
        let mut app = app.lock().await;
        create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test")
    };
    let attachment_id = {
        let mut app = app.lock().await;
        attach_test_client(
            &mut app,
            &created_session_id,
            "relay-client",
            ClientCapabilityLevel::MessageTransport,
        )
    };
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app),
        Arc::clone(&state),
        shutdown_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut client_socket, _) = connect_async(&url)
        .await
        .expect("client should connect to relay");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let _daemon_public_key = expect_client_connected(&mut client_socket).await;

    let subscription_private_key = relay_crypto::generate_private_key_base64();
    let subscription_public_key =
        relay_crypto::public_key_from_private_key_base64(&subscription_private_key)
            .expect("subscription public key should derive");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientSubscribe {
            request_id: "sub-1".to_string(),
            subscription_id: "subscription-1".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
            session_id: created_session_id.clone(),
            attachment_id: attachment_id.clone(),
            client_public_key: subscription_public_key.clone(),
            subscription_scope: None,
            resume_from_event_id: None,
        },
    )
    .await;
    let subscribe_response =
        expect_json_client_response(&mut client_socket, "sub-1", &subscription_private_key).await;
    assert_eq!(subscribe_response["ok"], serde_json::json!(true));

    let event = expect_client_event(&mut client_socket, &subscription_private_key).await;
    assert_eq!(event["event"], serde_json::json!("session_snapshot"));
    assert_eq!(
        event["session"]["id"],
        serde_json::json!(created_session_id)
    );
    let _initial_first_heartbeat =
        expect_named_client_event(&mut client_socket, &subscription_private_key, "heartbeat").await;

    let (mut second_client_socket, _) = connect_async(&url)
        .await
        .expect("second client should connect to relay");
    send_client_envelope(
        &mut second_client_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let _second_daemon_public_key = expect_client_connected(&mut second_client_socket).await;
    let second_subscription_private_key = relay_crypto::generate_private_key_base64();
    let second_subscription_public_key =
        relay_crypto::public_key_from_private_key_base64(&second_subscription_private_key)
            .expect("second subscription public key should derive");
    send_client_envelope(
        &mut second_client_socket,
        &RelayEnvelope::ClientSubscribe {
            request_id: "sub-2".to_string(),
            subscription_id: "subscription-2".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
            session_id: created_session_id.clone(),
            attachment_id: attachment_id.clone(),
            client_public_key: second_subscription_public_key,
            subscription_scope: None,
            resume_from_event_id: None,
        },
    )
    .await;
    let second_subscribe_response = expect_json_client_response(
        &mut second_client_socket,
        "sub-2",
        &second_subscription_private_key,
    )
    .await;
    assert_eq!(second_subscribe_response["ok"], serde_json::json!(true));
    let second_event =
        expect_client_event(&mut second_client_socket, &second_subscription_private_key).await;
    assert_eq!(second_event["event"], serde_json::json!("session_snapshot"));

    let first_heartbeat = tokio::time::timeout(
        Duration::from_secs(7),
        expect_named_client_event(&mut client_socket, &subscription_private_key, "heartbeat"),
    )
    .await
    .expect("first subscription should remain live after second subscription");
    assert_eq!(
        first_heartbeat.1["session_id"],
        serde_json::json!(created_session_id)
    );
    let second_heartbeat = tokio::time::timeout(
        Duration::from_secs(7),
        expect_named_client_event(
            &mut second_client_socket,
            &second_subscription_private_key,
            "heartbeat",
        ),
    )
    .await
    .expect("second subscription should remain live");
    assert_eq!(
        second_heartbeat.1["session_id"],
        serde_json::json!(created_session_id)
    );

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
#[tokio::test(flavor = "multi_thread")]
async fn relay_subscription_replays_recent_events_after_resume_cursor() {
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

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    let created_session_id = {
        let mut app = app.lock().await;
        create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test")
    };
    let attachment_id = {
        let mut app = app.lock().await;
        attach_test_client(
            &mut app,
            &created_session_id,
            "relay-client",
            ClientCapabilityLevel::MessageTransport,
        )
    };
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app),
        Arc::clone(&state),
        shutdown_rx,
    ));

    wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut client_socket, _) = connect_async(&url)
        .await
        .expect("client should connect to relay");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
        },
    )
    .await;
    let _daemon_public_key = expect_client_connected(&mut client_socket).await;

    let subscription_private_key = relay_crypto::generate_private_key_base64();
    let subscription_public_key =
        relay_crypto::public_key_from_private_key_base64(&subscription_private_key)
            .expect("subscription public key should derive");
    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientSubscribe {
            request_id: "sub-1".to_string(),
            subscription_id: "subscription-1".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
            session_id: created_session_id.clone(),
            attachment_id: attachment_id.clone(),
            client_public_key: subscription_public_key.clone(),
            subscription_scope: None,
            resume_from_event_id: None,
        },
    )
    .await;
    let _ =
        expect_json_client_response(&mut client_socket, "sub-1", &subscription_private_key).await;
    let first_event =
        expect_client_event_envelope(&mut client_socket, &subscription_private_key).await;
    assert_eq!(
        first_event.1["event"],
        serde_json::json!("session_snapshot")
    );

    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientUnsubscribe {
            request_id: "unsub-1".to_string(),
            subscription_id: "subscription-1".to_string(),
            client_public_key: subscription_public_key.clone(),
        },
    )
    .await;
    let _ =
        expect_json_client_response(&mut client_socket, "unsub-1", &subscription_private_key).await;

    send_client_envelope(
        &mut client_socket,
        &RelayEnvelope::ClientSubscribe {
            request_id: "sub-2".to_string(),
            subscription_id: "subscription-1".to_string(),
            target: ClientTarget {
                daemon_id: Some(config.daemon_id.clone()),
                daemon_alias: None,
            },
            session_id: created_session_id.clone(),
            attachment_id: attachment_id.clone(),
            client_public_key: subscription_public_key,
            subscription_scope: None,
            resume_from_event_id: Some(first_event.0),
        },
    )
    .await;
    let resume_response =
        expect_json_client_response(&mut client_socket, "sub-2", &subscription_private_key).await;
    assert_eq!(
        resume_response["resumed_from_event_id"],
        serde_json::json!(first_event.0)
    );
    let resumed_event = expect_named_client_event(
        &mut client_socket,
        &subscription_private_key,
        "transport_resumed",
    )
    .await;
    assert_eq!(
        resumed_event.1["resumed_from_event_id"],
        serde_json::json!(first_event.0)
    );

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
#[tokio::test(flavor = "multi_thread")]
async fn relay_subscription_emits_replay_gap_and_snapshot_for_stale_cursor() {
    let _relay_test_guard = relay_client_test_guard().await;
    let config = DaemonConfig::for_tests();
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    let created_session_id = {
        let mut app = app.lock().await;
        create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test")
    };
    let attachment_id = {
        let mut app = app.lock().await;
        attach_test_client(
            &mut app,
            &created_session_id,
            "relay-client",
            ClientCapabilityLevel::MessageTransport,
        )
    };
    let provider_runtime_lanes = {
        let app = app.lock().await;
        app.provider_run_operation_lanes()
    };
    let router = Arc::new(CommandRouter::with_interactive_capacity_and_provider_lanes(
        Arc::clone(&app),
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
        provider_runtime_lanes,
    ));
    let event_runtime = Arc::new(RelayEventRuntime::for_tests(1));
    let event_stream_id = subscription_event_stream_id(&created_session_id, &attachment_id);
    let first = event_runtime
        .event_log
        .append(
            event_stream_id.clone(),
            KernelEvent::Heartbeat {
                session_id: created_session_id.clone(),
            },
        )
        .await
        .expect("first event should append");
    let second = event_runtime
        .event_log
        .append(
            event_stream_id,
            KernelEvent::Heartbeat {
                session_id: created_session_id.clone(),
            },
        )
        .await
        .expect("second event should append");

    let (outgoing_tx, _priority_rx, mut event_rx) =
        RelayOutgoingSender::channel(RELAY_OUTGOING_QUEUE_LIMIT);
    let subscription_private_key = relay_crypto::generate_private_key_base64();
    let subscription_public_key =
        relay_crypto::public_key_from_private_key_base64(&subscription_private_key)
            .expect("subscription public key should derive");

    replay_recent_relay_events(
        &event_runtime,
        &router,
        &outgoing_tx,
        "subscription-1",
        &subscription_public_key,
        &created_session_id,
        &attachment_id,
        Some(first.event_id),
    )
    .await
    .expect("stale replay should emit recovery events");

    let gap = decrypt_relay_event_from_channel(&mut event_rx, &subscription_private_key).await;
    assert_eq!(gap.0, second.event_id + 1);
    assert_eq!(gap.1["event"], serde_json::json!("replay_gap"));
    assert_eq!(
        gap.1["requested_from_event_id"],
        serde_json::json!(first.event_id)
    );
    assert_eq!(
        gap.1["first_retained_event_id"],
        serde_json::json!(second.event_id)
    );
    assert_eq!(gap.1["latest_event_id"], serde_json::json!(second.event_id));

    let snapshot = decrypt_relay_event_from_channel(&mut event_rx, &subscription_private_key).await;
    assert_eq!(snapshot.0, second.event_id + 2);
    assert_eq!(snapshot.1["event"], serde_json::json!("session_snapshot"));
    assert_eq!(
        snapshot.1["session"]["id"],
        serde_json::json!(created_session_id)
    );

    let resumed = decrypt_relay_event_from_channel(&mut event_rx, &subscription_private_key).await;
    assert_eq!(resumed.0, second.event_id + 3);
    assert_eq!(resumed.1["event"], serde_json::json!("transport_resumed"));
    assert_eq!(
        resumed.1["resumed_from_event_id"],
        serde_json::json!(first.event_id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_replay_after_runtime_recreation_filters_historical_resume_and_resets_snapshot() {
    const HISTORICAL_ACTIVITY_REVISION: u64 = 13_088;

    let config = DaemonConfig::for_tests();
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("daemon should bootstrap"),
    ));
    let session_id = {
        let mut app = app.lock().await;
        create_test_session(&mut app, "workspace-relay-replay", "worktree-relay-replay")
    };
    let attachment_id = {
        let mut app = app.lock().await;
        attach_test_client(
            &mut app,
            &session_id,
            "relay-replay-client",
            ClientCapabilityLevel::MessageTransport,
        )
    };
    let provider_runtime_lanes = {
        let app = app.lock().await;
        app.provider_run_operation_lanes()
    };
    let router = Arc::new(CommandRouter::with_interactive_capacity_and_provider_lanes(
        Arc::clone(&app),
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
        provider_runtime_lanes,
    ));
    let current_activity_revision = router.session_projection_change_sequence();
    assert!(
        current_activity_revision < HISTORICAL_ACTIVITY_REVISION,
        "fixture must model a projection revision reset"
    );

    let root = RelayEventStoreTestRoot::new("relay-runtime-recreation");
    let counter_path = root.join("event-counter.json");
    let stream_id = subscription_event_stream_id(&session_id, &attachment_id);
    let historical_projection = router
        .session_snapshot_projection_for_attachment(
            &session_id,
            &attachment_id,
            HISTORICAL_ACTIVITY_REVISION,
        )
        .expect("historical projection should build");
    let first_runtime = RelayEventRuntime::new(&counter_path)
        .expect("first persistent relay event runtime should initialize");
    let cursor = first_runtime
        .event_log
        .append(
            stream_id.clone(),
            KernelEvent::Heartbeat {
                session_id: session_id.clone(),
            },
        )
        .await
        .expect("cursor event should persist");
    let historical_snapshot = first_runtime
        .event_log
        .append(
            stream_id.clone(),
            KernelEvent::SessionSnapshot {
                session: Box::new(historical_projection.session),
                provider_run: Box::new(historical_projection.provider_run),
                agent_activity: Box::new(historical_projection.agent_activity),
                agent_activity_revision: HISTORICAL_ACTIVITY_REVISION,
            },
        )
        .await
        .expect("historical snapshot should persist");
    first_runtime
        .event_log
        .append(
            stream_id.clone(),
            KernelEvent::TransportResumed {
                session_id: session_id.clone(),
                resumed_from_event_id: Some(cursor.event_id),
            },
        )
        .await
        .expect("historical resume marker should persist");
    let historical_heartbeat = first_runtime
        .event_log
        .append(
            stream_id,
            KernelEvent::Heartbeat {
                session_id: session_id.clone(),
            },
        )
        .await
        .expect("historical heartbeat should persist");
    drop(first_runtime);

    // Recreating the persistent relay event runtime models the transport portion of a
    // kernel restart. A separate process-restart drill remains outside this unit test.
    let restarted_runtime = Arc::new(
        RelayEventRuntime::new(&counter_path)
            .expect("persistent relay event runtime should reload"),
    );
    let (outgoing_tx, _priority_rx, mut event_rx) =
        RelayOutgoingSender::channel(RELAY_OUTGOING_QUEUE_LIMIT);
    let subscription_private_key = relay_crypto::generate_private_key_base64();
    let subscription_public_key =
        relay_crypto::public_key_from_private_key_base64(&subscription_private_key)
            .expect("subscription public key should derive");

    replay_recent_relay_events(
        &restarted_runtime,
        &router,
        &outgoing_tx,
        "subscription-recreated-runtime",
        &subscription_public_key,
        &session_id,
        &attachment_id,
        Some(cursor.event_id),
    )
    .await
    .expect("persisted events should replay after runtime recreation");

    let replayed_snapshot =
        decrypt_relay_event_from_channel(&mut event_rx, &subscription_private_key).await;
    assert_eq!(replayed_snapshot.0, historical_snapshot.event_id);
    assert_eq!(
        replayed_snapshot.1["event"],
        serde_json::json!("session_snapshot")
    );
    assert_eq!(
        replayed_snapshot.1["agent_activity_revision"],
        serde_json::json!(HISTORICAL_ACTIVITY_REVISION)
    );

    let replayed_heartbeat =
        decrypt_relay_event_from_channel(&mut event_rx, &subscription_private_key).await;
    assert_eq!(replayed_heartbeat.0, historical_heartbeat.event_id);
    assert_eq!(
        replayed_heartbeat.1["event"],
        serde_json::json!("heartbeat")
    );

    let replay_boundary =
        decrypt_relay_event_from_channel(&mut event_rx, &subscription_private_key).await;
    assert_eq!(
        replay_boundary.1["event"],
        serde_json::json!("transport_resumed")
    );
    assert!(replay_boundary.0 > historical_heartbeat.event_id);
    assert_eq!(
        replay_boundary.1["resumed_from_event_id"],
        serde_json::json!(cursor.event_id)
    );

    let subscription_task = tokio::spawn(run_relay_subscription_loop(
        Arc::clone(&router),
        outgoing_tx,
        "subscription-recreated-runtime".to_string(),
        subscription_public_key,
        session_id,
        attachment_id,
        None,
        Arc::clone(&restarted_runtime),
        true,
        crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
    ));
    let current_snapshot = tokio::time::timeout(
        Duration::from_secs(5),
        decrypt_relay_event_from_channel(&mut event_rx, &subscription_private_key),
    )
    .await
    .expect("current post-restart snapshot should arrive");
    assert_eq!(
        current_snapshot.1["event"],
        serde_json::json!("session_snapshot")
    );
    assert_eq!(
        current_snapshot.1["agent_activity_revision"],
        serde_json::json!(current_activity_revision)
    );

    let current_heartbeat = tokio::time::timeout(
        Duration::from_secs(5),
        decrypt_relay_event_from_channel(&mut event_rx, &subscription_private_key),
    )
    .await
    .expect("current post-restart heartbeat should follow the snapshot");
    assert_eq!(current_heartbeat.1["event"], serde_json::json!("heartbeat"));
    subscription_task.abort();
    let _ = subscription_task.await;
}
