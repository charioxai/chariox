//! MP-08/MP-11: direct loopback admission over real sockets and real dispatch.

use super::super::daemon_requests::RelayRequestOutcome;
use super::*;
use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
use crate::local::{ListSessionsRequest, LocalDaemonRequest};
use crate::runtime_transport::command_cache::CommandResultCache;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

const ORIGIN: &str = "https://cloud.example.test";
// MP-11: renewal warnings and their capture have process-wide state.
static BROWSER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Debug builds of the shared dispatch path need the kernel's large stacks.
fn large_stack(test: impl std::future::Future<Output = ()> + Send + 'static) {
    let _guard = BROWSER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

pub(super) struct Browser {
    private_key: String,
    public_key: String,
}

impl Browser {
    pub(super) fn new() -> Self {
        let private_key = relay_crypto::generate_private_key_base64();
        let public_key = relay_crypto::public_key_from_private_key_base64(&private_key).unwrap();
        Self {
            private_key,
            public_key,
        }
    }

    pub(super) fn identity(&self) -> RelayCallerIdentity {
        RelayCallerIdentity {
            realm_id: "realm-1".into(),
            subject: "browser:user-1:tab".into(),
            subject_kind: RelaySubjectKind::Client,
            expires_at_ms: crate::session::unix_epoch_ms() + GRANT_TTL_MS,
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
        Self::build(paired_at_start, Some(0))
    }

    fn build(paired_at_start: bool, port: Option<u16>) -> Self {
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
            port,
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
fn mp08_default_endpoint_lets_several_kernels_share_a_machine() {
    large_stack(async {
        let browser = Browser::new();
        let first = Kernel::build(true, configured_port(None));
        let second = Kernel::build(true, configured_port(None));
        let first_grant = first.mint(&browser, browser.identity()).await.unwrap();
        let second_grant = second.mint(&browser, browser.identity()).await.unwrap();
        assert_ne!(first_grant.endpoint, second_grant.endpoint);
        for (kernel, grant) in [(&first, &first_grant), (&second, &second_grant)] {
            let (_socket, verdict) = connect(kernel, &browser, grant).await;
            assert_eq!(verdict["kind"], "local_connected", "{verdict}");
        }
    });
}

#[test]
fn mp11_idle_local_connections_cannot_pin_handshake_slots() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        let address = grant
            .endpoint
            .trim_start_matches("ws://")
            .trim_end_matches(LOCAL_BROWSER_PATH);
        let mut idle = Vec::new();
        for _ in 0..admission::MAX_PENDING_HANDSHAKES * 2 {
            idle.push(tokio::net::TcpStream::connect(address).await.unwrap());
        }
        sleep(Duration::from_millis(200)).await;
        // Well inside the five-second handshake deadline the idle peers hold.
        let (_socket, verdict) =
            timeout(Duration::from_secs(2), connect(&kernel, &browser, &grant))
                .await
                .expect("paired browser admitted while idle peers hold the endpoint");
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
    assert_eq!(configured_port(None), Some(0));
    assert_eq!(configured_port(Some("off")), None);
    assert_eq!(configured_port(Some("43999")), Some(43999));
}

#[test]
fn mp08_mp11_local_browser_wire_is_bound_to_protocol473() {
    use sha2::{Digest, Sha256};
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 473);
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
        "default_port": configured_port(None),
        "request": {"local_browser_connect": {}},
        "terminal_request": {"local_terminal_connect": {}},
        "terminal_renew_request": {"local_terminal_renew": {"grant": "grant", "sequence": 1}},
        "terminal_response": ["LocalTerminalConnectIssued", "endpoint", "grant", "kernel_id", "endpoint_epoch", "expires_at_ms", "paired_origin"],
        "terminal_renew_response": ["LocalTerminalLeaseRenewed", "expires_at_ms", "next_sequence"],
        "renew_request": {"local_browser_renew": {"grant": "grant", "sequence": 1}},
        "renew_response": ["LocalBrowserLeaseRenewed", "expires_at_ms", "next_sequence"],
        "lease_ms": GRANT_TTL_MS,
        "response": ["LocalBrowserConnectIssued", "endpoint", "grant", "kernel_id", "endpoint_epoch", "expires_at_ms"],
        "proof": ["grant", "challenge", "origin", "kernel_id", "endpoint_epoch"],
        "kernel_proof": ["grant", "challenge"],
        "client": connect,
        "server": server,
    });
    assert_eq!(
        format!("{:x}", Sha256::digest(serde_json::to_vec(&wire).unwrap())),
        "1d76b5e7fa463afc67e4222a1b7ef74061b906c5846356b12fd2035422bdc473"
    );
}

#[test]
fn mp11_admitted_session_expires_without_relay_lease_renewal() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        kernel
            .direct
            .grants
            .lock()
            .unwrap()
            .get_mut(&grant.grant)
            .unwrap()
            .expires_at_ms = crate::session::unix_epoch_ms() + 200;
        let (mut socket, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(verdict["kind"], "local_connected");
        let closed = timeout(Duration::from_secs(1), next_json(&mut socket)).await;
        assert!(
            closed.is_ok(),
            "admitted browser outlived its relay-renewed lease"
        );
        assert_eq!(closed.unwrap()["kind"], "close");
    });
}

// MP-11: real encrypted relay dispatch, not an independently authorized path.
async fn renew(
    kernel: &Kernel,
    browser: &Browser,
    identity: RelayCallerIdentity,
    grant: &Grant,
    sequence: u64,
    via_relay: bool,
) -> RelayRequestOutcome {
    let request = relay_crypto::encrypt_payload_for_peer(
        &browser.private_key,
        &kernel.daemon_public_key,
        serde_json::json!({"local_browser_renew": {"grant": grant.grant, "sequence": sequence}})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    handle_daemon_request(
        &kernel.direct.router,
        &kernel.direct.command_sequence,
        Some(identity),
        request,
        &kernel.direct.command_result_cache,
        if via_relay {
            Some(&kernel.direct)
        } else {
            None
        },
    )
    .await
}

#[test]
fn mp11_renewal_requires_relay_live_same_key_user_and_one_use_sequence() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        assert!(
            renew(&kernel, &browser, browser.identity(), &grant, 1, true)
                .await
                .error
                .is_some(),
            "unredeemed grant renewed"
        );
        let (mut socket, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(verdict["kind"], "local_connected");
        // Wait for the session's lease registration.
        for _ in 0..100 {
            if kernel
                .direct
                .leases
                .lock()
                .unwrap()
                .contains_key(&grant.grant)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            renew(&kernel, &browser, browser.identity(), &grant, 1, false)
                .await
                .error
                .is_some(),
            "direct carrier renewed itself"
        );
        let foreign = Browser::new();
        assert!(
            renew(&kernel, &foreign, foreign.identity(), &grant, 1, true)
                .await
                .error
                .is_some(),
            "foreign key renewed"
        );
        let mut other_user = browser.identity();
        other_user.user_id = Some("other-user".into());
        assert!(
            renew(&kernel, &browser, other_user, &grant, 1, true)
                .await
                .error
                .is_some(),
            "foreign user renewed"
        );
        let mut expired = browser.identity();
        expired.expires_at_ms = 1;
        assert!(
            renew(&kernel, &browser, expired, &grant, 1, true)
                .await
                .error
                .is_some(),
            "expired identity renewed"
        );
        let mut long = browser.identity();
        long.expires_at_ms += GRANT_TTL_MS;
        assert!(
            renew(&kernel, &browser, long, &grant, 1, true)
                .await
                .error
                .is_some(),
            "long-lived identity renewed"
        );
        let mut short_identity = browser.identity();
        short_identity.expires_at_ms = crate::session::unix_epoch_ms() + 20_000;
        assert!(
            renew(&kernel, &browser, short_identity.clone(), &grant, 1, true)
                .await
                .error
                .is_none(),
            "valid relay renewal denied"
        );
        assert!(
            renew(&kernel, &browser, short_identity.clone(), &grant, 1, true)
                .await
                .error
                .is_some(),
            "renewal replay accepted"
        );
        socket.close(None).await.unwrap();
        for _ in 0..100 {
            if !kernel
                .direct
                .leases
                .lock()
                .unwrap()
                .contains_key(&grant.grant)
            {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
        assert!(
            renew(&kernel, &browser, short_identity, &grant, 2, true)
                .await
                .error
                .is_some(),
            "retired session renewed"
        );
    });
}

#[test]
fn mp11_relay_renewal_extends_the_socket_only_to_short_identity_expiry() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        kernel
            .direct
            .grants
            .lock()
            .unwrap()
            .get_mut(&grant.grant)
            .unwrap()
            .expires_at_ms = crate::session::unix_epoch_ms() + 300;
        let (mut socket, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(verdict["kind"], "local_connected");
        for _ in 0..100 {
            if kernel
                .direct
                .leases
                .lock()
                .unwrap()
                .contains_key(&grant.grant)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let mut short = browser.identity();
        short.expires_at_ms = crate::session::unix_epoch_ms() + 900;
        assert!(renew(&kernel, &browser, short.clone(), &grant, 1, true)
            .await
            .error
            .is_none());
        sleep(Duration::from_millis(350)).await;
        let request = relay_crypto::encrypt_payload_for_peer(
            &browser.private_key,
            &kernel.daemon_public_key,
            &serde_json::to_vec(&LocalDaemonRequest::ListSessions(ListSessionsRequest)).unwrap(),
        )
        .unwrap();
        send_json(
            &mut socket,
            serde_json::json!({ "kind": "client_request", "request_id": "lease-live",
            "target": { "daemon_id": kernel.daemon_id }, "encrypted_request": request }),
        )
        .await;
        let response = next_json(&mut socket).await;
        assert_eq!(
            response["kind"], "client_response",
            "renewed socket did not outlive initial lease"
        );
        assert!(response["error"].is_null());
        let close = timeout(Duration::from_secs(1), next_json(&mut socket))
            .await
            .unwrap();
        assert_eq!(close["kind"], "close");
        assert!(
            renew(&kernel, &browser, short, &grant, 2, true)
                .await
                .error
                .is_some(),
            "expired socket revived"
        );
    });
}

fn renewed_lease(browser: &Browser, outcome: RelayRequestOutcome) -> serde_json::Value {
    assert!(
        outcome.error.is_none(),
        "renewal refused: {:?}",
        outcome.error
    );
    let plaintext = relay_crypto::decrypt_payload_for_private_key(
        &browser.private_key,
        &outcome.encrypted_response.unwrap(),
    )
    .unwrap();
    serde_json::from_slice::<serde_json::Value>(&plaintext.plaintext).unwrap()
        ["LocalBrowserLeaseRenewed"]
        .clone()
}

async fn admitted_lease(kernel: &Kernel, browser: &Browser) -> (ClientSocket, Grant) {
    let grant = kernel.mint(browser, browser.identity()).await.unwrap();
    let (socket, verdict) = connect(kernel, browser, &grant).await;
    assert_eq!(verdict["kind"], "local_connected");
    for _ in 0..100 {
        if kernel
            .direct
            .leases
            .lock()
            .unwrap()
            .contains_key(&grant.grant)
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    (socket, grant)
}

#[test]
fn mp11_short_leases_tolerate_bounded_clock_skew_with_cloud() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();
        let now = crate::session::unix_epoch_ms;

        // This kernel's clock is 5 s behind Cloud: a fresh 30-second identity
        // looks like 35 s, but the lease stays 30 s on this clock.
        let mut behind = browser.identity();
        behind.expires_at_ms = now() + 35_000;
        let grant = kernel.mint(&browser, behind.clone()).await.unwrap();
        assert!(grant.expires_at_ms <= now() + 30_000);
        let (_socket, grant) = admitted_lease(&kernel, &browser).await;
        let lease = renewed_lease(
            &browser,
            renew(&kernel, &browser, behind, &grant, 1, true).await,
        );
        assert!(lease["expires_at_ms"].as_u64().unwrap() <= now() + 30_000);

        // This kernel's clock is 15 s ahead: the lease ends with the identity,
        // because every relay dispatch requires a live identity, and still
        // outlasts the 10-second renewal cadence.
        let mut ahead = browser.identity();
        ahead.expires_at_ms = now() + 15_000;
        let lease = renewed_lease(
            &browser,
            renew(&kernel, &browser, ahead.clone(), &grant, 2, true).await,
        );
        assert_eq!(lease["expires_at_ms"], ahead.expires_at_ms);

        // Beyond the allowance, long-lived and expired identities stay refused.
        let mut long = browser.identity();
        long.expires_at_ms = now() + GRANT_TTL_MS + CLOCK_SKEW_ALLOWANCE_MS + 1_000;
        assert_eq!(
            kernel.mint(&browser, long.clone()).await.unwrap_err().code,
            "unauthorized"
        );
        assert!(renew(&kernel, &browser, long, &grant, 3, true)
            .await
            .error
            .is_some());
        let mut expired = browser.identity();
        expired.expires_at_ms = now() - 1;
        assert!(renew(&kernel, &browser, expired, &grant, 3, true)
            .await
            .error
            .is_some());
    });
}

#[test]
fn mp11_renewal_recovers_a_lost_response_without_accepting_replays() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();
        let (_socket, grant) = admitted_lease(&kernel, &browser).await;
        // Sequence 1 was spent but its response was lost: the browser retries
        // with the next sequence, whether or not the kernel applied 1.
        let lease = renewed_lease(
            &browser,
            renew(&kernel, &browser, browser.identity(), &grant, 2, true).await,
        );
        assert_eq!(lease["next_sequence"], 3);
        for replay in [1, 2] {
            assert!(
                renew(&kernel, &browser, browser.identity(), &grant, replay, true)
                    .await
                    .error
                    .is_some(),
                "sequence {replay} replayed"
            );
        }
        let lease = renewed_lease(
            &browser,
            renew(&kernel, &browser, browser.identity(), &grant, 3, true).await,
        );
        assert_eq!(lease["next_sequence"], 4);
    });
}

// MP-08/MP-11: a clock step has an actionable reason and does not spend the sequence.
#[test]
fn mp11_renewal_skew_refusal_is_distinct_and_sequence_stays_usable() {
    large_stack(async {
        super::super::daemon_requests::renewal_refusals::reset_for_test();
        let capture = crate::logging::capture::start();
        let kernel = Kernel::new();
        let browser = Browser::new();
        let (_socket, grant) = admitted_lease(&kernel, &browser).await;
        let mut skewed = browser.identity();
        skewed.expires_at_ms += CLOCK_SKEW_ALLOWANCE_MS + 1_000;
        let error = renew(&kernel, &browser, skewed, &grant, 1, true)
            .await
            .error
            .expect("skewed identity renewed");
        assert!(
            capture.records().iter().any(|record| record
                .contains("local browser lease renewal refused")
                && record.contains("local_browser_lease_clock_skew")
                && record.contains("system clock is synchronized")),
            "encrypted renewal refusal produced no actionable kernel warning"
        );
        assert_eq!(error.code, "local_browser_lease_clock_skew");
        assert!(error.message.contains("system clock is synchronized"));
        assert!(error
            .message
            .contains("expires more than 40 seconds ahead of the kernel clock"));
        // An expired identity (including a kernel clock ahead of Cloud) is
        // refused by ordinary relay authentication, not this ceiling check.
        let mut expired = browser.identity();
        expired.expires_at_ms = crate::session::unix_epoch_ms() - 1;
        let error = renew(&kernel, &browser, expired, &grant, 1, true)
            .await
            .error
            .expect("expired identity renewed");
        assert_eq!(error.code, "unauthorized");
        let lease = renewed_lease(
            &browser,
            renew(&kernel, &browser, browser.identity(), &grant, 1, true).await,
        );
        assert_eq!(lease["next_sequence"], 2);
    });
}

// MP-08/MP-11: all carriers share the process warning budget, including relay-only refusals.
#[test]
fn mp11_renewal_refusal_warning_budget_is_shared_across_carriers() {
    large_stack(async {
        super::super::daemon_requests::renewal_refusals::reset_for_test();
        let prior_capture = crate::logging::capture::start();
        crate::logging::warn_with_fields(
            "daemon.local_browser",
            "local browser lease renewal refused",
            serde_json::json!({"code": "local_browser_lease_denied"}),
        );
        let capture = crate::logging::capture::start();
        let capture_start = capture.records().len();
        let browser = Browser::new();
        for _ in 0..2 {
            let kernel = Kernel::new();
            let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
            for via_relay in [true, false] {
                assert!(
                    renew(&kernel, &browser, browser.identity(), &grant, 1, via_relay)
                        .await
                        .error
                        .is_some()
                );
            }
        }
        assert_eq!(
            capture
                .records()
                .iter()
                .skip(capture_start)
                .filter(|record| {
                    record.contains("local browser lease renewal refused")
                        && (record.contains("local_browser_lease_denied")
                            || record.contains("local_browser_unavailable"))
                })
                .count(),
            1,
            "carriers must share the kernel process warning budget"
        );
        drop(prior_capture);
    });
}

#[test]
fn mp11_admitted_direct_command_settles_once_after_disconnect_and_relay_retry() {
    large_stack(async {
        let kernel = Kernel::new();
        let browser = Browser::new();
        let grant = kernel.mint(&browser, browser.identity()).await.unwrap();
        let (mut socket, verdict) = connect(&kernel, &browser, &grant).await;
        assert_eq!(verdict["kind"], "local_connected");
        let (reserved, release) = kernel
            .direct
            .command_result_cache
            .pause_next_dispatch_for_test()
            .await;
        let workspace = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let request = LocalDaemonRequest::CreateSession(crate::session::CreateSessionRequest::new(
            &workspace, &workspace,
        ));
        let envelope =
            serde_json::json!({ "command_id": "mp11-disconnect-create", "request": request });
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &browser.private_key,
            &kernel.daemon_public_key,
            envelope.to_string().as_bytes(),
        )
        .unwrap();
        send_json(&mut socket, serde_json::json!({ "kind": "client_request", "request_id": "create", "target": {"daemon_id": kernel.daemon_id}, "encrypted_request": encrypted })).await;
        timeout(Duration::from_secs(2), reserved)
            .await
            .expect("command was not reserved")
            .unwrap();
        socket.close(None).await.unwrap();
        for _ in 0..200 {
            if !kernel
                .direct
                .leases
                .lock()
                .unwrap()
                .contains_key(&grant.grant)
            {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
        assert!(
            !kernel
                .direct
                .leases
                .lock()
                .unwrap()
                .contains_key(&grant.grant),
            "direct session did not close"
        );
        let _ = release.send(());
        let retry = handle_daemon_request(
            &kernel.direct.router,
            &kernel.direct.command_sequence,
            Some(browser.identity()),
            encrypted.clone(),
            &kernel.direct.command_result_cache,
            Some(&kernel.direct),
        );
        let outcome = timeout(Duration::from_secs(2), retry)
            .await
            .expect("accepted direct command left a pending cache receipt after disconnect");
        assert!(outcome.error.is_none(), "{:?}", outcome.error);
        let first = relay_crypto::decrypt_payload_for_private_key(
            &browser.private_key,
            &outcome.encrypted_response.unwrap(),
        )
        .unwrap();
        let replay = handle_daemon_request(
            &kernel.direct.router,
            &kernel.direct.command_sequence,
            Some(browser.identity()),
            encrypted,
            &kernel.direct.command_result_cache,
            Some(&kernel.direct),
        )
        .await;
        let second = relay_crypto::decrypt_payload_for_private_key(
            &browser.private_key,
            &replay.encrypted_response.unwrap(),
        )
        .unwrap();
        assert_eq!(
            first.plaintext, second.plaintext,
            "relay retry duplicated the mutation"
        );
        let sessions = relay_crypto::encrypt_payload_for_peer(
            &browser.private_key,
            &kernel.daemon_public_key,
            serde_json::to_vec(&LocalDaemonRequest::ListSessions(ListSessionsRequest))
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let listed = handle_daemon_request(
            &kernel.direct.router,
            &kernel.direct.command_sequence,
            Some(browser.identity()),
            sessions,
            &kernel.direct.command_result_cache,
            Some(&kernel.direct),
        )
        .await;
        let list = relay_crypto::decrypt_payload_for_private_key(
            &browser.private_key,
            &listed.encrypted_response.unwrap(),
        )
        .unwrap();
        let list: serde_json::Value = serde_json::from_slice(&list.plaintext).unwrap();
        assert_eq!(
            list["SessionsListed"]["sessions"].as_array().unwrap().len(),
            1
        );
    });
}

// MP-08/MP-11: an ordinary paired CLI identity may outlive its explicit
// terminal lease. Only authenticated relay renewal extends that short lease.
async fn terminal_request(
    kernel: &Kernel,
    terminal: &Browser,
    identity: RelayCallerIdentity,
    body: serde_json::Value,
    via_relay: bool,
) -> Result<serde_json::Value, RelayError> {
    let request = relay_crypto::encrypt_payload_for_peer(
        &terminal.private_key,
        &kernel.daemon_public_key,
        body.to_string().as_bytes(),
    )
    .unwrap();
    let outcome = handle_daemon_request(
        &kernel.direct.router,
        &kernel.direct.command_sequence,
        Some(identity),
        request,
        &kernel.direct.command_result_cache,
        if via_relay {
            Some(&kernel.direct)
        } else {
            None
        },
    )
    .await;
    if let Some(error) = outcome.error {
        return Err(error);
    }
    let plaintext = relay_crypto::decrypt_payload_for_private_key(
        &terminal.private_key,
        &outcome.encrypted_response.unwrap(),
    )
    .unwrap();
    Ok(serde_json::from_slice(&plaintext.plaintext).unwrap())
}

#[test]
fn mp08_mp11_terminal_short_lease_uses_shared_bound_admission() {
    large_stack(async {
        let kernel = Kernel::new();
        let terminal = Browser::new();
        let mut identity = terminal.identity();
        identity.subject = "terminal:cli-1".into();
        identity.expires_at_ms = crate::session::unix_epoch_ms() + 900_000;
        assert!(
            kernel.mint(&terminal, identity.clone()).await.is_err(),
            "browser admission must still refuse long-lived identities"
        );
        let before = crate::session::unix_epoch_ms();
        let issued = terminal_request(
            &kernel,
            &terminal,
            identity.clone(),
            serde_json::json!({"local_terminal_connect": {}}),
            true,
        )
        .await
        .unwrap();
        let issued = &issued["LocalTerminalConnectIssued"];
        assert_eq!(issued["paired_origin"], ORIGIN);
        assert!(issued["expires_at_ms"].as_u64().unwrap() <= before + 30_100);
        let grant: Grant = serde_json::from_value(issued.clone()).unwrap();
        let (mut socket, verdict) = connect(&kernel, &terminal, &grant).await;
        assert_eq!(verdict["kind"], "local_connected");
        let renewal =
            serde_json::json!({"local_terminal_renew": {"grant": grant.grant, "sequence": 1}});
        assert!(
            terminal_request(&kernel, &terminal, identity.clone(), renewal.clone(), false)
                .await
                .is_err()
        );
        let mut foreign = identity.clone();
        foreign.user_id = Some("another-user".into());
        assert!(
            terminal_request(&kernel, &terminal, foreign, renewal.clone(), true)
                .await
                .is_err()
        );
        let mut unbound = identity.clone();
        unbound.public_key_thumbprint = None;
        assert!(
            terminal_request(&kernel, &terminal, unbound, renewal.clone(), true)
                .await
                .is_err()
        );
        let renewed = terminal_request(&kernel, &terminal, identity.clone(), renewal.clone(), true)
            .await
            .unwrap();
        assert_eq!(renewed["LocalTerminalLeaseRenewed"]["next_sequence"], 2);
        assert!(
            renewed["LocalTerminalLeaseRenewed"]["expires_at_ms"]
                .as_u64()
                .unwrap()
                <= crate::session::unix_epoch_ms() + 30_000
        );
        assert!(
            terminal_request(&kernel, &terminal, identity.clone(), renewal, true)
                .await
                .is_err()
        );
        assert!(
            renew(&kernel, &terminal, terminal.identity(), &grant, 2, true)
                .await
                .error
                .is_some(),
            "browser renewal cannot change a terminal lease"
        );
        socket.close(None).await.unwrap();
        assert!(terminal_request(
            &kernel,
            &terminal,
            identity,
            serde_json::json!({"local_terminal_connect": {}}),
            false
        )
        .await
        .is_err());
    });
}
