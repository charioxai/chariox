use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn client_connect_renewal_preserves_routes_and_refuses_permission_or_identity_changes() {
    let old = scoped_claim(
        "old",
        "terminal",
        RelaySubjectKind::Client,
        "realm",
        vec![RelayAction::ClientConnect, RelayAction::PacketRoute],
        Some(vec!["daemon"]),
    );
    let mut fresh = old.clone();
    fresh.token_id = "fresh".into();
    fresh.expires_at_ms = 200_000;
    let mut reduced = fresh.clone();
    reduced.allowed_actions = vec![RelayAction::ClientConnect];
    let mut foreign = fresh.clone();
    foreign.subject = "foreign".into();
    let mut old = old;
    old.expires_at_ms = 100_000;
    let mut daemon_claim = scoped_claim(
        "daemon",
        "daemon",
        RelaySubjectKind::Kernel,
        "realm",
        vec![RelayAction::DaemonRegister, RelayAction::DaemonHeartbeat],
        None,
    );
    daemon_claim.expires_at_ms = 100_000;
    let verifier = RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(
        BTreeMap::from([
            ("old".into(), old),
            ("fresh".into(), fresh),
            ("reduced".into(), reduced),
            ("foreign".into(), foreign),
            ("daemon".into(), daemon_claim),
        ]),
        BTreeMap::new(),
        Some(50),
    ));
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".into(),
            port: 0,
            shared_token: None,
        },
        verifier,
    );
    let listener = server.bind_listener().await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let run = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = stopped.await;
            })
            .await
            .unwrap()
    });
    let url = format!("ws://{addr}");
    let (mut daemon, _) = connect_async_with_retry(&url).await.unwrap();
    daemon
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonRegister {
                registration: test_registration_with_token(
                    "daemon", "machine", "Linux", 1, "daemon",
                ),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let _ = daemon.next().await.unwrap().unwrap();
    for invalid in ["reduced", "foreign"] {
        let (mut client, _) = connect_async_with_retry(&url).await.unwrap();
        for token in ["old", "fresh", invalid] {
            client
                .send(Message::Text(
                    serde_json::to_string(&RelayEnvelope::ClientConnect {
                        auth_token: token.into(),
                        target: ClientTarget {
                            daemon_id: Some("daemon".into()),
                            daemon_alias: None,
                        },
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            let reply = timeout(Duration::from_secs(2), client.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let Message::Text(text) = reply else {
                panic!("expected relay reply")
            };
            let value: RelayEnvelope = serde_json::from_str(&text).unwrap();
            if token == invalid {
                assert!(
                    matches!(value, RelayEnvelope::Close { .. }),
                    "renewal must refuse changed identity or retained-route permissions"
                )
            } else {
                assert!(matches!(value, RelayEnvelope::ClientConnected { .. }))
            }
        }
        let _ = client.close(None).await;
    }
    let _ = daemon.close(None).await;
    let _ = stop.send(());
    run.await.unwrap();
}
