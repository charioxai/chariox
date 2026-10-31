//! A relay that refuses the kernel's token, or drops it right after it
//! registered, must see a bounded number of reconnects, not one per second.
use super::*;
use chariox_relay::auth::{
    encode_scoped_hmac_token, RelayAction, RelayAuthVerifier, RelaySubjectKind, RelayTokenClaims,
};
use std::sync::atomic::{AtomicUsize, Ordering};

const OBSERVATION_WINDOW: Duration = Duration::from_millis(6_500);

/// Forwards every accepted connection to `target` and counts them.
async fn counting_proxy(target: std::net::SocketAddr) -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&accepted);
    tokio::spawn(async move {
        while let Ok((mut inbound, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                if let Ok(mut outbound) = tokio::net::TcpStream::connect(target).await {
                    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                }
            });
        }
    });
    (url, accepted)
}

async fn count_connector_connections(
    relay_url: String,
    relay_token: String,
    accepted: &AtomicUsize,
) -> usize {
    let config = crate::config::DaemonConfig::for_tests();
    let app = DaemonApp::bootstrap(config).expect("daemon should bootstrap");
    let (stop_tx, stop_rx) = watch::channel(false);
    let connector = tokio::spawn(run_daemon_relay_connector_with_static_relay(
        Arc::new(Mutex::new(app)),
        Arc::new(RwLock::new(RelayClientState::default())),
        stop_rx,
        relay_url,
        relay_token,
    ));
    sleep(OBSERVATION_WINDOW).await;
    let connections = accepted.load(Ordering::SeqCst);
    stop_tx.send(true).unwrap();
    let _ = timeout(Duration::from_secs(5), connector).await;
    connections
}

fn expired_kernel_token(issuer: &str, issuer_secret: &str) -> String {
    let daemon_id = crate::config::DaemonConfig::for_tests().daemon_id;
    let now = crate::session::unix_epoch_ms();
    encode_scoped_hmac_token(
        &RelayTokenClaims {
            issuer: issuer.into(),
            subject: daemon_id.clone(),
            subject_kind: RelaySubjectKind::Kernel,
            realm_id: "default".into(),
            allowed_actions: vec![RelayAction::DaemonRegister, RelayAction::DaemonHeartbeat],
            allowed_targets: Some(vec![daemon_id]),
            issued_at_ms: now - 600_000,
            expires_at_ms: now - 300_000,
            token_id: "refused-token-test".into(),
            account_id: None,
            organization_id: None,
            user_id: Some(crate::session::DEFAULT_LOCAL_USER_ID.into()),
            device_id: None,
            machine_id: None,
            client_id: None,
            session_id: None,
            public_key_thumbprint: None,
            entitlements_version: None,
        },
        issuer_secret,
    )
    .unwrap()
}

/// b3 drill: the kernel connector against a relay binary that is already running.
/// The relay's `relay_peer_removed` log lines are the count.
#[tokio::test]
#[ignore = "drill: needs CHARIOX_TOKLOOP_RELAY_URL and CHARIOX_TOKLOOP_ISSUER_SECRET for a running relay"]
async fn drill_expired_kernel_token_against_relay_binary() {
    let relay_url = std::env::var("CHARIOX_TOKLOOP_RELAY_URL").unwrap();
    let issuer_secret = std::env::var("CHARIOX_TOKLOOP_ISSUER_SECRET").unwrap();
    let seconds: u64 = std::env::var("CHARIOX_TOKLOOP_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(60);
    let token = expired_kernel_token("drill", &issuer_secret);
    let app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (stop_tx, stop_rx) = watch::channel(false);
    let connector = tokio::spawn(run_daemon_relay_connector_with_static_relay(
        Arc::new(Mutex::new(app)),
        Arc::new(RwLock::new(RelayClientState::default())),
        stop_rx,
        relay_url,
        token,
    ));
    sleep(Duration::from_secs(seconds)).await;
    stop_tx.send(true).unwrap();
    let _ = timeout(Duration::from_secs(5), connector).await;
}

#[tokio::test]
async fn expired_relay_token_is_not_presented_again() {
    let issuer_secret = "refused-token-test-secret-0123456789abcdef".to_string();
    let token = expired_kernel_token("refused-token-test", &issuer_secret);
    let relay = chariox_relay::server::RelayServer::with_auth_verifier(
        chariox_relay::config::RelayConfig {
            host: "127.0.0.1".into(),
            port: 0,
            shared_token: None,
        },
        RelayAuthVerifier::scoped_hmac(
            std::collections::BTreeMap::from([("refused-token-test".into(), issuer_secret)]),
            None,
        ),
    );
    let listener = relay.bind_listener().await.unwrap();
    let (proxy_url, accepted) = counting_proxy(listener.local_addr().unwrap()).await;
    let (relay_stop_tx, relay_stop_rx) = tokio::sync::oneshot::channel::<()>();
    let relay_task = tokio::spawn(async move {
        relay
            .run_listener_until(listener, async {
                let _ = relay_stop_rx.await;
            })
            .await
            .unwrap();
    });

    let connections = count_connector_connections(proxy_url, token, &accepted).await;

    let _ = relay_stop_tx.send(());
    let _ = relay_task.await;
    assert!(
        connections <= 1,
        "an expired token was presented {connections} times in {OBSERVATION_WINDOW:?}"
    );
}

#[tokio::test]
async fn connection_the_relay_drops_after_registration_backs_off() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("ws://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&accepted);
    let relay = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                use futures_util::StreamExt;
                if let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await {
                    let _ = socket.next().await;
                    let _ = socket.close(None).await;
                }
            });
        }
    });

    let connections = count_connector_connections(relay_url, "unused".into(), &accepted).await;

    relay.abort();
    assert!(
        connections <= 4,
        "a relay that drops every registration saw {connections} reconnects in {OBSERVATION_WINDOW:?}"
    );
}
