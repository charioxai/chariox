use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn daemon_display_tunnel_registration_is_tracked_revoked_and_disconnected() {
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

    let server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let registry = server.registry();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("relay server should run");
    });

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut daemon_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon should connect to relay");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration("daemon-1", "machine-1", "macOS", 10),
            })
            .expect("register envelope should serialize")
            .into(),
        ))
        .await
        .expect("register should send");

    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRegister {
                registration: RelayDisplayTunnelRegistration {
                    tunnel_id: "display-opaque-1".to_string(),
                    expires_at_ms: u64::MAX,
                    capabilities: vec!["view".to_string(), "keyboard".to_string()],
                },
            })
            .expect("display tunnel register should serialize")
            .into(),
        ))
        .await
        .expect("display tunnel register should send");

    let registered_payload = match daemon_socket.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected display tunnel registration response: {other:?}"),
    };
    match serde_json::from_str::<RelayEnvelope>(&registered_payload)
        .expect("display tunnel registration response should decode")
    {
        RelayEnvelope::DaemonDisplayTunnelRegistered {
            tunnel_id,
            expires_at_ms,
            error: None,
        } => {
            assert_eq!(tunnel_id, "display-opaque-1");
            assert_eq!(expires_at_ms, u64::MAX);
        }
        other => panic!("unexpected display tunnel registration envelope: {other:?}"),
    }

    {
        let guard = registry.read().await;
        assert_eq!(guard.display_tunnel_count(), 1);
    }

    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRevoke {
                tunnel_id: "display-opaque-1".to_string(),
            })
            .expect("display tunnel revoke should serialize")
            .into(),
        ))
        .await
        .expect("display tunnel revoke should send");
    sleep(Duration::from_millis(50)).await;
    assert_eq!(registry.read().await.display_tunnel_count(), 0);

    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRegister {
                registration: RelayDisplayTunnelRegistration {
                    tunnel_id: "display-opaque-2".to_string(),
                    expires_at_ms: u64::MAX,
                    capabilities: vec!["view".to_string()],
                },
            })
            .expect("display tunnel register should serialize")
            .into(),
        ))
        .await
        .expect("second display tunnel register should send");
    let _ = daemon_socket.next().await;
    assert_eq!(registry.read().await.display_tunnel_count(), 1);

    daemon_socket
        .close(None)
        .await
        .expect("daemon should close");
    sleep(Duration::from_millis(50)).await;
    assert_eq!(registry.read().await.display_tunnel_count(), 0);

    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn display_http_route_resolves_registered_tunnel_state() {
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

    let server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let registry = server.registry();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("relay server should run");
    });

    let missing = relay_http_get(addr, "/display/missing/vnc.html").await;
    assert!(missing.starts_with("HTTP/1.1 404 Not Found"));

    {
        let mut guard = registry.write().await;
        guard.register_display_tunnel(
            DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-expired"),
            "expired".to_string(),
            1,
            Vec::new(),
        );
        guard.register_display_tunnel(
            DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-disconnected"),
            "disconnected".to_string(),
            u64::MAX,
            Vec::new(),
        );
    }
    let expired = relay_http_get(addr, "/display/expired/vnc.html").await;
    assert!(expired.starts_with("HTTP/1.1 410 Gone"));

    let disconnected = relay_http_get(addr, "/display/disconnected/vnc.html").await;
    assert!(disconnected.starts_with("HTTP/1.1 502 Bad Gateway"));

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut daemon_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon should connect to relay");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration("daemon-live", "machine-1", "macOS", 10),
            })
            .expect("register envelope should serialize")
            .into(),
        ))
        .await
        .expect("register should send");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRegister {
                registration: RelayDisplayTunnelRegistration {
                    tunnel_id: "live".to_string(),
                    expires_at_ms: u64::MAX,
                    capabilities: vec!["view".to_string()],
                },
            })
            .expect("display tunnel register should serialize")
            .into(),
        ))
        .await
        .expect("display tunnel register should send");
    let _ = daemon_socket.next().await;

    let response_task =
        tokio::spawn(async move { relay_http_get(addr, "/display/live/vnc.html").await });
    let open_payload = match daemon_socket.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected display tunnel open request: {other:?}"),
    };
    let stream_id = match serde_json::from_str::<RelayEnvelope>(&open_payload)
        .expect("display tunnel open should decode")
    {
        RelayEnvelope::DaemonDisplayTunnelOpen { request } => {
            assert_eq!(request.tunnel_id, "live");
            assert_eq!(request.method, "GET");
            assert_eq!(request.path, "/display/live/vnc.html");
            request.stream_id
        }
        other => panic!("unexpected display tunnel open envelope: {other:?}"),
    };
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelResponseStart {
                response: crate::protocol::RelayDisplayTunnelResponseStart {
                    stream_id: stream_id.clone(),
                    status: 200,
                    headers: vec![RelayDisplayTunnelHeader {
                        name: "content-type".to_string(),
                        value: "text/plain".to_string(),
                    }],
                },
            })
            .expect("display response start should serialize")
            .into(),
        ))
        .await
        .expect("display response start should send");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelChunk {
                chunk: RelayDisplayTunnelStreamChunk {
                    stream_id: stream_id.clone(),
                    data: "aGVsbG8=".to_string(),
                    message_kind: None,
                },
            })
            .expect("display chunk should serialize")
            .into(),
        ))
        .await
        .expect("display chunk should send");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelClose {
                stream_id,
                error: None,
            })
            .expect("display close should serialize")
            .into(),
        ))
        .await
        .expect("display close should send");
    let live = response_task
        .await
        .expect("display response task should join");
    assert!(live.starts_with("HTTP/1.1 200 OK"));
    assert!(live.contains("content-type: text/plain"));
    assert!(live.ends_with("\r\n\r\nhello"));

    let _ = daemon_socket.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn display_http_request_closes_promptly_when_daemon_disconnects() {
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

    let server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("relay server should run");
    });

    let url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut daemon_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon should connect to relay");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration("daemon-live", "machine-1", "macOS", 10),
            })
            .expect("register envelope should serialize")
            .into(),
        ))
        .await
        .expect("register should send");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRegister {
                registration: RelayDisplayTunnelRegistration {
                    tunnel_id: "live-disconnect".to_string(),
                    expires_at_ms: u64::MAX,
                    capabilities: vec!["view".to_string()],
                },
            })
            .expect("display tunnel register should serialize")
            .into(),
        ))
        .await
        .expect("display tunnel register should send");
    let _ = daemon_socket.next().await;

    let response_task =
        tokio::spawn(
            async move { relay_http_get(addr, "/display/live-disconnect/vnc.html").await },
        );
    match daemon_socket.next().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
            .expect("display tunnel open should decode")
        {
            RelayEnvelope::DaemonDisplayTunnelOpen { request } => {
                assert_eq!(request.tunnel_id, "live-disconnect");
            }
            other => panic!("unexpected display tunnel open envelope: {other:?}"),
        },
        other => panic!("unexpected display tunnel open request: {other:?}"),
    }

    daemon_socket
        .close(None)
        .await
        .expect("daemon socket should close");
    let response = response_task
        .await
        .expect("display response task should join");
    assert!(response.starts_with("HTTP/1.1 502 Bad Gateway"));
    assert!(response.contains("display tunnel failed: target daemon disconnected from relay"));

    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn health_endpoint_reports_healthy_and_draining_status() {
    let healthy_server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = healthy_server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let healthy_server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        healthy_server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("relay server should run");
    });

    let healthy = relay_http_get(addr, "/healthz").await;
    assert!(healthy.starts_with("HTTP/1.1 200 OK"));
    assert!(healthy.contains("\"status\":\"healthy\""));
    assert!(healthy.contains("\"draining\":false"));
    assert!(healthy.contains("\"backpressure\""));
    assert!(healthy.contains("\"target_queue_full_count\":0"));
    assert!(healthy.contains("\"slow_subscription_close_count\":0"));
    let _ = shutdown_tx.send(());
    server_task.await.expect("healthy server task should join");

    let draining_server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    });
    let listener = draining_server
        .bind_listener()
        .await
        .expect("draining relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let draining_server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    draining_server.set_draining(true);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        draining_server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("draining relay server should run");
    });

    let draining_health = relay_http_get(addr, "/healthz").await;
    assert!(draining_health.starts_with("HTTP/1.1 200 OK"));
    assert!(draining_health.contains("\"status\":\"draining\""));
    assert!(draining_health.contains("\"draining\":true"));

    let draining_ready = relay_http_get(addr, "/readyz").await;
    assert!(draining_ready.starts_with("HTTP/1.1 503 Service Unavailable"));
    assert!(draining_ready.contains("\r\nretry-after: 5\r\n"));
    assert!(draining_ready.contains("\"retry_after_seconds\":5"));
    assert!(draining_ready.contains("\"status\":\"draining\""));
    assert!(draining_ready.contains("\"draining\":true"));

    let websocket_error = connect_async(format!("ws://{addr}"))
        .await
        .expect_err("draining relay should reject new websocket admissions");
    assert!(
        websocket_error.to_string().contains("503"),
        "unexpected websocket error: {websocket_error}"
    );

    let websocket_response = relay_http_get_until_close_or_reset(addr, "/runtime").await;
    assert!(websocket_response.starts_with("HTTP/1.1 503 Service Unavailable"));
    assert!(websocket_response.contains("\r\nretry-after: 5\r\n"));
    assert!(websocket_response.contains("\"retry_after_seconds\":5"));

    let _ = shutdown_tx.send(());
    server_task.await.expect("draining server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn display_websocket_route_bridges_browser_and_daemon_frames() {
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

    let server = RelayServer::new(RelayConfig {
        host: addr.ip().to_string(),
        port: addr.port(),
        shared_token: Some("secret".to_string()),
    });
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("relay server should run");
    });

    let relay_url = format!("ws://{}:{}", addr.ip(), addr.port());
    let (mut daemon_socket, _) = connect_async_with_retry(&relay_url)
        .await
        .expect("daemon should connect to relay");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration("daemon-live", "machine-1", "macOS", 10),
            })
            .expect("register envelope should serialize")
            .into(),
        ))
        .await
        .expect("register should send");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRegister {
                registration: RelayDisplayTunnelRegistration {
                    tunnel_id: "live".to_string(),
                    expires_at_ms: u64::MAX,
                    capabilities: vec!["view".to_string(), "keyboard".to_string()],
                },
            })
            .expect("display tunnel register should serialize")
            .into(),
        ))
        .await
        .expect("display tunnel register should send");
    let _ = daemon_socket.next().await;

    let browser_url = format!("ws://{}:{}/display/live/websockify", addr.ip(), addr.port());
    let browser_task = tokio::spawn(async move {
        connect_async(browser_url)
            .await
            .expect("browser display websocket should connect")
            .0
    });
    let open_payload = match daemon_socket.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected display tunnel websocket open: {other:?}"),
    };
    let stream_id = match serde_json::from_str::<RelayEnvelope>(&open_payload)
        .expect("display websocket open should decode")
    {
        RelayEnvelope::DaemonDisplayTunnelOpen { request } => {
            assert_eq!(request.tunnel_id, "live");
            assert_eq!(request.path, "/display/live/websockify");
            assert!(request
                .headers
                .iter()
                .any(|header| header.name.eq_ignore_ascii_case("upgrade")
                    && header.value.eq_ignore_ascii_case("websocket")));
            request.stream_id
        }
        other => panic!("unexpected display tunnel open envelope: {other:?}"),
    };
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelResponseStart {
                response: crate::protocol::RelayDisplayTunnelResponseStart {
                    stream_id: stream_id.clone(),
                    status: 101,
                    headers: Vec::new(),
                },
            })
            .expect("display websocket response start should serialize")
            .into(),
        ))
        .await
        .expect("display websocket response start should send");
    let mut browser_socket = browser_task.await.expect("browser task should join");

    browser_socket
        .send(Message::Binary(Vec::from("from-browser").into()))
        .await
        .expect("browser frame should send");
    let client_chunk_payload = match daemon_socket.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected display tunnel client chunk: {other:?}"),
    };
    match serde_json::from_str::<RelayEnvelope>(&client_chunk_payload)
        .expect("display client chunk should decode")
    {
        RelayEnvelope::DaemonDisplayTunnelClientChunk { chunk } => {
            assert_eq!(chunk.stream_id, stream_id);
            assert_eq!(chunk.message_kind.as_deref(), Some("binary"));
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(chunk.data)
                .expect("chunk data should decode");
            assert_eq!(decoded, b"from-browser");
        }
        other => panic!("unexpected display client chunk envelope: {other:?}"),
    };

    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelChunk {
                chunk: RelayDisplayTunnelStreamChunk {
                    stream_id: stream_id.clone(),
                    data: base64::engine::general_purpose::STANDARD.encode("from-daemon"),
                    message_kind: Some("binary".to_string()),
                },
            })
            .expect("display daemon chunk should serialize")
            .into(),
        ))
        .await
        .expect("display daemon chunk should send");
    match browser_socket.next().await {
        Some(Ok(Message::Binary(data))) => assert_eq!(data.as_ref(), b"from-daemon"),
        other => panic!("unexpected browser display frame: {other:?}"),
    }

    let _ = browser_socket.close(None).await;
    let _ = daemon_socket.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
