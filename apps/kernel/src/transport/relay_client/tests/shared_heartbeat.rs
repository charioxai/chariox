//! A viewer's subscriptions on one lane share one `heartbeat` per interval: any event proves the lane alive,
//! so a second subscription must not double the idle traffic.
use super::support::*;

const INTERVAL_MS: u64 = 1_000;

#[tokio::test(flavor = "multi_thread")]
async fn two_session_subscriptions_of_one_viewer_share_one_heartbeat() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
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
    let (session_id, first_attachment, second_attachment) = {
        let mut app = app.lock().await;
        let session_id =
            create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test");
        let first = attach_test_client(
            &mut app,
            &session_id,
            "viewer-a",
            ClientCapabilityLevel::MessageTransport,
        );
        let second = attach_test_client(
            &mut app,
            &session_id,
            "viewer-b",
            ClientCapabilityLevel::MessageTransport,
        );
        (session_id, first, second)
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
    let (mut socket, _) = connect_async(&url)
        .await
        .expect("viewer should connect to relay");
    let target = ClientTarget {
        daemon_id: Some(config.daemon_id.clone()),
        daemon_alias: None,
    };
    send_client_envelope(
        &mut socket,
        &RelayEnvelope::ClientConnect {
            auth_token: "secret".to_string(),
            target: target.clone(),
        },
    )
    .await;
    let _daemon_public_key = expect_client_connected(&mut socket).await;
    let private_key = relay_crypto::generate_private_key_base64();
    let public_key = relay_crypto::public_key_from_private_key_base64(&private_key)
        .expect("viewer public key should derive");
    approve_test_terminal(&config, &public_key);
    for (n, attachment_id) in [first_attachment, second_attachment]
        .into_iter()
        .enumerate()
    {
        send_client_envelope(
            &mut socket,
            &RelayEnvelope::ClientSubscribe {
                request_id: format!("sub-{n}"),
                subscription_id: format!("subscription-{n}"),
                target: target.clone(),
                session_id: session_id.clone(),
                attachment_id,
                client_public_key: public_key.clone(),
                subscription_scope: None,
                resume_from_event_id: None,
                heartbeat_interval_ms: Some(INTERVAL_MS),
            },
        )
        .await;
    }

    // Three intervals after both subscriptions started: one per interval, not one per subscription.
    let observed = tokio::time::Instant::now() + Duration::from_millis(3 * INTERVAL_MS + 400);
    let mut heartbeats = 0;
    while let Ok(event) =
        tokio::time::timeout_at(observed, expect_client_event(&mut socket, &private_key)).await
    {
        if event["event"] == serde_json::json!("heartbeat") {
            heartbeats += 1;
        }
    }
    assert!(
        (3..=5).contains(&heartbeats),
        "two subscriptions of one viewer sent {heartbeats} heartbeats in three intervals"
    );

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
