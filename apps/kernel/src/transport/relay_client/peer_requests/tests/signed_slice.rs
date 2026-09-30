//! Exercise the signed relay boundary and the owner handler together. Directly
//! injecting a Kernel identity misses the relay's Machine identity projection.
use super::*;
use chariox_relay::protocol::{ClientTarget, DaemonRegistration, RelayEnvelope};
use chariox_relay::{RelayAuthVerifier, RelayConfig, RelayServer};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use std::collections::BTreeMap;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
const ISSUER_SECRET: &str = "synthetic-signed-slice-test-issuer";

async fn send(socket: &mut Socket, envelope: RelayEnvelope) {
    socket
        .send(Message::Text(
            serde_json::to_string(&envelope).unwrap().into(),
        ))
        .await
        .unwrap();
}

async fn receive(socket: &mut Socket) -> RelayEnvelope {
    let frame = timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("relay response deadline")
        .expect("relay connection")
        .expect("relay frame");
    let Message::Text(text) = frame else {
        panic!("expected relay envelope")
    };
    serde_json::from_str(&text).unwrap()
}

fn registration(
    id: &str,
    machine: &str,
    public_key: &str,
    parent: Option<&str>,
) -> DaemonRegistration {
    let now = crate::session::unix_epoch_ms() / 1000;
    let header =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let claims = serde_json::json!({
        "iss": "issuer-1", "sub": id, "subject_kind": "kernel",
        "realm_id": "realm-1", "allowed_actions": ["daemon.register", "daemon.heartbeat", "packet.route", "peer.request", "peer.event"],
        "allowed_targets": parent.map(|id| vec![id]), "iat": now, "exp": now + 3600,
        "jti": format!("synthetic-{id}"), "account_id": "account-1", "user_id": "user-1",
        "machine_id": machine, "public_key_thumbprint": public_key_thumbprint(public_key)
    });
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims).unwrap());
    let signing_input = format!("{header}.{payload}");
    let mut mac = Hmac::<Sha256>::new_from_slice(ISSUER_SECRET.as_bytes()).unwrap();
    mac.update(signing_input.as_bytes());
    let signature =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    DaemonRegistration {
        auth_token: format!("{signing_input}.{signature}"),
        daemon_id: id.to_owned(),
        machine_id: machine.to_owned(),
        machine_alias: None,
        os_name: Some("Linux".to_owned()),
        kernel_started_at_ms: now * 1000,
        daemon_alias: parent.map(|_| "slice:drill".to_owned()),
        kernel_alias: None,
        public_key: public_key.to_owned(),
        capabilities: vec!["kernel_ws".to_owned()],
        available_providers: Vec::new(),
        provider_accounts: Vec::new(),
        accepting_remote_leases: false,
        leased_agent_count: 0,
        local_session_count: 0,
    }
}

struct SignedSliceHarness {
    router: Arc<CommandRouter>,
    state: Arc<RwLock<RelayClientState>>,
    outgoing: RelayOutgoingSender,
    owner: Socket,
    worker: Socket,
    owner_id: String,
    owner_public_key: String,
    worker_private_key: String,
}

impl SignedSliceHarness {
    async fn request(&mut self, request: RelayPeerRequest) -> RelayPeerResponse {
        let encrypted_request = relay_crypto::encrypt_payload_for_peer(
            &self.worker_private_key,
            &self.owner_public_key,
            &serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
        send(
            &mut self.worker,
            RelayEnvelope::DaemonPeerRequest {
                request_id: "signed-slice-request".to_owned(),
                target: ClientTarget {
                    daemon_id: Some(self.owner_id.clone()),
                    daemon_alias: None,
                },
                encrypted_request,
            },
        )
        .await;
        let RelayEnvelope::DaemonIncomingPeerRequest {
            relay_request_id,
            from_daemon_id,
            caller_identity,
            encrypted_request,
        } = receive(&mut self.owner).await
        else {
            panic!("owner receives routed request")
        };
        // Pass exactly the identity emitted by the relay to the production handler.
        let outcome = handle_daemon_peer_request(
            &self.router,
            &self.state,
            &self.outgoing,
            &from_daemon_id,
            caller_identity,
            encrypted_request,
        )
        .await;
        send(
            &mut self.owner,
            RelayEnvelope::DaemonIncomingPeerResponse {
                relay_request_id,
                encrypted_response: outcome.encrypted_response,
                error: outcome.error,
            },
        )
        .await;
        let RelayEnvelope::DaemonPeerResponse {
            request_id,
            encrypted_response,
            error,
            ..
        } = receive(&mut self.worker).await
        else {
            panic!("worker receives routed response")
        };
        assert_eq!(request_id, "signed-slice-request");
        assert!(error.is_none(), "unexpected relay error: {error:?}");
        let decrypted = relay_crypto::decrypt_payload_for_private_key(
            &self.worker_private_key,
            &encrypted_response.expect("encrypted owner response"),
        )
        .unwrap();
        serde_json::from_slice(&decrypted.plaintext).unwrap()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn signed_slice_relay_preserves_kernel_authority_for_owner_confirmation_and_refresh() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/slice-worker-identity.json"
    )))
    .unwrap();
    let vector = &fixture["cases"][0];
    let worker_id = vector["workerKernelRef"].as_str().unwrap().to_owned();
    let machine_id = vector["machineId"].as_str().unwrap().to_owned();
    let owner_id = vector["ownerKernelId"].as_str().unwrap().to_owned();
    let (cloud_url, cloud_fixture) = cloud_token_fixture().await;
    let root = std::env::temp_dir().join(format!(
        "chariox-signed-slice-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("sessions"));
    config.daemon_id = owner_id.clone();
    config.host_machine_id = machine_id.clone();
    config.user_config.history.operational.path =
        Some(root.join("operational.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    config.user_config.state.path = Some(root.join("kernel/state.db").display().to_string());
    config.cloud_relay = Some(test_cloud_profile(cloud_url, machine_id.clone()));
    let owner_public_key = config.relay_public_key.clone();
    let app = DaemonApp::bootstrap(config).unwrap();
    let state = app.relay_client_state();
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        1,
    ));
    let slice = router
        .runtime_state()
        .create_slice(crate::local::CreateSliceRequest {
            name: "drill".to_owned(),
            backend: crate::slice::SliceBackendKind::LocalDocker,
            os: "linux".to_owned(),
            display_mode: crate::slice::SliceDisplayMode::Headless,
            display_backend: crate::slice::SliceDisplayBackend::default(),
            workspace_id: None,
            worktree_id: None,
            workspace_mount: None,
            development: None,
            worker_kernel_ref: Some(worker_id.clone()),
            display_url: None,
            provider_auth: Vec::new(),
            from_saved_state: None,
            base: Some(crate::local::SliceCreateBase::Clean),
        })
        .await
        .unwrap();
    router
        .runtime_state()
        .mark_slice_starting(
            &slice.id,
            crate::slice::SliceRelayEndpoint {
                url: "wss://relay.example.test".to_owned(),
                private: false,
            },
        )
        .unwrap();
    let worker_private_key = relay_crypto::generate_private_key_base64();
    let worker_public_key =
        relay_crypto::public_key_from_private_key_base64(&worker_private_key).unwrap();
    state.write().await.begin_managed_slice_relay_activation(
        slice.id.clone(),
        worker_id.clone(),
        worker_id.clone(),
        worker_public_key.clone(),
        "activation-1".to_owned(),
    );
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".to_owned(),
            port: 0,
            shared_token: None,
        },
        RelayAuthVerifier::scoped_hmac(
            BTreeMap::from([("issuer-1".to_owned(), ISSUER_SECRET.to_owned())]),
            None,
        ),
    );
    let registry = server.registry();
    let listener = server.bind_listener().await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let relay_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });
    let (mut owner, _) = connect_async(&url).await.unwrap();
    let (mut worker, _) = connect_async(&url).await.unwrap();
    send(
        &mut owner,
        RelayEnvelope::DaemonRegister {
            registration: registration(&owner_id, &machine_id, &owner_public_key, None),
        },
    )
    .await;
    send(
        &mut worker,
        RelayEnvelope::DaemonRegister {
            registration: registration(
                &worker_id,
                &machine_id,
                &worker_public_key,
                Some(&owner_id),
            ),
        },
    )
    .await;
    timeout(Duration::from_secs(5), async {
        while registry.read().await.daemon_count() != 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("signed owner and worker register");
    let (outgoing, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
    let mut harness = SignedSliceHarness {
        router,
        state,
        outgoing,
        owner,
        worker,
        owner_id: owner_id.clone(),
        owner_public_key,
        worker_private_key,
    };
    let confirm = |nonce: &str| RelayPeerRequest::ConfirmManagedSliceRelayToken {
        slice_id: slice.id.clone(),
        owner_kernel_id: owner_id.clone(),
        worker_kernel_id: worker_id.clone(),
        activation_nonce: nonce.to_owned(),
    };
    let wrong_nonce = harness.request(confirm("wrong-nonce")).await;
    assert!(
        matches!(wrong_nonce, RelayPeerResponse::ManagedSliceRelayTokenFailed { ref code, retryable: false } if code == "unauthorized")
    );
    let confirmed = harness.request(confirm("activation-1")).await;
    assert!(
        matches!(&confirmed, RelayPeerResponse::ManagedSliceRelayTokenActivated { slice_id, activation_nonce, .. } if slice_id == &slice.id && activation_nonce == "activation-1"),
        "real signed relay confirmation failed: {confirmed:?}"
    );
    let refreshed = harness
        .request(RelayPeerRequest::RefreshManagedSliceRelayToken {
            slice_id: slice.id.clone(),
            owner_kernel_id: owner_id,
            worker_kernel_id: worker_id.clone(),
        })
        .await;
    assert!(
        matches!(&refreshed, RelayPeerResponse::ManagedSliceRelayTokenRefreshed { slice_id, .. } if slice_id == &slice.id),
        "real signed relay refresh failed: {refreshed:?}"
    );
    assert_eq!(
        harness
            .router
            .runtime_state()
            .resolve_slice(&slice.id)
            .unwrap()
            .worker_kernel_id
            .as_deref(),
        Some(worker_id.as_str())
    );
    timeout(Duration::from_secs(5), cloud_fixture)
        .await
        .expect("two scoped Cloud requests")
        .unwrap();
    let _ = harness.worker.close(None).await;
    let _ = harness.owner.close(None).await;
    let _ = shutdown_tx.send(());
    relay_task.await.unwrap();
    // All state is confined to the disposable test container's unique temp root.
}

async fn cloud_token_fixture() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Cloud token fixture");
    let address = listener.local_addr().expect("Cloud token fixture address");
    let fixture = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept token request");
            let mut request = Vec::new();
            loop {
                let mut chunk = [0_u8; 4096];
                let read = tokio::io::AsyncReadExt::read(&mut stream, &mut chunk)
                    .await
                    .expect("read token request");
                assert!(read > 0, "token request ended before its body");
                request.extend_from_slice(&chunk[..read]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .expect("token request content length");
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&serde_json::json!({
                    "exp": crate::session::unix_epoch_ms() / 1_000 + 3_600,
                }))
                .expect("encode fixture token claims"),
            );
            let body = format!(
                r#"{{"token":"header.{claims}.signature","expiresAt":"2099-01-01T00:00:00Z"}}"#
            );
            let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
            tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes())
                .await
                .expect("write token response");
        }
    });
    (format!("http://{address}"), fixture)
}
