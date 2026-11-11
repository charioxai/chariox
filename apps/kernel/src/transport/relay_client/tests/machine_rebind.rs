//! A kernel enrolled under one machine id and started on a computer that derives
//! another (Landing 8) must never register with a pair the relay refuses.
//! Real connector, relay and sockets; a local Cloud stands in for the hosted one.
use super::support::*;
use base64::Engine;
use chariox_relay::auth::{
    RelayAction, RelayAuthVerifier, RelaySubjectKind, RelayTokenClaims, ScopedTokenVerifier,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const OLD_MACHINE: &str = "machine-enrolled-before";
const NEW_MACHINE: &str = "machine-derived-from-os";

#[derive(Clone, Copy)]
enum Rebind {
    Accept,
    Refuse,
    Unavailable,
}

struct LocalCloud {
    api_url: String,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
}

impl LocalCloud {
    fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| seen.as_str() == path)
            .count()
    }
}

/// The same token string for the same kernel and machine, so the relay's verifier can list it.
fn relay_token(config: &DaemonConfig, machine_id: &str) -> String {
    static EXPIRES_AT_SECS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let claims = serde_json::json!({
        "exp": *EXPIRES_AT_SECS.get_or_init(|| crate::session::unix_epoch_ms() / 1_000 + 3_600),
        "machine_id": machine_id,
        "public_key_thumbprint":
            crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key),
    });
    format!(
        "header.{}.signature-{machine_id}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}

/// Cloud issues tokens from the enrolled grant's machine, as the hosted one does.
async fn local_cloud(config: DaemonConfig, mode: Rebind) -> LocalCloud {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = requests.clone();
    let grant_machine = Arc::new(std::sync::Mutex::new(OLD_MACHINE.to_string()));
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let (config, seen, grant_machine) =
                (config.clone(), seen.clone(), grant_machine.clone());
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut chunk = [0_u8; 8192];
                let body_start = loop {
                    let read = stream.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    request.extend_from_slice(&chunk[..read]);
                    if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        break at + 4;
                    }
                };
                let head = String::from_utf8_lossy(&request[..body_start]).to_string();
                let length = head
                    .to_ascii_lowercase()
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: ")?.trim().parse().ok())
                    .unwrap_or(0_usize);
                while request.len() < body_start + length {
                    let read = stream.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    request.extend_from_slice(&chunk[..read]);
                }
                let body: serde_json::Value =
                    serde_json::from_slice(&request[body_start..]).unwrap_or_default();
                let path = head
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                seen.lock().unwrap().push(path.clone());
                let (status, payload) = match path.as_str() {
                    "/relay/token" => {
                        let machine = grant_machine.lock().unwrap().clone();
                        // Cloud refuses a token request for another machine than the grant's.
                        if body["machineId"] != machine.as_str() {
                            (
                                "403 Forbidden",
                                r#"{"error":{"code":"authorization_denied","message":"machine mismatch"}}"#.to_string(),
                            )
                        } else {
                            let expires = chrono::Utc::now() + chrono::Duration::minutes(10);
                            (
                                "201 Created",
                                serde_json::json!({"token": relay_token(&config, &machine), "expiresAt": expires.to_rfc3339()}).to_string(),
                            )
                        }
                    }
                    "/kernels/rebind-machine" => match mode {
                        Rebind::Accept => {
                            *grant_machine.lock().unwrap() =
                                body["machineId"].as_str().unwrap().to_string();
                            (
                                "200 OK",
                                serde_json::json!({"machineId": body["machineId"]}).to_string(),
                            )
                        }
                        Rebind::Refuse => (
                            "403 Forbidden",
                            r#"{"error":{"code":"authorization_denied","message":"not allowed"}}"#
                                .to_string(),
                        ),
                        Rebind::Unavailable => (
                            "503 Service Unavailable",
                            r#"{"error":{"code":"dependency_unavailable","message":"later"}}"#
                                .to_string(),
                        ),
                    },
                    _ => ("200 OK", "{}".to_string()),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    LocalCloud { api_url, requests }
}

/// A kernel enrolled under `OLD_MACHINE` that now runs on a computer deriving `NEW_MACHINE`,
/// with the relay token it persisted before the upgrade.
fn upgraded_kernel(base: &DaemonConfig, relay_url: &str, api_url: &str) -> DaemonConfig {
    let mut config = base.clone();
    config.host_machine_id = NEW_MACHINE.into();
    config.relay_heartbeat_ms = 500;
    config.relay_url = Some(relay_url.into());
    config.relay_token = Some(relay_token(&config, OLD_MACHINE));
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        api_url: api_url.into(),
        relay_url: relay_url.into(),
        account_id: "account".into(),
        user_id: "user".into(),
        realm_id: "realm".into(),
        machine_id: Some(OLD_MACHINE.into()),
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-enrollment".into()),
        kernel_public_key_thumbprint: Some(
            crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key),
        ),
        token_expires_at_ms: Some(crate::session::unix_epoch_ms() + 600_000),
        ..Default::default()
    });
    config
}

fn start_connector(
    config: &DaemonConfig,
) -> (
    Arc<CommandRouter>,
    watch::Sender<bool>,
    tokio::task::JoinHandle<()>,
) {
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap()));
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        app,
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(run_daemon_relay_connector_with_router(
        router.clone(),
        Arc::new(RwLock::new(RelayClientState::default())),
        stopped,
    ));
    (router, stop, task)
}

#[test]
fn upgraded_kernel_rebinds_then_stays_registered_with_a_matching_token() {
    run_async_with_large_test_stack("machine-rebind-accepted", || async {
        let _guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let base = DaemonConfig::for_tests();
        let token_claims = |machine: &str| RelayTokenClaims {
            issuer: "machrebind-test".into(),
            subject: base.daemon_id.clone(),
            subject_kind: RelaySubjectKind::Kernel,
            realm_id: "default".into(),
            allowed_actions: vec![
                RelayAction::DaemonRegister,
                RelayAction::DaemonHeartbeat,
                RelayAction::PacketRoute,
            ],
            allowed_targets: None,
            issued_at_ms: crate::session::unix_epoch_ms(),
            expires_at_ms: crate::session::unix_epoch_ms() + 600_000,
            token_id: machine.into(),
            account_id: None,
            organization_id: None,
            user_id: None,
            device_id: None,
            machine_id: Some(machine.into()),
            client_id: None,
            session_id: None,
            public_key_thumbprint: None,
            entitlements_version: None,
        };
        // The relay trusts the tokens Cloud issues for either machine and applies its own
        // rule that a token's machine equals the registration's.
        let server = RelayServer::with_auth_verifier(
            RelayConfig {
                host: "127.0.0.1".into(),
                port: 0,
                shared_token: None,
            },
            RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(
                BTreeMap::from([
                    (relay_token(&base, OLD_MACHINE), token_claims(OLD_MACHINE)),
                    (relay_token(&base, NEW_MACHINE), token_claims(NEW_MACHINE)),
                ]),
                BTreeMap::new(),
                None,
            )),
        );
        let listener = server.bind_listener().await.unwrap();
        let relay_url = format!("ws://{}", listener.local_addr().unwrap());
        let registry = server.registry();
        let (stop_server, stopped) = oneshot::channel();
        let relay_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let cloud = local_cloud(base.clone(), Rebind::Accept).await;
        let config = upgraded_kernel(&base, &relay_url, &cloud.api_url);
        let (router, stop, task) = start_connector(&config);

        wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;
        // Past several heartbeats: the relay keeps the kernel.
        sleep(Duration::from_millis(2_500)).await;
        let registered = registry
            .read()
            .await
            .daemon(&config.daemon_id)
            .map(|d| (d.machine_id.clone(), d.auth_token.clone()));
        let (machine, token) = registered.expect("kernel is still registered after the heartbeats");
        assert_eq!(machine, NEW_MACHINE);
        assert_eq!(token, relay_token(&base, NEW_MACHINE));
        assert_eq!(
            cloud.count("/kernels/rebind-machine"),
            1,
            "one rebind, not one per tick"
        );
        let saved = router.relay_config_snapshot();
        assert_eq!(
            saved.cloud_relay.unwrap().machine_id.as_deref(),
            Some(NEW_MACHINE)
        );

        let _ = stop.send(true);
        let _ = task.await;
        let _ = stop_server.send(());
        let _ = relay_task.await;
    });
}

/// Counts the connections the kernel opens; it never answers.
async fn silent_relay() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let connections = Arc::new(AtomicUsize::new(0));
    let counted = connections.clone();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            held.push(stream);
        }
    });
    (url, connections)
}

#[test]
fn kernel_whose_rebind_is_unavailable_keeps_off_the_relay_and_retries() {
    run_async_with_large_test_stack("machine-rebind-unavailable", || async {
        let _guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let (relay_url, connections) = silent_relay().await;
        let base = DaemonConfig::for_tests();
        let cloud = local_cloud(base.clone(), Rebind::Unavailable).await;
        let config = upgraded_kernel(&base, &relay_url, &cloud.api_url);
        let (router, stop, task) = start_connector(&config);

        sleep(Duration::from_millis(3_000)).await;
        assert_eq!(
            connections.load(Ordering::SeqCst),
            0,
            "a registration for {NEW_MACHINE} with a token for {OLD_MACHINE} is refused by the relay; do not send it"
        );
        assert!(
            cloud.count("/kernels/rebind-machine") >= 1,
            "the rebind is retried"
        );
        assert_eq!(
            router
                .relay_config_snapshot()
                .cloud_relay
                .unwrap()
                .machine_id
                .as_deref(),
            Some(OLD_MACHINE),
            "an unanswered rebind changes nothing"
        );
        let _ = stop.send(true);
        let _ = task.await;
    });
}

#[test]
fn kernel_whose_rebind_is_refused_ends_unlinked_without_a_reconnect_loop() {
    run_async_with_large_test_stack("machine-rebind-refused", || async {
        let _guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let (relay_url, connections) = silent_relay().await;
        let base = DaemonConfig::for_tests();
        let cloud = local_cloud(base.clone(), Rebind::Refuse).await;
        let config = upgraded_kernel(&base, &relay_url, &cloud.api_url);
        let (router, stop, task) = start_connector(&config);

        sleep(Duration::from_millis(1_500)).await;
        let settled = cloud.requests.lock().unwrap().len();
        sleep(Duration::from_millis(2_500)).await;
        assert_eq!(
            connections.load(Ordering::SeqCst),
            0,
            "no registration for a refused pair"
        );
        assert_eq!(
            cloud.count("/kernels/rebind-machine"),
            1,
            "a refusal is final, not retried"
        );
        assert_eq!(
            cloud.requests.lock().unwrap().len(),
            settled,
            "no further Cloud calls once it is final"
        );
        assert!(
            router.relay_config_snapshot().cloud_relay.is_none(),
            "the kernel ends in the definite re-link state"
        );
        let _ = stop.send(true);
        let _ = task.await;
    });
}
