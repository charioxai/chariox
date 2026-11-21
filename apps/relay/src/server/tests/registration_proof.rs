use super::*;
use crate::protocol::{RelayMachinePresence, RelayUpdateNeededKernel};

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

#[tokio::test(flavor = "multi_thread")]
async fn malformed_registration_gets_a_close_reason() {
    let relay = Relay::start().await;
    let (mut kernel, _) = relay.connect().await;
    let mut registration = victim_registration();
    registration.local_protocol = Some(1012);
    kernel
        .send(Message::Text(
            serde_json::json!({"kind":"daemon_register", "registration":registration})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert!(close_reason(&mut kernel)
        .await
        .contains("missing registration proof"));
    assert!(relay.live_peer().await.is_none());
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn signed_registration_is_acknowledged() {
    let relay = Relay::start().await;
    let (mut kernel, handshake) = relay.connect().await;
    let mut registration = victim_registration();
    registration.local_protocol = Some(1013);
    send(
        &mut kernel,
        &signed_register(&relay.url, &handshake, registration),
    )
    .await;
    let frame = timeout(Duration::from_secs(2), kernel.next())
        .await
        .expect("relay acknowledges an accepted registration")
        .expect("relay remains connected")
        .expect("ack frame decodes");
    assert!(
        matches!(frame, Message::Text(text) if text.as_str() == r#"{"kind":"daemon_registered"}"#)
    );
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn protocol_1012_registers_without_receiving_the_new_envelope() {
    let relay = Relay::start().await;
    let (mut kernel, handshake) = relay.connect().await;
    let mut registration = victim_registration();
    registration.local_protocol = Some(1012);
    send(
        &mut kernel,
        &signed_register(&relay.url, &handshake, registration),
    )
    .await;
    assert!(
        timeout(Duration::from_millis(100), kernel.next())
            .await
            .is_err(),
        "the L12 kernel must not receive an unknown application envelope"
    );
    assert!(relay.live_peer().await.is_some());
    relay.stop().await;
}

/// What a kernel older than the registration proof sends: its registration with no proof, and
/// with the protocol field only from the kernels that have it.
fn old_kernel_register(registration: &DaemonRegistration, local_protocol: Option<u32>) -> String {
    let mut registration = serde_json::to_value(registration).expect("registration serializes");
    let fields = registration
        .as_object_mut()
        .expect("registration is an object");
    fields.remove("local_protocol");
    if let Some(protocol) = local_protocol {
        fields.insert("local_protocol".into(), protocol.into());
    }
    serde_json::json!({ "kind": "daemon_register", "registration": registration }).to_string()
}

/// The machine list a client of the realm reads, as the account's other kernels read it.
async fn listed_machines(relay: &Relay) -> Vec<RelayMachinePresence> {
    let mut claims = scoped_claim(
        "client-token",
        "client",
        RelaySubjectKind::Client,
        "realm",
        vec![RelayAction::ClientMetadataRead],
        None,
    );
    claims.issuer = "hosted-cloud".to_string();
    claims.expires_at_ms = 20_000;
    let (mut client, _) = relay.connect().await;
    send(
        &mut client,
        &RelayEnvelope::ClientMetadataRequest {
            request_id: "machines".to_string(),
            auth_token: hosted_kernel_jwt(&claims, ISSUER_SECRET),
            query: RelayMetadataQuery::ListLiveMachines,
        },
    )
    .await;
    loop {
        match timeout(Duration::from_secs(2), client.next())
            .await
            .expect("the relay answers the machine list")
        {
            Some(Ok(Message::Text(text))) => {
                if let RelayEnvelope::ClientMetadataResponse {
                    machines, error, ..
                } = serde_json::from_str(&text).expect("relay frame decodes")
                {
                    assert_eq!(error, None);
                    return machines.expect("a machine list");
                }
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            other => panic!("the relay closed the machine list request: {other:?}"),
        }
    }
}

fn update_needed(
    machines: &[RelayMachinePresence],
) -> Vec<(String, usize, Vec<RelayUpdateNeededKernel>)> {
    machines
        .iter()
        .map(|machine| {
            (
                machine.machine_id.clone(),
                machine.kernel_count,
                machine.update_needed_kernels.clone(),
            )
        })
        .collect()
}

// A kernel too old for this relay is refused as before, but its own machine entry names it with
// the protocol it reported, so clients say Update needed instead of losing the kernel.
#[tokio::test(flavor = "multi_thread")]
async fn a_kernel_too_old_to_register_is_listed_on_its_machine_as_update_needed() {
    let relay = Relay::start().await;
    for (reported, words) in [
        (None, "reports no protocol"),
        (Some(1011), "runs protocol 1011"),
    ] {
        let (mut old, _) = relay.connect().await;
        old.send(Message::Text(
            old_kernel_register(&victim_registration(), reported).into(),
        ))
        .await
        .expect("old registration sends");
        assert_eq!(
            close_reason(&mut old).await,
            format!(
                "update needed: this kernel {words} and is older than the relay; update Chariox on this machine and restart the kernel"
            )
        );
        assert_eq!(
            relay.live_peer().await,
            None,
            "the old kernel carries no traffic"
        );
        assert_eq!(
            update_needed(&listed_machines(&relay).await),
            vec![(
                "machine".to_string(),
                0,
                vec![RelayUpdateNeededKernel {
                    kernel_id: VICTIM.to_string(),
                    local_protocol: reported,
                }],
            )]
        );
    }

    // Updated, the kernel registers and is listed as a live kernel only.
    let (_updated, _) = register_victim(&relay).await;
    assert_eq!(
        update_needed(&listed_machines(&relay).await),
        vec![("machine".to_string(), 1, Vec::new())]
    );
    relay.stop().await;
}

// Only the kernel's own token, machine and key record it: a token bound to another key, or a
// temporary peer connection of an old kernel, leaves no entry.
#[tokio::test(flavor = "multi_thread")]
async fn an_old_registration_that_is_not_the_kernel_itself_is_not_listed() {
    let relay = Relay::start().await;
    let (mut other_key, _) = relay.connect().await;
    let mut registration = victim_registration_bound_to(&test_public_key("attacker-kernel"));
    registration.public_key = test_public_key(VICTIM);
    other_key
        .send(Message::Text(
            old_kernel_register(&registration, Some(1011)).into(),
        ))
        .await
        .expect("old registration sends");
    assert_eq!(
        close_reason(&mut other_key).await,
        "relay token key does not match daemon registration"
    );

    let (mut peer_connection, _) = relay.connect().await;
    let mut registration = victim_registration();
    registration.capabilities.clear();
    peer_connection
        .send(Message::Text(
            old_kernel_register(&registration, Some(1011)).into(),
        ))
        .await
        .expect("old registration sends");
    assert!(close_reason(&mut peer_connection)
        .await
        .starts_with("update needed: "));
    assert_eq!(listed_machines(&relay).await, Vec::new());
    relay.stop().await;
}
