use super::*;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const ISSUER_SECRET: &str = "hosted-issuer-secret";
const VICTIM: &str = "victim-kernel";

struct Relay {
    url: String,
    registry: Arc<RwLock<RelayRegistry>>,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    server_task: tokio::task::JoinHandle<()>,
}

impl Relay {
    async fn start() -> Self {
        let mut issuers = BTreeMap::new();
        issuers.insert("hosted-cloud".to_string(), ISSUER_SECRET.to_string());
        let server = RelayServer::with_auth_verifier(
            RelayConfig {
                host: "127.0.0.1".to_string(),
                port: 0,
                shared_token: None,
            },
            RelayAuthVerifier::scoped_hmac(issuers, Some(10_000)),
        );
        let registry = server.registry();
        let listener = server.bind_listener().await.expect("relay listener binds");
        let addr = listener.local_addr().expect("relay address");
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server runs");
        });
        Self {
            url: format!("ws://{addr}"),
            registry,
            shutdown_tx,
            server_task,
        }
    }

    async fn connect(
        &self,
    ) -> (
        Socket,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ) {
        connect_async_with_retry(&self.url)
            .await
            .expect("kernel connects")
    }

    async fn live_peer(&self) -> Option<std::net::SocketAddr> {
        self.registry
            .read()
            .await
            .daemon_peers
            .get(&DaemonKey::new("realm".to_string(), VICTIM.to_string()))
            .copied()
    }

    async fn stop(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.server_task.await;
    }
}

/// The relay token Cloud issued the victim kernel: bound to the victim's public key.
fn victim_registration() -> DaemonRegistration {
    victim_registration_bound_to(&test_public_key(VICTIM))
}

fn victim_registration_bound_to(public_key: &str) -> DaemonRegistration {
    let mut claims = scoped_claim(
        "victim-token",
        VICTIM,
        RelaySubjectKind::Kernel,
        "realm",
        vec![RelayAction::DaemonRegister, RelayAction::DaemonHeartbeat],
        None,
    );
    claims.issuer = "hosted-cloud".to_string();
    claims.expires_at_ms = 20_000;
    claims.account_id = Some("account".to_string());
    claims.user_id = Some("user".to_string());
    claims.machine_id = Some("machine".to_string());
    claims.public_key_thumbprint = Some(
        <Sha256 as sha2::Digest>::digest(public_key.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    );
    let mut registration = test_registration(VICTIM, "machine", "Linux", 10);
    registration.auth_token = hosted_kernel_jwt(&claims, ISSUER_SECRET);
    registration.public_key = public_key.to_string();
    registration
}

async fn send(socket: &mut Socket, envelope: &RelayEnvelope) {
    socket
        .send(Message::Text(
            serde_json::to_string(envelope)
                .expect("envelope serializes")
                .into(),
        ))
        .await
        .expect("envelope sends");
}

async fn close_reason(socket: &mut Socket) -> String {
    loop {
        match timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("the relay answers the registration")
        {
            Some(Ok(Message::Text(text))) => {
                if let RelayEnvelope::Close { reason, .. } =
                    serde_json::from_str(&text).expect("relay frame decodes")
                {
                    return reason;
                }
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            other => panic!("the relay closed without a reason: {other:?}"),
        }
    }
}

async fn register_victim(relay: &Relay) -> (Socket, RelayEnvelope) {
    let (mut socket, handshake) = relay.connect().await;
    let register = signed_register(&relay.url, &handshake, victim_registration());
    send(&mut socket, &register).await;
    timeout(Duration::from_secs(2), async {
        while relay.live_peer().await.is_none() {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the real kernel registers");
    (socket, register)
}

async fn assert_victim_stays(relay: &Relay, victim: &mut Socket, peer: std::net::SocketAddr) {
    assert_no_relay_close(victim).await;
    assert_eq!(
        relay.live_peer().await,
        Some(peer),
        "the live kernel must stay registered on its own connection"
    );
}

// A leaked relay token and the kernel's public key are not the kernel: without its private key
// another process can neither register as it nor displace it.
#[tokio::test(flavor = "multi_thread")]
async fn leaked_relay_token_without_the_identity_key_cannot_displace_the_kernel() {
    let relay = Relay::start().await;
    let (mut victim, _) = register_victim(&relay).await;
    let peer = relay.live_peer().await.expect("victim registered");

    let (mut attacker, handshake) = relay.connect().await;
    let registration = victim_registration();
    let proof = crate::registration_proof::prove(
        &test_identity_key("attacker-kernel"),
        &crate::registration_proof::dialed_address(&relay.url).unwrap(),
        &registration,
        crate::registration_proof::offered_challenge(&handshake).unwrap(),
    );
    send(
        &mut attacker,
        &RelayEnvelope::DaemonRegister {
            registration,
            proof,
        },
    )
    .await;
    assert_eq!(
        close_reason(&mut attacker).await,
        "registration proof is not signed by the kernel identity key for this connection"
    );
    assert_victim_stays(&relay, &mut victim, peer).await;

    // The real kernel still reconnects and takes over its own registration.
    let (mut reconnected, handshake) = relay.connect().await;
    send(
        &mut reconnected,
        &signed_register(&relay.url, &handshake, victim_registration()),
    )
    .await;
    timeout(Duration::from_secs(2), async {
        while relay.live_peer().await == Some(peer) {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the real kernel's reconnect replaces its old connection");
    assert_eq!(close_reason(&mut victim).await, "daemon reconnected");
    relay.stop().await;
}

// A registration captured off the wire was signed for another connection's challenge.
#[tokio::test(flavor = "multi_thread")]
async fn replayed_registration_proof_is_refused() {
    let relay = Relay::start().await;
    let (mut victim, captured) = register_victim(&relay).await;
    let peer = relay.live_peer().await.expect("victim registered");

    let (mut attacker, _) = relay.connect().await;
    send(&mut attacker, &captured).await;
    assert_eq!(
        close_reason(&mut attacker).await,
        "registration proof is not signed by the kernel identity key for this connection"
    );
    assert_victim_stays(&relay, &mut victim, peer).await;

    // Nor does the victim's own connection admit its challenge twice.
    send(&mut victim, &captured).await;
    assert_eq!(
        close_reason(&mut victim).await,
        "relay connection already registered; reconnect to register again"
    );
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn proof_for_another_relay_or_without_a_signature_is_refused() {
    let relay = Relay::start().await;
    let (mut victim, _) = register_victim(&relay).await;
    let peer = relay.live_peer().await.expect("victim registered");

    let (mut other_relay, handshake) = relay.connect().await;
    let registration = victim_registration();
    let proof = crate::registration_proof::prove(
        &test_identity_key(VICTIM),
        "other-relay.example",
        &registration,
        crate::registration_proof::offered_challenge(&handshake).unwrap(),
    );
    send(
        &mut other_relay,
        &RelayEnvelope::DaemonRegister {
            registration,
            proof,
        },
    )
    .await;
    assert_eq!(
        close_reason(&mut other_relay).await,
        "registration proof is not signed by the kernel identity key for this connection"
    );

    let (mut unsigned, _) = relay.connect().await;
    send(
        &mut unsigned,
        &RelayEnvelope::DaemonRegister {
            registration: victim_registration(),
            proof: String::new(),
        },
    )
    .await;
    assert_eq!(
        close_reason(&mut unsigned).await,
        "registration proof is not a signature"
    );
    assert_victim_stays(&relay, &mut victim, peer).await;
    relay.stop().await;
}

// A heartbeat renews the token; it cannot swap in a key the kernel never proved.
#[tokio::test(flavor = "multi_thread")]
async fn heartbeat_cannot_change_the_proven_identity_key() {
    let relay = Relay::start().await;
    let (mut victim, _) = register_victim(&relay).await;
    // Even a token Cloud bound to another key renews the token, not the key.
    let registration = victim_registration_bound_to(&test_public_key("attacker-kernel"));
    send(
        &mut victim,
        &RelayEnvelope::DaemonHeartbeat {
            daemon_id: VICTIM.to_string(),
            registration: Some(registration),
        },
    )
    .await;
    assert_eq!(
        close_reason(&mut victim).await,
        "daemon identity key changed; reconnect to register it"
    );
    relay.stop().await;
}
