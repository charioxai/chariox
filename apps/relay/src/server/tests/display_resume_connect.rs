//! MP-08/MP-10/MP-11: actual relay socket admission, supplementary to live drills.
use super::*;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;
const CAPABILITY: &str = "chariox-room-display-resume-v721";

struct Fixture {
    url: String,
    daemon: Socket,
    counter: Arc<AtomicU64>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}
async fn next(socket: &mut Socket) -> Option<serde_json::Value> {
    timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => return Some(serde_json::from_str(&text).unwrap()),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => return None,
            }
        }
    })
    .await
    .expect("MP-10: bounded relay control reply")
}
async fn fixture() -> Fixture {
    let mut tokens = BTreeMap::new();
    for (token, kind, realm, actions, targets) in [
        (
            "daemon",
            RelaySubjectKind::Kernel,
            "realm",
            vec![RelayAction::DaemonRegister, RelayAction::DaemonHeartbeat],
            None,
        ),
        (
            "full",
            RelaySubjectKind::Client,
            "realm",
            vec![RelayAction::ClientConnect, RelayAction::PacketRoute],
            Some(vec!["kernel"]),
        ),
        (
            "connect-only",
            RelaySubjectKind::Client,
            "realm",
            vec![RelayAction::ClientConnect],
            Some(vec!["kernel"]),
        ),
        (
            "foreign",
            RelaySubjectKind::Client,
            "other-realm",
            vec![RelayAction::ClientConnect, RelayAction::PacketRoute],
            Some(vec!["kernel"]),
        ),
        (
            "wrong-target",
            RelaySubjectKind::Client,
            "realm",
            vec![RelayAction::ClientConnect, RelayAction::PacketRoute],
            Some(vec!["other"]),
        ),
    ] {
        let mut claim = scoped_claim(
            token,
            if token == "daemon" {
                "kernel"
            } else {
                "viewer"
            },
            kind,
            realm,
            actions,
            targets,
        );
        claim.expires_at_ms = 100_000;
        tokens.insert(token.to_string(), claim);
    }
    let mut expired = tokens["full"].clone();
    expired.expires_at_ms = 49;
    tokens.insert("expired".into(), expired);
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".into(),
            port: 0,
            shared_token: None,
        },
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(tokens, BTreeMap::new(), Some(50))),
    );
    let listener = server.bind_listener().await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let registry = server.registry();
    let counter = Arc::clone(&server.relay_request_counter);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let (mut daemon, _) = connect_async(&url).await.unwrap();
    daemon
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration_with_token(
                    "kernel", "machine", "Linux", 1, "daemon",
                ),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    timeout(Duration::from_secs(3), async {
        while registry.read().await.daemons.is_empty() {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    Fixture {
        url,
        daemon,
        counter,
        stop: Some(stop),
    }
}
fn packet(token: &str, pin: &str) -> serde_json::Value {
    serde_json::json!({"kind":"client_connect_display_resume","auth_token":token,"target":{"daemon_id":"kernel","daemon_alias":null},
        "expected_daemon_public_key":pin,"registration":{"request_id":"initial-resume","subscription_id":"fresh-route",
        "session_id":"old-lease","attachment_id":"7","client_public_key":"opaque-viewer-key"}})
}
async fn client(f: &Fixture, combined: bool) -> Socket {
    let mut request = f.url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        if combined {
            format!("{CAPABILITY}, {}", crate::binary_event::CAPABILITY)
                .parse()
                .unwrap()
        } else {
            crate::binary_event::CAPABILITY.parse().unwrap()
        },
    );
    let (socket, reply) = connect_async(request).await.unwrap();
    assert_eq!(
        reply
            .headers()
            .get("sec-websocket-protocol")
            .unwrap()
            .to_str()
            .unwrap(),
        if combined {
            CAPABILITY
        } else {
            crate::binary_event::CAPABILITY
        },
        "MP-10: initial535 requires positive relay capability negotiation"
    );
    socket
}

#[tokio::test]
async fn mp10_combined_connect_routes_one_ordinary_535_subscription_without_a_client_round_trip() {
    let mut f = fixture().await;
    let mut client = client(&f, true).await;
    client
        .send(Message::Text(
            packet("full", "public-key-kernel").to_string().into(),
        ))
        .await
        .unwrap();
    assert_eq!(next(&mut client).await.unwrap()["kind"], "client_connected");
    let forwarded = next(&mut f.daemon).await.unwrap();
    assert_eq!(forwarded["kind"], "daemon_subscribe");
    assert_eq!(forwarded["session_id"], "old-lease");
    assert_eq!(forwarded["attachment_id"], "7");
    assert_eq!(
        forwarded["subscription_scope"],
        crate::frame_transport::DISPLAY_RESUME_SCOPE
    );
    assert!(forwarded["resume_from_event_id"].is_null());
    assert_eq!(f.counter.load(Ordering::Relaxed), 1);
    // A repeated combined connect is not a token-renewal or second resume path.
    client
        .send(Message::Text(
            packet("full", "public-key-kernel").to_string().into(),
        ))
        .await
        .unwrap();
    assert_eq!(next(&mut client).await.unwrap()["kind"], "close");
}

#[tokio::test]
async fn mp11_combined_connect_preserves_capability_pin_realm_target_action_and_expiry_refusals() {
    for (token, pin, combined) in [
        ("full", "wrong-pin", true),
        ("full", "public-key-kernel", false),
        ("connect-only", "public-key-kernel", true),
        ("foreign", "public-key-kernel", true),
        ("wrong-target", "public-key-kernel", true),
        ("expired", "public-key-kernel", true),
    ] {
        let f = fixture().await;
        let mut client = client(&f, combined).await;
        client
            .send(Message::Text(packet(token, pin).to_string().into()))
            .await
            .unwrap();
        let reply = next(&mut client).await;
        if token == "connect-only" {
            assert_eq!(reply.unwrap()["kind"], "client_connected");
            assert_eq!(
                next(&mut client).await.unwrap()["error"]["code"],
                "action_not_allowed"
            );
        } else {
            assert!(reply.is_none_or(|value| value["kind"] == "close"));
        }
        assert_eq!(
            f.counter.load(Ordering::Relaxed),
            0,
            "MP-11: refused initial registration must not reach a kernel"
        );
    }
}
