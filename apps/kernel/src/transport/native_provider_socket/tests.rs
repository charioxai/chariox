//! MP-08 / MP-10 / MP-11: scoped native socket regression checks.
use super::*;
#[test]
fn mp08_native_transport_rejects_prompt_approval_and_arbitrary_endpoint() {
    for method in [
        "turn/start",
        "turn/steer",
        "turn/interrupt",
        "command/exec",
        "account/login/start",
        "config/value/write",
    ] {
        assert!(validate_frame(&serde_json::json!({"id":1,"method":method}).to_string()).is_err());
    }
    assert!(validate_frame(r#"{"id":1,"result":{"decision":"accept"}}"#).is_err());
    assert!(validate_frame(r#"{"id":1,"method":"initialize"}"#).is_ok());
    for endpoint in [
        "ws://example.com:1234",
        "ws://127.0.0.1",
        "wss://localhost:1234",
        "ws://user@localhost:1234",
    ] {
        assert!(validate_endpoint(endpoint).is_err());
    }
    assert!(validate_endpoint("ws://127.0.0.1:1234").is_ok());
}

#[test]
fn mp08_native_folder_trust_is_only_for_the_owning_workspace() {
    let scope = SocketScope {
        session_id: "room".into(),
        agent_id: "agent".into(),
        attachment_id: "attachment".into(),
        provider_run_id: "run".into(),
        leased_agent_id: None,
        endpoint: "ws://127.0.0.1:1234".into(),
        working_directory: Some("/workspace/repo".into()),
    };
    let frame = serde_json::json!({"id":1,"method":"config/batchWrite","params":{
        "edits":[{"keyPath":"projects.\"/workspace/repo\".trust_level","value":"trusted","mergeStrategy":"replace"}],
        "filePath":null,"expectedVersion":null,"reloadUserConfig":true
    }});
    assert!(validate_scoped_frame(&frame.to_string(), &scope).is_ok());
    for (field, value) in [
        (
            "keyPath",
            serde_json::json!("projects.\"/workspace/other\".trust_level"),
        ),
        ("keyPath", serde_json::json!("sandbox_mode")),
        ("value", serde_json::json!("danger-full-access")),
        ("mergeStrategy", serde_json::json!("upsert")),
    ] {
        let mut bad = frame.clone();
        bad["params"]["edits"][0][field] = value;
        assert!(validate_scoped_frame(&bad.to_string(), &scope).is_err());
    }
    let mut bad = frame.clone();
    bad["params"]["filePath"] = serde_json::json!("/workspace/config.toml");
    assert!(validate_scoped_frame(&bad.to_string(), &scope).is_err());
    let mut bad = frame.clone();
    bad["params"]["edits"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"keyPath":"model","value":"other","mergeStrategy":"replace"}));
    assert!(validate_scoped_frame(&bad.to_string(), &scope).is_err());
}

#[tokio::test]
async fn mp08_native_large_official_catalog_uses_shared_relay_sized_frames() {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let catalog =
        serde_json::json!({"id":1,"result":{"catalog":"x".repeat(14 * 1024 * 1024)}}).to_string();
    let expected = catalog.len();
    let expected_hash = Sha256::digest(catalog.as_bytes());
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        socket.send(Message::Text(catalog.into())).await.unwrap();
        while let Some(Ok(message)) = socket.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
    });
    let scope = SocketScope {
        session_id: "room".into(),
        agent_id: "agent".into(),
        attachment_id: "attachment".into(),
        provider_run_id: "run".into(),
        leased_agent_id: None,
        endpoint: format!("ws://{address}"),
        working_directory: Some("/workspace".into()),
    };
    let sockets = NativeProviderSockets::default();
    let id = sockets
        .execute(
            scope.clone(),
            NativeProviderTransportOperation::Open {
                request_id: "open-test".into(),
            },
            || async { Ok(()) },
        )
        .await
        .unwrap()
        .connection_id;
    let mut received = false;
    let mut compressed = Vec::new();
    for index in 0..500 {
        let result = sockets
            .execute(
                scope.clone(),
                NativeProviderTransportOperation::Exchange {
                    connection_id: id.clone(),
                    frames: vec![],
                    sequence: index + 1,
                },
                || async { Ok(()) },
            )
            .await
            .unwrap();
        assert!(
            result.frames.is_empty(),
            "a large reply must use bounded pages"
        );
        assert!(serde_json::to_vec(&result).unwrap().len() < 128 * 1024);
        if let Some(page) = result.gzip_fragment {
            assert_eq!(page.offset as usize, compressed.len());
            compressed.extend(
                base64::engine::general_purpose::STANDARD
                    .decode(page.data)
                    .unwrap(),
            );
            if compressed.len() == page.total_bytes as usize {
                let mut decoded = Vec::new();
                flate2::read::GzDecoder::new(compressed.as_slice())
                    .read_to_end(&mut decoded)
                    .unwrap();
                assert_eq!(decoded.len(), expected);
                assert_eq!(Sha256::digest(&decoded), expected_hash);
                assert!(!result.closed);
                received = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        received,
        "native official 14 MiB catalog must arrive intact"
    );
    let closed = sockets
        .execute(
            scope,
            NativeProviderTransportOperation::Close { connection_id: id },
            || async { Ok(()) },
        )
        .await
        .unwrap();
    assert!(closed.closed);
    tokio::time::timeout(Duration::from_secs(3), upstream)
        .await
        .unwrap()
        .unwrap();
}

// MP-08 / MP-10 / MP-11: a final reply must survive upstream closure and reply loss.
#[tokio::test]
async fn mp08_native_closed_upstream_retains_final_receipt() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        socket
            .send(Message::Text(r#"{"id":7,"result":{}}"#.into()))
            .await
            .unwrap();
        socket.close(None).await.unwrap();
    });
    let sockets = NativeProviderSockets::default();
    let scope = SocketScope {
        session_id: "room".into(),
        agent_id: "agent".into(),
        attachment_id: "view".into(),
        provider_run_id: "run".into(),
        leased_agent_id: None,
        endpoint: format!("ws://{address}"),
        working_directory: Some("/workspace".into()),
    };
    let opened = sockets
        .execute(
            scope.clone(),
            NativeProviderTransportOperation::Open {
                request_id: "open-final".into(),
            },
            || async { Ok(()) },
        )
        .await
        .unwrap();
    upstream.await.unwrap();
    // Wait until the owner observes the upstream close rather than racing its final frame.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let operation = NativeProviderTransportOperation::Exchange {
        connection_id: opened.connection_id.clone(),
        sequence: 1,
        frames: vec![],
    };
    let first = sockets
        .execute(scope.clone(), operation.clone(), || async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(first.frames, [r#"{"id":7,"result":{}}"#]);
    assert!(first.closed);
    let repeated = sockets
        .execute(scope.clone(), operation, || async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(repeated, first);
    let repeated_open = sockets
        .execute(
            scope.clone(),
            NativeProviderTransportOperation::Open {
                request_id: "open-final".into(),
            },
            || async { Ok(()) },
        )
        .await
        .unwrap();
    assert_eq!(repeated_open.connection_id, opened.connection_id);
    sockets
        .execute(
            scope,
            NativeProviderTransportOperation::Close {
                connection_id: opened.connection_id,
            },
            || async { Ok(()) },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn mp08_native_socket_fences_attachment_and_passive_run_revocation() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (ended_tx, ended_rx) = tokio::sync::oneshot::channel();
    let writes = Arc::new(AtomicUsize::new(0));
    let writes_observed = writes.clone();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        while let Some(Ok(message)) = socket.next().await {
            match message {
                Message::Text(_) => {
                    writes_observed.fetch_add(1, Ordering::SeqCst);
                    socket
                        .send(Message::Text(
                            r#"{"id":1,"result":{"userAgent":"official-test-seam"}}"#.into(),
                        ))
                        .await
                        .unwrap();
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = ended_tx.send(());
    });
    let scope = SocketScope {
        session_id: "room".into(),
        agent_id: "agent".into(),
        attachment_id: "attachment".into(),
        provider_run_id: "run".into(),
        leased_agent_id: Some("lease-agent".into()),
        endpoint: format!("ws://{address}"),
        working_directory: Some("/workspace/repo".into()),
    };
    let allowed = Arc::new(AtomicBool::new(true));
    let guard = {
        let allowed = allowed.clone();
        move || {
            let allowed = allowed.clone();
            async move {
                if allowed.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err(error("run replaced"))
                }
            }
        }
    };
    let sockets = NativeProviderSockets::default();
    let id = sockets
        .execute(
            scope.clone(),
            NativeProviderTransportOperation::Open {
                request_id: "open-test".into(),
            },
            guard.clone(),
        )
        .await
        .unwrap()
        .connection_id;
    let replayed_open = sockets
        .execute(
            scope.clone(),
            NativeProviderTransportOperation::Open {
                request_id: "open-test".into(),
            },
            guard.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        replayed_open.connection_id, id,
        "lost Open acknowledgement cannot create another upstream"
    );
    let mut foreign = scope.clone();
    foreign.attachment_id = "foreign".into();
    assert!(sockets
        .execute(
            foreign,
            NativeProviderTransportOperation::Close {
                connection_id: id.clone()
            },
            guard.clone()
        )
        .await
        .is_err());
    sockets
        .execute(
            scope.clone(),
            NativeProviderTransportOperation::Exchange {
                connection_id: id.clone(),
                frames: vec![r#"{"id":1,"method":"initialize"}"#.into()],
                sequence: 1,
            },
            guard.clone(),
        )
        .await
        .unwrap();
    let replayed = sockets
        .execute(
            scope.clone(),
            NativeProviderTransportOperation::Exchange {
                connection_id: id.clone(),
                frames: vec![r#"{"id":1,"method":"initialize"}"#.into()],
                sequence: 1,
            },
            guard.clone(),
        )
        .await
        .unwrap();
    assert_eq!(replayed.sequence, 1);
    let mut observed = false;
    for index in 0..100 {
        let result = sockets
            .execute(
                scope.clone(),
                NativeProviderTransportOperation::Exchange {
                    connection_id: id.clone(),
                    frames: vec![],
                    sequence: index + 2,
                },
                guard.clone(),
            )
            .await
            .unwrap();
        if !result.frames.is_empty() {
            observed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(observed);
    assert_eq!(
        writes.load(Ordering::SeqCst),
        1,
        "duplicate exchange cannot resend official frames"
    );
    allowed.store(false, Ordering::SeqCst);
    assert!(
        sockets
            .execute(
                scope.clone(),
                NativeProviderTransportOperation::Open {
                    request_id: "open-test".into()
                },
                guard.clone()
            )
            .await
            .is_err(),
        "replay cannot bypass revoked run authority"
    );
    tokio::time::timeout(Duration::from_secs(3), ended_rx)
        .await
        .unwrap()
        .unwrap();
    upstream.await.unwrap();
    // The socket stops even when a revoked attachment never polls again.
    for _ in 0..100 {
        if sockets.0.lock().await.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("revoked native connection was retained");
}

// Full byte and frame limits exercise each capacity await independently.
#[tokio::test]
async fn mp08_native_capacity_close_releases_full_byte_budget() {
    assert_capacity_shutdown(true, false).await;
}
#[tokio::test]
async fn mp08_native_capacity_revocation_releases_full_byte_budget() {
    assert_capacity_shutdown(true, true).await;
}
#[tokio::test]
async fn mp08_native_capacity_close_releases_full_inbound_queue() {
    assert_capacity_shutdown(false, false).await;
}
#[tokio::test]
async fn mp08_native_capacity_revocation_releases_full_inbound_queue() {
    assert_capacity_shutdown(false, true).await;
}

async fn assert_capacity_shutdown(byte_limit: bool, revoke: bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (sent, sent_rx) = tokio::sync::oneshot::channel();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let count = if byte_limit { 3 } else { QUEUE_FRAMES + 2 };
        let size = if byte_limit { OUTPUT_FRAME_BYTES } else { 8 };
        for _ in 0..count {
            socket
                .send(Message::Text("x".repeat(size).into()))
                .await
                .unwrap();
        }
        let _ = sent.send(());
        while let Some(Ok(message)) = socket.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
    });
    let scope = SocketScope {
        session_id: "capacity-room".into(),
        agent_id: "agent".into(),
        attachment_id: "attachment".into(),
        provider_run_id: "run".into(),
        leased_agent_id: Some("leased".into()),
        endpoint: format!("ws://{address}"),
        working_directory: Some("/workspace/repo".into()),
    };
    let allowed = Arc::new(AtomicBool::new(true));
    let guard = {
        let allowed = allowed.clone();
        move || {
            let allowed = allowed.clone();
            async move {
                if allowed.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err(error("attachment revoked"))
                }
            }
        }
    };
    let sockets = NativeProviderSockets::default();
    let id = sockets
        .execute(
            scope.clone(),
            NativeProviderTransportOperation::Open {
                request_id: "capacity".into(),
            },
            guard.clone(),
        )
        .await
        .unwrap()
        .connection_id;
    let entry = Arc::downgrade(sockets.0.lock().await.get(&id).unwrap());
    tokio::time::timeout(Duration::from_secs(5), sent_rx)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = entry.upgrade().unwrap();
            let queued = current.exchange.lock().await.buffered_frames();
            if byte_limit {
                if queued == 2 && current.output_budget.reserve("probe".into()).is_err() {
                    break;
                }
            } else if queued == QUEUE_FRAMES {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("MP-08 test must actually fill the capacity limit");
    tokio::time::sleep(Duration::from_millis(100)).await;
    if revoke {
        allowed.store(false, Ordering::SeqCst);
    } else {
        let closed = sockets
            .execute(
                scope,
                NativeProviderTransportOperation::Close {
                    connection_id: id.clone(),
                },
                guard,
            )
            .await
            .unwrap();
        assert!(closed.closed);
    }
    let shutdown = tokio::time::timeout(Duration::from_secs(3), async {
        upstream.await.unwrap();
        while entry.upgrade().is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        shutdown.is_ok(),
        "MP-08 Close/revocation must close upstream and release queued permits under backpressure"
    );
    assert!(!sockets.0.lock().await.contains_key(&id));
}
