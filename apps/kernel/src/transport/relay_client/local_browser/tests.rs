//! MP-08/MP-11: direct loopback admission over real sockets and real dispatch.

use super::*;
use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
use crate::local::{ListSessionsRequest, LocalDaemonRequest};
use crate::runtime_transport::command_cache::CommandResultCache;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

const ORIGIN: &str = "https://cloud.example.test";

/// Debug builds of the shared dispatch path need the kernel's large stacks.
fn large_stack(test: impl std::future::Future<Output = ()> + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(32 * 1024 * 1024)
                .build()
                .unwrap()
                .block_on(test)
        })
        .unwrap()
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Browser {
    private_key: String,
    public_key: String,
}

impl Browser {
    fn new() -> Self {
        let private_key = relay_crypto::generate_private_key_base64();
        let public_key = relay_crypto::public_key_from_private_key_base64(&private_key).unwrap();
        Self {
            private_key,
            public_key,
        }
    }

    fn identity(&self) -> RelayCallerIdentity {
        RelayCallerIdentity {
            realm_id: "realm-1".into(),
            subject: "browser:user-1:tab".into(),
            subject_kind: RelaySubjectKind::Client,
            expires_at_ms: crate::session::unix_epoch_ms() + 60_000,
            token_id: Some("token-1".into()),
            user_id: Some("user-1".into()),
            public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
                &self.public_key,
            )),
        }
    }
}

struct Kernel {
    direct: Arc<LocalBrowserDirect>,
    projection: crate::runtime::projection::DaemonConfigProjectionStore,
    daemon_id: String,
    daemon_public_key: String,
    _shutdown: watch::Sender<bool>,
    _scratch: Scratch,
}

impl Kernel {
    fn new() -> Self {
        Self::with_pairing(true)
    }

    /// `paired_at_start: false` pairs only after the carrier exists, as when a
    /// running kernel completes `/cloud link`.
    fn with_pairing(paired_at_start: bool) -> Self {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            api_url: ORIGIN.into(),
            email: "user@example.test".into(),
            account_id: "account-1".into(),
            user_id: "user-1".into(),
            account_slug: "account".into(),
            realm_id: "realm-1".into(),
            relay_url: "wss://relay.example.test".into(),
            issuer_id: "issuer-1".into(),
            client_id: None,
            client_alias: None,
            machine_id: None,
            machine_alias: None,
            machine_credential: None,
            cloud_session_token: None,
            cloud_session_expires_at_ms: None,
            token_expires_at_ms: None,
        });
        let scratch = Scratch(
            std::path::PathBuf::from(config.user_config.state.path.as_ref().unwrap())
                .parent()
                .unwrap()
                .to_path_buf(),
        );
        let profile = config.cloud_relay.clone();
        if !paired_at_start {
            config.cloud_relay = None;
        }
        let daemon_id = config.daemon_id.clone();
        let daemon_public_key = config.relay_public_key.clone();
        let app = DaemonApp::bootstrap(config).unwrap();
        let projection = app.config_projection_store();
        let router = Arc::new(CommandRouter::with_interactive_capacity(
            Arc::new(Mutex::new(app)),
            4,
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let direct = LocalBrowserDirect::with_port(
            router,
            Arc::new(AtomicU64::new(1)),
            Arc::new(RelayEventRuntime::for_tests(64)),
            Arc::new(CommandResultCache::default()),
            shutdown_rx,
            Some(0),
        );
        if !paired_at_start {
            let mut paired = projection.snapshot();
            paired.cloud_relay = profile;
            projection.update(paired);
        }
        Self {
            direct,
            projection,
            daemon_id,
            daemon_public_key,
            _shutdown: shutdown_tx,
            _scratch: scratch,
        }
    }

    /// Mints a grant exactly as the relay carrier does.
    async fn mint(
        &self,
        browser: &Browser,
        identity: RelayCallerIdentity,
    ) -> Result<Grant, RelayError> {
        let request = relay_crypto::encrypt_payload_for_peer(
            &browser.private_key,
            &self.daemon_public_key,
            br#"{"local_browser_connect":{}}"#,
        )
        .unwrap();
        let outcome = handle_daemon_request(
            &self.direct.router,
            &self.direct.command_sequence,
            Some(identity),
            request,
            &self.direct.command_result_cache,
            Some(&self.direct),
        )
        .await;
        if let Some(error) = outcome.error {
            return Err(error);
        }
        let plaintext = relay_crypto::decrypt_payload_for_private_key(
            &browser.private_key,
            &outcome.encrypted_response.unwrap(),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&plaintext.plaintext).unwrap();
        Ok(serde_json::from_value(value["LocalBrowserConnectIssued"].clone()).unwrap())
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct Grant {
    endpoint: String,
    grant: String,
    kernel_id: String,
    endpoint_epoch: String,
    expires_at_ms: u64,
}

type ClientSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn upgrade(endpoint: &str, origins: &[&str]) -> Result<ClientSocket, u16> {
    let mut request = endpoint.into_client_request().unwrap();
    for origin in origins {
        request
            .headers_mut()
            .append("origin", HeaderValue::from_str(origin).unwrap());
    }
    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            Err(response.status().as_u16())
        }
        Err(error) => panic!("unexpected upgrade failure: {error}"),
    }
}

async fn next_json(socket: &mut ClientSocket) -> serde_json::Value {
    loop {
        match timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
        {
            Some(Ok(Message::Text(text))) => return serde_json::from_str(&text).unwrap(),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

async fn send_json(socket: &mut ClientSocket, value: serde_json::Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

fn proof(browser: &Browser, kernel: &Kernel, plaintext: serde_json::Value) -> serde_json::Value {
    serde_json::to_value(
        relay_crypto::encrypt_payload_for_peer(
            &browser.private_key,
            &kernel.daemon_public_key,
            plaintext.to_string().as_bytes(),
        )
        .unwrap(),
    )
    .unwrap()
}

/// Runs the browser half of the handshake; returns the server's verdict frame.
async fn connect(
    kernel: &Kernel,
    browser: &Browser,
    grant: &Grant,
) -> (ClientSocket, serde_json::Value) {
    let mut socket = upgrade(&grant.endpoint, &[ORIGIN])
        .await
        .expect("upgrade admitted");
    let challenge = next_json(&mut socket).await;
    assert_eq!(challenge["kind"], "local_challenge");
    assert_eq!(challenge["kernel_id"], kernel.daemon_id.as_str());
    let proof = proof(
        browser,
        kernel,
        serde_json::json!({
            "grant": grant.grant,
            "challenge": challenge["challenge"],
            "origin": ORIGIN,
            "kernel_id": grant.kernel_id,
            "endpoint_epoch": grant.endpoint_epoch,
        }),
    );
    send_json(
        &mut socket,
        serde_json::json!({"kind": "local_connect", "proof": proof}),
    )
    .await;
    let verdict = next_json(&mut socket).await;
    (socket, verdict)
}

#[test]
fn mp08_mp11_paired_browser_upgrades_and_dispatches_through_shared_relay_path() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        assert!(grant.endpoint.starts_with("ws://127.0.0.1:"));
        assert!(grant.endpoint.ends_with(LOCAL_BROWSER_PATH));
        assert!(grant.expires_at_ms <= crate::session::unix_epoch_ms() + GRANT_TTL_MS);

        let (mut socket, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(verdict["kind"], "local_connected", "{verdict}");
        assert_eq!(
            verdict["daemon_public_key"],
            kernel.daemon_public_key.as_str()
        );
        let kernel_proof: EncryptedRelayPayload =
            serde_json::from_value(verdict["proof"].clone()).unwrap();
        assert_eq!(kernel_proof.sender_public_key, kernel.daemon_public_key);
        let kernel_proof =
            relay_crypto::decrypt_payload_for_private_key(&browser.private_key, &kernel_proof)
                .unwrap();
        let kernel_proof: serde_json::Value =
            serde_json::from_slice(&kernel_proof.plaintext).unwrap();
        assert_eq!(kernel_proof["grant"], grant.grant.as_str());

        let request_key = Browser::new();
        let request = relay_crypto::encrypt_payload_for_peer(
            &request_key.private_key,
            &kernel.daemon_public_key,
            &serde_json::to_vec(&LocalDaemonRequest::ListSessions(ListSessionsRequest)).unwrap(),
        )
        .unwrap();
        send_json(
            &mut socket,
            serde_json::json!({
                "kind": "client_request",
                "request_id": "list-1",
                "target": {"daemon_id": kernel.daemon_id},
                "encrypted_request": request,
            }),
        )
        .await;
        let response = next_json(&mut socket).await;
        assert_eq!(response["kind"], "client_response", "{response}");
        assert_eq!(response["request_id"], "list-1");
        assert!(response["error"].is_null(), "{response}");
        let response: EncryptedRelayPayload =
            serde_json::from_value(response["encrypted_response"].clone()).unwrap();
        let response =
            relay_crypto::decrypt_payload_for_private_key(&request_key.private_key, &response)
                .unwrap();
        let response: serde_json::Value = serde_json::from_slice(&response.plaintext).unwrap();
        assert!(response.get("SessionsListed").is_some(), "{response}");

        send_json(
            &mut socket,
            serde_json::json!({
                "kind": "client_request",
                "request_id": "elsewhere",
                "target": {"daemon_id": "another-kernel"},
                "encrypted_request": request_key_payload(&kernel),
            }),
        )
        .await;
        let mismatch = next_json(&mut socket).await;
        assert_eq!(mismatch["error"]["code"], "target_mismatch");
    });
}

fn request_key_payload(kernel: &Kernel) -> EncryptedRelayPayload {
    relay_crypto::encrypt_payload_for_peer(
        &Browser::new().private_key,
        &kernel.daemon_public_key,
        b"{}",
    )
    .unwrap()
}

#[test]
fn mp08_kernel_paired_after_start_admits_its_first_browser() {
    large_stack(async {
        let kernel = Kernel::with_pairing(false);
        let browser = Browser::new();
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        let (_socket, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(verdict["kind"], "local_connected", "{verdict}");
    });
}

#[test]
fn mp11_upgrade_refuses_foreign_missing_null_and_duplicate_origins() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        for origins in [
            vec!["https://evil.example.test"],
            vec![],
            vec!["null"],
            vec![ORIGIN, ORIGIN],
            vec!["https://sub.cloud.example.test"],
            vec!["http://cloud.example.test"],
        ] {
            assert_eq!(
                upgrade(&grant.endpoint, &origins).await.err(),
                Some(403),
                "{origins:?}"
            );
        }
        let localhost = grant.endpoint.replace("127.0.0.1", "localhost");
        assert_eq!(upgrade(&localhost, &[ORIGIN]).await.err(), Some(403));
        let other_path = grant.endpoint.replace(LOCAL_BROWSER_PATH, "/kernel");
        assert_eq!(upgrade(&other_path, &[ORIGIN]).await.err(), Some(403));
        // Refusals did not spend the grant.
        let (_socket, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(verdict["kind"], "local_connected", "{verdict}");
    });
}

#[test]
fn mp11_grant_is_single_use_and_bound_to_key_challenge_and_expiry() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();

        // Wrong key: the proof is not from the key bound into the relay identity.
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        let (_socket, verdict) = connect(&kernel, &Browser::new(), &grant).await;
        assert_eq!(verdict["kind"], "close");
        assert_eq!(
            verdict["reason"],
            "local browser key does not match its grant"
        );

        // Replay: a redeemed grant cannot admit a second connection.
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        let (_first, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(verdict["kind"], "local_connected");
        let (_second, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(
            verdict["reason"],
            "local browser grant is unknown or already used"
        );

        // Replay of a whole proof onto a fresh connection fails its challenge.
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        let mut socket = upgrade(&grant.endpoint, &[ORIGIN]).await.unwrap();
        let _challenge = next_json(&mut socket).await;
        let stale = proof(
            &browser,
            &kernel,
            serde_json::json!({
                "grant": grant.grant, "challenge": "previous-challenge", "origin": ORIGIN,
                "kernel_id": grant.kernel_id, "endpoint_epoch": grant.endpoint_epoch,
            }),
        );
        send_json(
            &mut socket,
            serde_json::json!({"kind": "local_connect", "proof": stale}),
        )
        .await;
        assert_eq!(
            next_json(&mut socket).await["reason"],
            "local browser proof is not bound to this connection"
        );

        // Expired grant.
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        kernel
            .direct
            .grants
            .lock()
            .unwrap()
            .get_mut(&grant.grant)
            .unwrap()
            .expires_at_ms = 1;
        let (_socket, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(verdict["reason"], "local browser grant has expired");
    });
}

#[test]
fn mp11_grant_minting_requires_relay_carrier_and_bound_live_identity() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();
        let mut unbound = browser.identity();
        unbound.public_key_thumbprint = None;
        assert_eq!(
            kernel.mint(&browser, unbound).await.unwrap_err().code,
            "unauthorized"
        );
        let mut other_key = browser.identity();
        other_key.public_key_thumbprint = Some("not-this-key".into());
        assert_eq!(
            kernel.mint(&browser, other_key).await.unwrap_err().code,
            "unauthorized"
        );
        let mut expired = browser.identity();
        expired.expires_at_ms = 1;
        assert_eq!(
            kernel.mint(&browser, expired).await.unwrap_err().code,
            "unauthorized"
        );
        let mut other_realm = browser.identity();
        other_realm.realm_id = "realm-2".into();
        assert_eq!(
            kernel.mint(&browser, other_realm).await.unwrap_err().code,
            "unauthorized"
        );

        let request = relay_crypto::encrypt_payload_for_peer(
            &browser.private_key,
            &kernel.daemon_public_key,
            br#"{"local_browser_connect":{}}"#,
        )
        .unwrap();
        let direct_carrier = handle_daemon_request(
            &kernel.direct.router,
            &kernel.direct.command_sequence,
            Some(browser.identity()),
            request,
            &kernel.direct.command_result_cache,
            None,
        )
        .await;
        assert_eq!(
            direct_carrier.error.unwrap().code,
            "local_browser_unavailable"
        );
    });
}

#[test]
fn mp11_revocation_and_identity_expiry_close_admitted_sessions() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        let (mut socket, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(verdict["kind"], "local_connected");
        // Logout clears the Cloud pairing; the authority watcher retires the session.
        let mut unpaired = kernel.projection.snapshot();
        unpaired.cloud_relay = None;
        kernel.projection.update(unpaired);
        let close = next_json(&mut socket).await;
        assert_eq!(
            close,
            serde_json::json!({"kind": "close", "reason": "local browser authority revoked"})
        );
        // An unpaired kernel cannot mint another grant.
        assert_eq!(
            kernel
                .mint(&browser, browser.identity())
                .await
                .unwrap_err()
                .code,
            "local_browser_unavailable"
        );

        let kernel = Kernel::new();
        let mut short = browser.identity();
        short.expires_at_ms = crate::session::unix_epoch_ms() + 300;
        let grant = kernel.mint(&browser, short).await.unwrap();
        let (mut socket, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(verdict["kind"], "local_connected");
        assert_eq!(
            next_json(&mut socket).await["reason"],
            "relay token expired"
        );
    });
}

#[test]
fn mp11_authority_tracks_pairing_origin_and_kernel_key() {
    let mut config = DaemonConfig::for_tests();
    assert_eq!(current_authority(&config), None);
    config.cloud_relay = Some(PersistedCloudRelayProfile {
        api_url: "https://cloud.example.test/".into(),
        email: String::new(),
        account_id: "account-1".into(),
        user_id: "user-1".into(),
        account_slug: String::new(),
        realm_id: "realm-1".into(),
        relay_url: String::new(),
        issuer_id: String::new(),
        client_id: None,
        client_alias: None,
        machine_id: None,
        machine_alias: None,
        machine_credential: None,
        cloud_session_token: None,
        cloud_session_expires_at_ms: None,
        token_expires_at_ms: None,
    });
    let paired = current_authority(&config).unwrap();
    assert_eq!(paired.origin, ORIGIN);
    config.relay_public_key = "rotated".into();
    assert_ne!(current_authority(&config), Some(paired));

    assert_eq!(paired_cloud_origin("http://cloud.example.test"), None);
    assert_eq!(
        paired_cloud_origin("http://127.0.0.1:3000/api").as_deref(),
        Some("http://127.0.0.1:3000")
    );
    assert_eq!(
        paired_cloud_origin("https://val.example:8443/x").as_deref(),
        Some("https://val.example:8443")
    );
    assert_eq!(configured_port(None), Some(DEFAULT_LOCAL_BROWSER_PORT));
    assert_eq!(configured_port(Some("off")), None);
    assert_eq!(configured_port(Some("43999")), Some(43999));
}

#[test]
fn mp08_mp11_local_browser_wire_is_bound_to_protocol456() {
    use sha2::{Digest, Sha256};
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 463);
    let payload = EncryptedRelayPayload {
        sender_public_key: "key".into(),
        nonce: "nonce".into(),
        ciphertext: "ciphertext".into(),
    };
    let server = [
        admission::ServerHandshakeFrame::LocalChallenge {
            kernel_id: "kernel",
            endpoint_epoch: "epoch",
            challenge: "challenge",
        },
        admission::ServerHandshakeFrame::LocalConnected {
            daemon_public_key: "kernel-key".into(),
            proof: payload.clone(),
        },
        admission::ServerHandshakeFrame::Close { reason: "reason" },
    ]
    .map(|frame| serde_json::to_value(frame).unwrap());
    let connect = serde_json::json!({"kind": "local_connect", "proof": payload});
    assert!(matches!(
        serde_json::from_value::<admission::ClientHandshakeFrame>(connect.clone()).unwrap(),
        admission::ClientHandshakeFrame::LocalConnect { .. }
    ));
    assert!(serde_json::from_value::<admission::ClientHandshakeFrame>(
        serde_json::json!({"kind": "local_connect", "proof": payload, "grant": "leak"})
    )
    .is_err());
    let wire = serde_json::json!({
        "path": LOCAL_BROWSER_PATH,
        "default_port": DEFAULT_LOCAL_BROWSER_PORT,
        "request": {"local_browser_connect": {}},
        "response": ["LocalBrowserConnectIssued", "endpoint", "grant", "kernel_id", "endpoint_epoch", "expires_at_ms"],
        "proof": ["grant", "challenge", "origin", "kernel_id", "endpoint_epoch"],
        "kernel_proof": ["grant", "challenge"],
        "client": connect,
        "server": server,
    });
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
        "6f99800ac7b32c4bc6cb56a7f521eb9cc46d687b9742ba30bbe7051a99fb1b8f"
    );
}
