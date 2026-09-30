use super::*;

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

async fn send(socket: &mut Socket, envelope: RelayEnvelope) {
    socket
        .send(Message::Text(
            serde_json::to_string(&envelope).unwrap().into(),
        ))
        .await
        .unwrap();
}

async fn receive(socket: &mut Socket) -> RelayEnvelope {
    let message = timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("relay response deadline")
        .expect("relay stays connected")
        .expect("valid WebSocket frame");
    let Message::Text(text) = message else {
        panic!("expected relay envelope")
    };
    serde_json::from_str(&text).expect("valid relay envelope")
}

#[tokio::test(flavor = "multi_thread")]
async fn same_named_slices_on_two_machines_keep_distinct_signed_routes() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/slice-worker-identity.json"
    )))
    .unwrap();
    let cases = &fixture["cases"].as_array().unwrap()[..2];
    assert_eq!(cases[0]["localName"], cases[1]["localName"]);
    assert_ne!(cases[0]["workerKernelRef"], cases[1]["workerKernelRef"]);

    let secret = "synthetic-slice-fixture-issuer";
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".to_owned(),
            port: 0,
            shared_token: None,
        },
        RelayAuthVerifier::scoped_hmac(
            BTreeMap::from([("test-issuer".to_owned(), secret.to_owned())]),
            Some(10_000),
        ),
    );
    let registry = server.registry();
    let listener = server.bind_listener().await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });
    let mut parents = Vec::new();
    let mut workers = Vec::new();
    for case in cases {
        let machine = case["machineId"].as_str().unwrap();
        let parent = case["ownerKernelId"].as_str().unwrap();
        let worker = case["workerKernelRef"].as_str().unwrap();
        for (id, is_worker) in [(parent, false), (worker, true)] {
            let (mut socket, _) = connect_async_with_retry(&url).await.unwrap();
            let mut registration = test_registration(id, machine, "Linux", 1);
            if is_worker {
                registration.daemon_alias =
                    Some(format!("slice:{}", case["localName"].as_str().unwrap()));
            }
            let mut claims = scoped_claim(
                &format!("token-{id}"),
                id,
                RelaySubjectKind::Kernel,
                "shared-realm",
                vec![RelayAction::DaemonRegister, RelayAction::PeerRequest],
                is_worker.then_some(vec![parent]),
            );
            claims.issued_at_ms = 1_000;
            claims.expires_at_ms = 20_000;
            claims.account_id = Some("account-owner".to_owned());
            claims.user_id = Some("user-owner".to_owned());
            claims.machine_id = Some(machine.to_owned());
            {
                use sha2::Digest;
                claims.public_key_thumbprint = Some(format!(
                    "{:x}",
                    Sha256::digest(registration.public_key.as_bytes())
                ));
            }
            registration.auth_token = hosted_kernel_jwt(&claims, secret);
            send(&mut socket, RelayEnvelope::DaemonRegister { registration }).await;
            if is_worker {
                workers.push(socket);
            } else {
                parents.push(socket);
            }
        }
    }
    timeout(Duration::from_secs(2), async {
        while registry.read().await.daemons.len() != 4 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("both canonical workers and both parents register without replacement");
    {
        let guard = registry.read().await;
        for case in cases {
            let key = DaemonKey::new(
                "shared-realm".to_owned(),
                case["workerKernelRef"].as_str().unwrap().to_owned(),
            );
            assert_eq!(
                guard.daemons.get(&key).unwrap().machine_id,
                case["machineId"].as_str().unwrap()
            );
        }
    }
    for (index, case) in cases.iter().enumerate() {
        let worker = case["workerKernelRef"].as_str().unwrap();
        send(
            &mut workers[index],
            request(
                &format!("request-{index}"),
                case["ownerKernelId"].as_str().unwrap(),
                worker,
            ),
        )
        .await;
        let RelayEnvelope::DaemonIncomingPeerRequest {
            caller_identity: Some(identity),
            encrypted_request,
            ..
        } = receive(&mut parents[index]).await
        else {
            panic!("the corresponding parent must receive its slice request")
        };
        assert_eq!(identity.subject_kind, RelaySubjectKind::Machine);
        assert_eq!(identity.subject, case["machineId"].as_str().unwrap());
        assert_eq!(
            encrypted_request.sender_public_key,
            format!("public-key-{worker}")
        );
    }
    send(
        &mut workers[0],
        request(
            "foreign-parent",
            cases[1]["ownerKernelId"].as_str().unwrap(),
            cases[0]["workerKernelRef"].as_str().unwrap(),
        ),
    )
    .await;
    let RelayEnvelope::DaemonPeerResponse {
        request_id,
        error: Some(error),
        ..
    } = receive(&mut workers[0]).await
    else {
        panic!("a scoped slice must receive a foreign-parent rejection")
    };
    assert_eq!(request_id, "foreign-parent");
    assert_eq!(error.code, "target_not_allowed");
    for socket in workers.iter_mut().chain(parents.iter_mut()) {
        let _ = socket.close(None).await;
    }
    let _ = shutdown_tx.send(());
    task.await.unwrap();
}

fn request(request_id: &str, parent: &str, worker: &str) -> RelayEnvelope {
    RelayEnvelope::DaemonPeerRequest {
        request_id: request_id.to_owned(),
        target: ClientTarget {
            daemon_id: Some(parent.to_owned()),
            daemon_alias: None,
        },
        encrypted_request: EncryptedRelayPayload {
            sender_public_key: format!("public-key-{worker}"),
            nonce: "synthetic-nonce".to_owned(),
            ciphertext: "synthetic-payload".to_owned(),
        },
    }
}
