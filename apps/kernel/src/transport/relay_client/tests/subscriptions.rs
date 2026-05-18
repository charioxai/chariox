#![allow(unused_imports)]
use super::support::*;

#[tokio::test(flavor = "multi_thread")]
async fn proxied_session_subscriptions_are_forwarded_through_relay() {
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
    drop(listener);

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
                .run_until(async {
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
    drop(listener);

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
                .run_until(async {
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

    let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel();
    let subscription_private_key = relay_crypto::generate_private_key_base64();
    let subscription_public_key =
        relay_crypto::public_key_from_private_key_base64(&subscription_private_key)
            .expect("subscription public key should derive");

    replay_recent_relay_events(
        &event_runtime,
        &router,
        &app,
        &outgoing_tx,
        "subscription-1",
        &subscription_public_key,
        &created_session_id,
        &attachment_id,
        Some(first.event_id),
    )
    .await
    .expect("stale replay should emit recovery events");

    let gap = decrypt_relay_event_from_channel(&mut outgoing_rx, &subscription_private_key).await;
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

    let snapshot =
        decrypt_relay_event_from_channel(&mut outgoing_rx, &subscription_private_key).await;
    assert_eq!(snapshot.0, second.event_id + 2);
    assert_eq!(snapshot.1["event"], serde_json::json!("session_snapshot"));
    assert_eq!(
        snapshot.1["session"]["id"],
        serde_json::json!(created_session_id)
    );
}
