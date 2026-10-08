use super::*;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, WebSocketStream};

#[tokio::test]
async fn persistent_peer_relay_refusal_preserves_retryability_without_peer_authority() {
    let home = crate::config::DaemonConfig::for_tests();
    let worker = crate::config::DaemonConfig::for_tests();
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (sender, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(4);
    state
        .write()
        .await
        .test_set_connected_sender(sender, "ws://fixture");
    let waiter = peer_client::enqueue_peer_request_to_known_kernel_via_relay_authorized(
        &home,
        &state,
        ClientTarget {
            daemon_id: Some("worker".into()),
            daemon_alias: None,
        },
        &worker.relay_public_key,
        RelayPeerRequest::DestroyLeasedAgent {
            leased_agent_id: "bound-agent".into(),
        },
        Duration::from_secs(3),
        || Ok(()),
    )
    .await
    .unwrap();
    let RelayEnvelope::DaemonPeerRequest { request_id, .. } = priority_rx.recv().await.unwrap()
    else {
        panic!("expected peer request")
    };
    // MP-11: a relay refusal carries no peer identity or encrypted success.
    peer_client::resolve_pending_peer_response(
        &state,
        request_id,
        RelayPeerResponseEnvelope {
            from_daemon_id: String::new(),
            encrypted_response: None,
            error: Some(chariox_relay::protocol::RelayError {
                code: "target_not_connected".into(),
                message: "target daemon is not connected to relay".into(),
                retryable: true,
            }),
        },
    )
    .await;
    assert!(
        matches!(waiter.wait().await.unwrap_err(), DaemonError::RelayTransport {
        code, retryable: true, ..
    } if code == "target_not_connected")
    );
    assert!(state
        .read()
        .await
        .pinned_peer_public_key("worker")
        .is_none());
}

#[tokio::test]
async fn known_peer_request_rechecks_authority_after_enqueue_lock_wait() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let home = crate::config::DaemonConfig::for_tests();
    let worker = crate::config::DaemonConfig::for_tests();
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (sender, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(4);
    let mut guard = state.write().await;
    guard.test_set_connected_sender(sender, "ws://fixture");
    let authorized = AtomicBool::new(true);
    let pending = peer_client::enqueue_peer_request_to_known_kernel_via_relay_authorized(
        &home,
        &state,
        ClientTarget {
            daemon_id: Some("worker".into()),
            daemon_alias: None,
        },
        &worker.relay_public_key,
        RelayPeerRequest::Ping {
            value: "must-not-send".into(),
        },
        Duration::from_secs(3),
        || {
            if authorized.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(DaemonError::LocalTransport {
                    operation: "authorize test peer request",
                    message: "revoked".into(),
                })
            }
        },
    );
    tokio::pin!(pending);
    // Poll to the held write lock before revoking; no timer determines the race.
    std::future::poll_fn(|cx| {
        use std::future::Future;
        assert!(pending.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    authorized.store(false, Ordering::SeqCst);
    drop(guard);
    assert!(matches!(pending.await, Err(error) if error.to_string().contains("revoked")));
    assert!(
        priority_rx.try_recv().is_err(),
        "revoked request was enqueued"
    );
    assert!(state.read().await.pending_peer_requests.is_empty());
}

// The relay's claimed sender and the encrypted sender must both match discovery.
// This peer-client test injects hostile wire responses, not mocked kernel code.
#[tokio::test]
async fn temporary_peer_request_rejects_response_from_another_key() {
    rejects_mismatched_identity(true).await;
}

#[tokio::test]
async fn temporary_peer_request_rejects_response_from_another_kernel() {
    rejects_mismatched_identity(false).await;
}

async fn receive(socket: &mut WebSocketStream<TcpStream>) -> RelayEnvelope {
    let message = timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

async fn send(socket: &mut WebSocketStream<TcpStream>, envelope: RelayEnvelope) {
    socket
        .send(Message::Text(
            serde_json::to_string(&envelope).unwrap().into(),
        ))
        .await
        .unwrap();
}

async fn rejects_mismatched_identity(wrong_key: bool) {
    let mut home = crate::config::DaemonConfig::for_tests();
    let worker = crate::config::DaemonConfig::for_tests();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    home.relay_url = Some(format!("ws://{}", listener.local_addr().unwrap()));
    home.relay_token = Some("identity-fixture".into());
    let home_key = home.relay_public_key.clone();
    let server = tokio::spawn(async move {
        let mut metadata = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        let RelayEnvelope::ClientMetadataRequest { request_id, .. } = receive(&mut metadata).await
        else {
            panic!("expected discovery request");
        };
        let presence = serde_json::from_value(serde_json::json!({
            "kernel_id":"worker", "machine_id":"slice:fixture", "public_key":worker.relay_public_key
        }))
        .unwrap();
        send(
            &mut metadata,
            RelayEnvelope::ClientMetadataResponse {
                request_id,
                machines: None,
                kernels: None,
                kernel: Some(presence),
                error: None,
            },
        )
        .await;
        assert!(matches!(metadata.next().await, Some(Ok(Message::Close(_)))));
        let _ = metadata.close(None).await;
        let mut socket = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        assert!(matches!(
            receive(&mut socket).await,
            RelayEnvelope::DaemonRegister { .. }
        ));
        let RelayEnvelope::DaemonPeerRequest {
            request_id,
            encrypted_request,
            ..
        } = receive(&mut socket).await
        else {
            panic!("expected encrypted peer request");
        };
        relay_crypto::decrypt_payload_for_private_key(
            &worker.relay_private_key,
            &encrypted_request,
        )
        .unwrap();
        let response_key = if wrong_key {
            relay_crypto::generate_private_key_base64()
        } else {
            worker.relay_private_key
        };
        let response = RelayPeerResponse::Pong {
            value: "reply".into(),
            daemon_id: "worker".into(),
            relay_peer_protocol_version: None,
        };
        send(
            &mut socket,
            RelayEnvelope::DaemonPeerResponse {
                request_id,
                from_daemon_id: if wrong_key {
                    "worker".into()
                } else {
                    "other-worker".into()
                },
                encrypted_response: Some(
                    relay_crypto::encrypt_payload_for_peer(
                        &response_key,
                        &home_key,
                        &serde_json::to_vec(&response).unwrap(),
                    )
                    .unwrap(),
                ),
                error: None,
            },
        )
        .await;
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        let _ = socket.close(None).await;
    });
    let response = send_peer_request_via_temporary_connection_with_timeout(
        &home,
        ClientTarget {
            daemon_id: Some("worker".into()),
            daemon_alias: None,
        },
        RelayPeerRequest::Ping {
            value: "request".into(),
        },
        Duration::from_secs(3),
    )
    .await;
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    let error = response.expect_err("mismatched response identity must not be accepted");
    assert!(
        error
            .to_string()
            .contains("peer response identity mismatch"),
        "{error}"
    );
}

#[tokio::test]
async fn temporary_peer_request_rechecks_authority_after_discovery_before_send() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut home = crate::config::DaemonConfig::for_tests();
    let worker = crate::config::DaemonConfig::for_tests();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    home.relay_url = Some(format!("ws://{}", listener.local_addr().unwrap()));
    home.relay_token = Some("authorization-fixture".into());
    let server = tokio::spawn(async move {
        let mut metadata = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        let RelayEnvelope::ClientMetadataRequest { request_id, .. } = receive(&mut metadata).await
        else {
            panic!("expected discovery")
        };
        let presence = serde_json::from_value(serde_json::json!({"kernel_id":"worker", "machine_id":"fixture-machine", "public_key":worker.relay_public_key})).unwrap();
        send(
            &mut metadata,
            RelayEnvelope::ClientMetadataResponse {
                request_id,
                machines: None,
                kernels: None,
                kernel: Some(presence),
                error: None,
            },
        )
        .await;
        assert!(matches!(metadata.next().await, Some(Ok(Message::Close(_)))));
        let _ = metadata.close(None).await;
        listener
    });
    let checks = AtomicUsize::new(0);
    let result = send_peer_request_via_temporary_connection_authorized(
        &home,
        ClientTarget {
            daemon_id: Some("worker".into()),
            daemon_alias: None,
        },
        RelayPeerRequest::Ping {
            value: "must-not-send".into(),
        },
        Duration::from_secs(3),
        || {
            if checks.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(())
            } else {
                Err(DaemonError::LocalTransport {
                    operation: "peer request authorization",
                    message: "test grant revoked".into(),
                })
            }
        },
    )
    .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("test grant revoked"));
    assert_eq!(checks.load(Ordering::SeqCst), 2);
    let listener = server.await.unwrap().into_std().unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "invalid authority opened a peer socket"
    );
}

#[tokio::test]
async fn temporary_peer_request_connection_is_capped_independently_of_card_deadline() {
    for deadline in [Some(Duration::from_secs(315)), None] {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_request_timeout_ms = 500;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        config.relay_url = Some(format!("ws://{}", listener.local_addr().unwrap()));
        config.relay_token = Some("connect-deadline-fixture".into());
        let peer_key = relay_crypto::public_key_from_private_key_base64(
            &relay_crypto::generate_private_key_base64(),
        )
        .unwrap();
        let (release, released) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut metadata = accept_async(listener.accept().await.unwrap().0)
                .await
                .unwrap();
            let RelayEnvelope::ClientMetadataRequest { request_id, .. } =
                receive(&mut metadata).await
            else {
                panic!("expected metadata discovery");
            };
            send(&mut metadata, RelayEnvelope::ClientMetadataResponse {
                request_id, machines: None, kernels: None,
                kernel: Some(serde_json::from_value(serde_json::json!({
                    "kernel_id": "worker", "machine_id": "slice:fixture", "public_key": peer_key
                })).unwrap()), error: None,
            }).await;
            assert!(matches!(metadata.next().await, Some(Ok(Message::Close(_)))));
            let _ = metadata.close(None).await;
            // TCP accepts, but the peer WebSocket handshake never finishes.
            let _stalled_socket = listener.accept().await.unwrap().0;
            let _ = released.await;
        });
        let error = timeout(
            Duration::from_secs(3),
            send_peer_request_via_temporary_connection_with_optional_timeout(
                &config,
                ClientTarget {
                    daemon_id: Some("worker".into()),
                    daemon_alias: None,
                },
                RelayPeerRequest::Ping {
                    value: "connect deadline".into(),
                },
                deadline,
            ),
        )
        .await
        .expect("connect must not wait for a human deadline")
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("connect temporary relay peer socket"),
            "{error}"
        );
        assert!(error.to_string().contains("500ms"), "{error}");
        let _ = release.send(());
        server.await.unwrap();
    }
}

// Inject the actual pending-response envelope, covering the persistent path as
// well as the classifier shared by temporary peer connections.
#[tokio::test]
async fn cleanup_absence_requires_matching_non_retryable_target_reply() {
    for (from, code, retryable, accepted) in [
        ("worker", "leased_agent_not_found", false, true),
        ("other-worker", "leased_agent_not_found", false, false),
        ("", "leased_agent_not_found", false, false),
        ("worker", "leased_agent_not_found", true, false),
        ("worker", "execution_lease_not_found", false, false),
        ("worker", "target_disconnected", true, false),
        ("worker", "unauthorized", false, false),
    ] {
        let home = crate::config::DaemonConfig::for_tests();
        let worker = crate::config::DaemonConfig::for_tests();
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (sender, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(4);
        state
            .write()
            .await
            .test_set_connected_sender(sender, "ws://fixture");
        let waiter = peer_client::enqueue_peer_request_to_known_kernel_via_relay_authorized(
            &home,
            &state,
            ClientTarget {
                daemon_id: Some("worker".into()),
                daemon_alias: None,
            },
            &worker.relay_public_key,
            RelayPeerRequest::DestroyLeasedAgent {
                leased_agent_id: "bound-agent".into(),
            },
            Duration::from_secs(3),
            || Ok(()),
        )
        .await
        .unwrap();
        let RelayEnvelope::DaemonPeerRequest { request_id, .. } = priority_rx.recv().await.unwrap()
        else {
            panic!("request");
        };
        peer_client::resolve_pending_peer_response(
            &state,
            request_id,
            RelayPeerResponseEnvelope {
                from_daemon_id: from.into(),
                encrypted_response: None,
                error: Some(chariox_relay::protocol::RelayError {
                    code: code.into(),
                    message: "synthetic".into(),
                    retryable,
                }),
            },
        )
        .await;
        let error = waiter.wait().await.unwrap_err();
        assert_eq!(
            matches!(error, DaemonError::RelayPeerCleanupAbsent { ref worker_kernel_id, ref resource_id, code: "leased_agent_not_found" }
            if worker_kernel_id == "worker" && resource_id == "bound-agent"),
            accepted,
            "source={from} code={code} retryable={retryable}: {error}"
        );
    }
}

#[test]
fn cleanup_absence_is_never_promoted_for_unbound_target_or_non_cleanup_request() {
    let error = || chariox_relay::protocol::RelayError {
        code: "leased_agent_not_found".into(),
        message: "synthetic".into(),
        retryable: false,
    };
    let destroy = RelayPeerRequest::DestroyLeasedAgent {
        leased_agent_id: "bound-agent".into(),
    };
    assert!(matches!(
        cleanup_peer_error(
            "test",
            error(),
            cleanup_request_resource(&destroy),
            None,
            "worker"
        ),
        DaemonError::RelayTransport { .. }
    ));
    assert!(matches!(
        cleanup_peer_error(
            "test",
            error(),
            cleanup_request_resource(&RelayPeerRequest::Ping {
                value: "test".into()
            }),
            Some("worker"),
            "worker"
        ),
        DaemonError::RelayTransport { .. }
    ));
}

#[tokio::test]
async fn persistent_peer_cleanup_rejects_encrypted_ack_from_another_identity() {
    for wrong_key in [true, false] {
        let home = crate::config::DaemonConfig::for_tests();
        let worker = crate::config::DaemonConfig::for_tests();
        let other = crate::config::DaemonConfig::for_tests();
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (sender, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(4);
        state
            .write()
            .await
            .test_set_connected_sender(sender, "ws://fixture");
        let waiter = peer_client::enqueue_peer_request_to_known_kernel_via_relay_authorized(
            &home,
            &state,
            ClientTarget {
                daemon_id: Some("worker".into()),
                daemon_alias: None,
            },
            &worker.relay_public_key,
            RelayPeerRequest::DestroyLeasedAgent {
                leased_agent_id: "bound-agent".into(),
            },
            Duration::from_secs(3),
            || Ok(()),
        )
        .await
        .unwrap();
        let RelayEnvelope::DaemonPeerRequest { request_id, .. } = priority_rx.recv().await.unwrap()
        else {
            panic!("request");
        };
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            if wrong_key {
                &other.relay_private_key
            } else {
                &worker.relay_private_key
            },
            &home.relay_public_key,
            &serde_json::to_vec(&RelayPeerResponse::LeasedAgentDestroyed {
                leased_agent_id: "bound-agent".into(),
            })
            .unwrap(),
        )
        .unwrap();
        peer_client::resolve_pending_peer_response(
            &state,
            request_id,
            RelayPeerResponseEnvelope {
                from_daemon_id: if wrong_key {
                    "worker".into()
                } else {
                    "other-worker".into()
                },
                encrypted_response: Some(encrypted),
                error: None,
            },
        )
        .await;
        assert!(matches!(
            waiter.wait().await.unwrap_err(),
            DaemonError::LocalTransport {
                operation: "authenticate relay peer response",
                ..
            }
        ));
    }
}

#[test]
fn verified_cleanup_absence_keeps_relay_business_code() {
    let error = DaemonError::RelayPeerCleanupAbsent {
        worker_kernel_id: "worker".into(),
        resource_id: "agent".into(),
        code: "leased_agent_not_found",
    };
    let mapped = crate::transport::relay_client::request_errors::map_relay_error(&error);
    assert_eq!(mapped.code, "leased_agent_not_found");
    assert!(!mapped.retryable);
}

#[tokio::test]
async fn mp11_persistent_peer_response_rejects_another_key() {
    let home = crate::config::DaemonConfig::for_tests();
    let worker = crate::config::DaemonConfig::for_tests();
    let attacker = crate::config::DaemonConfig::for_tests();
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (sender, mut priority_rx, _event_rx) = RelayOutgoingSender::channel(4);
    state
        .write()
        .await
        .test_set_connected_sender(sender, "ws://fixture");
    let waiter = peer_client::enqueue_peer_request_to_known_kernel_via_relay_with_timeout(
        &home,
        &state,
        ClientTarget {
            daemon_id: Some("worker".into()),
            daemon_alias: None,
        },
        &worker.relay_public_key,
        RelayPeerRequest::Ping {
            value: "fixture".into(),
        },
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    let RelayEnvelope::DaemonPeerRequest { request_id, .. } = priority_rx.recv().await.unwrap()
    else {
        panic!("expected request");
    };
    let response = RelayPeerResponse::Pong {
        value: "forged".into(),
        daemon_id: "worker".into(),
        relay_peer_protocol_version: Some(
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
        ),
    };
    let encrypted = relay_crypto::encrypt_payload_for_peer(
        &attacker.relay_private_key,
        &home.relay_public_key,
        &serde_json::to_vec(&response).unwrap(),
    )
    .unwrap();
    resolve_pending_peer_response_for_test(&state, request_id, "worker".into(), encrypted).await;
    assert!(
        waiter.wait().await.is_err(),
        "persistent response accepted the wrong encrypted sender key"
    );
}
