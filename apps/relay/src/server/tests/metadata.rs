use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn metadata_queries_return_live_machines_and_kernels() {
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
    let (mut daemon_socket, _) = connect_async(&url)
        .await
        .expect("daemon should connect to relay");
    let register = RelayEnvelope::DaemonRegister {
        registration: DaemonRegistration {
            auth_token: "secret".to_string(),
            daemon_id: "daemon-1".to_string(),
            machine_id: "machine-1".to_string(),
            machine_alias: Some("workstation".to_string()),
            os_name: Some("macOS".to_string()),
            kernel_started_at_ms: 10,
            daemon_alias: Some("mbp".to_string()),
            kernel_alias: Some("default".to_string()),
            public_key: "public-key".to_string(),
            capabilities: vec!["kernel_ws".to_string()],
            available_providers: vec!["opencode".to_string(), "codex".to_string()],
            provider_accounts: Vec::new(),
            accepting_remote_leases: true,
            leased_agent_count: 2,
            local_session_count: 3,
        },
    };
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&register)
                .expect("register envelope should serialize")
                .into(),
        ))
        .await
        .expect("register frame should send");
    sleep(Duration::from_millis(50)).await;

    let (mut client_socket, _) = connect_async(&url)
        .await
        .expect("client should connect to relay");
    let machines_request = RelayEnvelope::ClientMetadataRequest {
        request_id: "machines-1".to_string(),
        auth_token: "secret".to_string(),
        query: RelayMetadataQuery::ListLiveMachines,
    };
    client_socket
        .send(Message::Text(
            serde_json::to_string(&machines_request)
                .expect("machines request should serialize")
                .into(),
        ))
        .await
        .expect("machines request should send");
    let machines_payload = match client_socket.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected machines response: {other:?}"),
    };
    let machines_response: RelayEnvelope =
        serde_json::from_str(&machines_payload).expect("machines response should decode");
    match machines_response {
        RelayEnvelope::ClientMetadataResponse {
            request_id,
            machines: Some(machines),
            kernels: None,
            kernel: None,
            error: None,
        } => {
            assert_eq!(request_id, "machines-1");
            assert_eq!(machines.len(), 1);
            assert_eq!(machines[0].machine_id, "machine-1");
            assert_eq!(
                machines[0].machine_alias.as_deref(),
                Some("machine 1 (macOS)")
            );
            assert_eq!(machines[0].available_providers, vec!["codex", "opencode"]);
        }
        other => panic!("unexpected machines response envelope: {other:?}"),
    }

    let kernels_request = RelayEnvelope::ClientMetadataRequest {
        request_id: "kernels-1".to_string(),
        auth_token: "secret".to_string(),
        query: RelayMetadataQuery::ListLiveKernelsForMachine {
            machine_ref: "machine 1 (macOS)".to_string(),
        },
    };
    client_socket
        .send(Message::Text(
            serde_json::to_string(&kernels_request)
                .expect("kernels request should serialize")
                .into(),
        ))
        .await
        .expect("kernels request should send");
    let kernels_payload = match client_socket.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected kernels response: {other:?}"),
    };
    let kernels_response: RelayEnvelope =
        serde_json::from_str(&kernels_payload).expect("kernels response should decode");
    match kernels_response {
        RelayEnvelope::ClientMetadataResponse {
            request_id,
            machines: None,
            kernels: Some(kernels),
            kernel: None,
            error: None,
        } => {
            assert_eq!(request_id, "kernels-1");
            assert_eq!(kernels.len(), 1);
            assert_eq!(kernels[0].kernel_id, "daemon-1");
            assert_eq!(
                kernels[0].machine_alias.as_deref(),
                Some("machine 1 (macOS)")
            );
            assert_eq!(kernels[0].relay_alias.as_deref(), Some("machine 1 (macOS)"));
            assert_eq!(kernels[0].available_providers, vec!["opencode", "codex"]);
            assert!(kernels[0].accepting_remote_leases);
            assert_eq!(kernels[0].leased_agent_count, 2);
            assert_eq!(kernels[0].local_session_count, 3);
        }
        other => panic!("unexpected kernels response envelope: {other:?}"),
    }

    let kernel_request = RelayEnvelope::ClientMetadataRequest {
        request_id: "kernel-1".to_string(),
        auth_token: "secret".to_string(),
        query: RelayMetadataQuery::GetLiveKernel {
            kernel_ref: "default".to_string(),
        },
    };
    client_socket
        .send(Message::Text(
            serde_json::to_string(&kernel_request)
                .expect("kernel request should serialize")
                .into(),
        ))
        .await
        .expect("kernel request should send");
    let kernel_payload = match client_socket.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected kernel response: {other:?}"),
    };
    let kernel_response: RelayEnvelope =
        serde_json::from_str(&kernel_payload).expect("kernel response should decode");
    match kernel_response {
        RelayEnvelope::ClientMetadataResponse {
            request_id,
            machines: None,
            kernels: None,
            kernel: Some(kernel),
            error: None,
        } => {
            assert_eq!(request_id, "kernel-1");
            assert_eq!(kernel.kernel_id, "daemon-1");
            assert_eq!(kernel.public_key, "public-key");
        }
        other => panic!("unexpected kernel response envelope: {other:?}"),
    }

    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn scoped_tokens_route_and_list_only_within_their_realm() {
    let mut claims = BTreeMap::new();
    claims.insert(
        "daemon-a-token".to_string(),
        scoped_claim(
            "daemon-a-token",
            "daemon-a",
            RelaySubjectKind::Kernel,
            "realm-a",
            vec![RelayAction::DaemonRegister],
            None,
        ),
    );
    claims.insert(
        "daemon-b-token".to_string(),
        scoped_claim(
            "daemon-b-token",
            "daemon-b",
            RelaySubjectKind::Kernel,
            "realm-b",
            vec![RelayAction::DaemonRegister],
            None,
        ),
    );
    claims.insert(
        "client-a-token".to_string(),
        scoped_claim(
            "client-a-token",
            "client-a",
            RelaySubjectKind::Client,
            "realm-a",
            vec![RelayAction::ClientConnect, RelayAction::ClientMetadataRead],
            None,
        ),
    );
    claims.insert(
        "client-b-token".to_string(),
        scoped_claim(
            "client-b-token",
            "client-b",
            RelaySubjectKind::Client,
            "realm-b",
            vec![RelayAction::ClientConnect, RelayAction::ClientMetadataRead],
            None,
        ),
    );
    let auth_verifier =
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(claims, BTreeMap::new(), Some(10)));
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: None,
        },
        auth_verifier,
    );
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let auth_verifier = server.auth_verifier.clone();
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: None,
        },
        auth_verifier,
    );
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
    let (mut daemon_a, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon A should connect");
    let (mut daemon_b, _) = connect_async_with_retry(&url)
        .await
        .expect("daemon B should connect");
    let mut registration_a =
        test_registration_with_token("daemon-a", "machine-a", "Linux", 10, "daemon-a-token");
    registration_a.daemon_alias = Some("shared".to_string());
    registration_a.public_key = "public-key-a".to_string();
    let mut registration_b =
        test_registration_with_token("daemon-b", "machine-b", "Linux", 10, "daemon-b-token");
    registration_b.daemon_alias = Some("shared".to_string());
    registration_b.public_key = "public-key-b".to_string();
    for (socket, registration) in [
        (&mut daemon_a, registration_a),
        (&mut daemon_b, registration_b),
    ] {
        socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister { registration })
                    .expect("register should serialize")
                    .into(),
            ))
            .await
            .expect("register should send");
    }
    sleep(Duration::from_millis(50)).await;

    let (mut client_a, _) = connect_async_with_retry(&url)
        .await
        .expect("client A should connect");
    client_a
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientMetadataRequest {
                request_id: "machines-a".to_string(),
                auth_token: "client-a-token".to_string(),
                query: RelayMetadataQuery::ListLiveMachines,
            })
            .expect("metadata request should serialize")
            .into(),
        ))
        .await
        .expect("metadata request should send");
    let machines_payload = match client_a.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected machines response: {other:?}"),
    };
    match serde_json::from_str::<RelayEnvelope>(&machines_payload)
        .expect("machines response should decode")
    {
        RelayEnvelope::ClientMetadataResponse {
            machines: Some(machines),
            error: None,
            ..
        } => {
            assert_eq!(machines.len(), 1);
            assert_eq!(machines[0].machine_id, "machine-a");
        }
        other => panic!("unexpected machines response envelope: {other:?}"),
    }

    client_a
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "client-a-token".to_string(),
                target: ClientTarget {
                    daemon_id: None,
                    daemon_alias: Some("shared".to_string()),
                },
            })
            .expect("client connect should serialize")
            .into(),
        ))
        .await
        .expect("client connect should send");
    let connect_payload = match client_a.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected connect response: {other:?}"),
    };
    match serde_json::from_str::<RelayEnvelope>(&connect_payload)
        .expect("connect response should decode")
    {
        RelayEnvelope::ClientConnected {
            daemon_public_key, ..
        } => assert_eq!(daemon_public_key, "public-key-a"),
        other => panic!("unexpected connect response envelope: {other:?}"),
    }

    let (mut client_b, _) = connect_async_with_retry(&url)
        .await
        .expect("client B should connect");
    client_b
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "client-b-token".to_string(),
                target: ClientTarget {
                    daemon_id: None,
                    daemon_alias: Some("shared".to_string()),
                },
            })
            .expect("client connect should serialize")
            .into(),
        ))
        .await
        .expect("client connect should send");
    let connect_payload = match client_b.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected connect response: {other:?}"),
    };
    match serde_json::from_str::<RelayEnvelope>(&connect_payload)
        .expect("connect response should decode")
    {
        RelayEnvelope::ClientConnected {
            daemon_public_key, ..
        } => assert_eq!(daemon_public_key, "public-key-b"),
        other => panic!("unexpected connect response envelope: {other:?}"),
    }

    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn accepted_daemon_socket_is_closed_when_initial_token_expires() {
    let now_ms = test_current_unix_ms();
    let mut daemon_claim = scoped_claim(
        "daemon-token",
        "daemon-1",
        RelaySubjectKind::Kernel,
        "realm-a",
        vec![RelayAction::DaemonRegister],
        None,
    );
    daemon_claim.issued_at_ms = now_ms.saturating_sub(1);
    daemon_claim.expires_at_ms = now_ms + 100;
    let mut claims = BTreeMap::new();
    claims.insert("daemon-token".to_string(), daemon_claim);
    let auth_verifier =
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(claims, BTreeMap::new(), None));
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: None,
        },
        auth_verifier,
    );
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let auth_verifier = server.auth_verifier.clone();
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: None,
        },
        auth_verifier,
    );
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
        .expect("daemon should connect");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration_with_token(
                    "daemon-1",
                    "machine-1",
                    "Linux",
                    10,
                    "daemon-token",
                ),
            })
            .expect("register should serialize")
            .into(),
        ))
        .await
        .expect("register should send");

    sleep(Duration::from_millis(75)).await;
    {
        let guard = registry.read().await;
        assert_eq!(guard.daemon_count(), 1);
        assert_eq!(guard.peer_count(), 1);
    }
    assert_relay_close(&mut daemon_socket).await;

    let _ = daemon_socket.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn accepted_client_socket_is_closed_when_initial_token_expires() {
    let now_ms = test_current_unix_ms();
    let mut daemon_claim = scoped_claim(
        "daemon-token",
        "daemon-1",
        RelaySubjectKind::Kernel,
        "realm-a",
        vec![RelayAction::DaemonRegister],
        None,
    );
    daemon_claim.issued_at_ms = now_ms.saturating_sub(1);
    daemon_claim.expires_at_ms = now_ms + 1_000;
    let mut client_claim = scoped_claim(
        "client-token",
        "client-1",
        RelaySubjectKind::Client,
        "realm-a",
        vec![RelayAction::ClientConnect],
        Some(vec!["daemon-1"]),
    );
    client_claim.issued_at_ms = now_ms.saturating_sub(1);
    client_claim.expires_at_ms = now_ms + 150;
    let mut claims = BTreeMap::new();
    claims.insert("daemon-token".to_string(), daemon_claim);
    claims.insert("client-token".to_string(), client_claim);
    let auth_verifier =
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(claims, BTreeMap::new(), None));
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: None,
        },
        auth_verifier,
    );
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let auth_verifier = server.auth_verifier.clone();
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: None,
        },
        auth_verifier,
    );
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
        .expect("daemon should connect");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration_with_token(
                    "daemon-1",
                    "machine-1",
                    "Linux",
                    10,
                    "daemon-token",
                ),
            })
            .expect("register should serialize")
            .into(),
        ))
        .await
        .expect("register should send");
    sleep(Duration::from_millis(50)).await;

    let (mut client_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("client should connect");
    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "client-token".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
            })
            .expect("client connect should serialize")
            .into(),
        ))
        .await
        .expect("client connect should send");
    let connect_payload = match client_socket.next().await {
        Some(Ok(Message::Text(text))) => text,
        other => panic!("unexpected connect response: {other:?}"),
    };
    match serde_json::from_str::<RelayEnvelope>(&connect_payload)
        .expect("connect response should decode")
    {
        RelayEnvelope::ClientConnected {
            daemon_public_key, ..
        } => assert_eq!(daemon_public_key, "public-key-daemon-1"),
        other => panic!("unexpected connect response envelope: {other:?}"),
    }

    sleep(Duration::from_millis(75)).await;
    assert_relay_close(&mut client_socket).await;

    let _ = client_socket.close(None).await;
    let _ = daemon_socket.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_token_is_rejected_for_new_client_connection() {
    let mut claims = BTreeMap::new();
    claims.insert(
        "daemon-token".to_string(),
        scoped_claim(
            "daemon-token",
            "daemon-1",
            RelaySubjectKind::Kernel,
            "realm-a",
            vec![RelayAction::DaemonRegister],
            None,
        ),
    );
    let mut expired_client_claim = scoped_claim(
        "expired-client-token",
        "client-1",
        RelaySubjectKind::Client,
        "realm-a",
        vec![RelayAction::ClientConnect],
        Some(vec!["daemon-1"]),
    );
    expired_client_claim.expires_at_ms = 5;
    claims.insert("expired-client-token".to_string(), expired_client_claim);
    let auth_verifier =
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(claims, BTreeMap::new(), Some(10)));
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: None,
        },
        auth_verifier,
    );
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");

    let auth_verifier = server.auth_verifier.clone();
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: None,
        },
        auth_verifier,
    );
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
        .expect("daemon should connect");
    daemon_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration_with_token(
                    "daemon-1",
                    "machine-1",
                    "Linux",
                    10,
                    "daemon-token",
                ),
            })
            .expect("register should serialize")
            .into(),
        ))
        .await
        .expect("register should send");
    sleep(Duration::from_millis(50)).await;

    let (mut client_socket, _) = connect_async_with_retry(&url)
        .await
        .expect("client should connect");
    client_socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::ClientConnect {
                auth_token: "expired-client-token".to_string(),
                target: ClientTarget {
                    daemon_id: Some("daemon-1".to_string()),
                    daemon_alias: None,
                },
            })
            .expect("client connect should serialize")
            .into(),
        ))
        .await
        .expect("client connect should send");

    match timeout(Duration::from_millis(500), client_socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            let envelope =
                serde_json::from_str::<RelayEnvelope>(&text).expect("relay response should decode");
            assert!(
                !matches!(envelope, RelayEnvelope::ClientConnected { .. }),
                "expired token must not connect a client"
            );
        }
        Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => {}
        Ok(other) => panic!("unexpected expired-token response: {other:?}"),
        Err(_) => panic!("expired token did not close or reject promptly"),
    }

    let _ = daemon_socket.close(None).await;
    let _ = shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
