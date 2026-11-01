//! MP-08/MP-10/MP-11: real socket regression; not hosted/live acceptance.
use super::support::*;
use chariox_relay::auth::{
    RelayAction, RelayAuthVerifier, RelaySubjectKind, RelayTokenClaims, ScopedTokenVerifier,
};

fn claim(config: &DaemonConfig, token: &str, expires_at_ms: u64) -> RelayTokenClaims {
    RelayTokenClaims {
        issuer: "tokenroll-test".into(),
        subject: config.daemon_id.clone(),
        subject_kind: RelaySubjectKind::Kernel,
        realm_id: "default".into(),
        allowed_actions: vec![
            RelayAction::DaemonRegister,
            RelayAction::DaemonHeartbeat,
            RelayAction::ClientMetadataRead,
            RelayAction::PacketRoute,
            RelayAction::PeerRequest,
        ],
        allowed_targets: None,
        issued_at_ms: crate::session::unix_epoch_ms(),
        expires_at_ms,
        token_id: token.into(),
        account_id: None,
        organization_id: None,
        user_id: None,
        device_id: None,
        machine_id: Some(config.host_machine_id.clone()),
        client_id: None,
        session_id: None,
        public_key_thumbprint: None,
        entitlements_version: None,
    }
}

#[test]
fn token_rollover_preserves_waiting_room_during_large_peer_request() {
    run_async_with_large_test_stack("token-rollover", || async {
        let _guard = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let mut target_config = DaemonConfig::for_tests();
        target_config.relay_token = Some("old".into());
        // The first tick is immediate. Leave a deterministic window for the
        // post-peer check before the next heartbeat authenticates the new token.
        target_config.relay_heartbeat_ms = 10_000;
        let mut source_config = DaemonConfig::for_tests();
        source_config.daemon_id = "tokenroll-source".into();
        source_config.host_machine_id = "tokenroll-source-machine".into();
        source_config.relay_token = Some("source".into());
        let now = crate::session::unix_epoch_ms();
        let mut client_claim = claim(&target_config, "client", now + 60_000);
        client_claim.subject_kind = RelaySubjectKind::Client;
        client_claim.subject = "terminal".into();
        client_claim.machine_id = None;
        client_claim.allowed_actions = vec![RelayAction::ClientConnect, RelayAction::PacketRoute];
        client_claim.allowed_targets = Some(vec![target_config.daemon_id.clone()]);
        let server = RelayServer::with_auth_verifier(
            RelayConfig {
                host: "127.0.0.1".into(),
                port: 0,
                shared_token: None,
            },
            RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(
                BTreeMap::from([
                    ("old".into(), claim(&target_config, "old", now + 15_000)),
                    ("fresh".into(), claim(&target_config, "fresh", now + 60_000)),
                    (
                        "source".into(),
                        claim(&source_config, "source", now + 60_000),
                    ),
                    ("client".into(), client_claim),
                ]),
                BTreeMap::new(),
                None,
            )),
        );
        let listener = server.bind_listener().await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
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
        target_config.relay_url = Some(url.clone());
        source_config.relay_url = Some(url.clone());
        let target_app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(target_config.clone()).unwrap(),
        ));
        let source_app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(source_config.clone()).unwrap(),
        ));
        let target_router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
            target_app,
            INTERACTIVE_COMMAND_QUEUE_LIMIT,
        ));
        let target_state = Arc::new(RwLock::new(RelayClientState::default()));
        let source_state = Arc::new(RwLock::new(RelayClientState::default()));
        let (stop_target, target_stopped) = watch::channel(false);
        let (stop_source, source_stopped) = watch::channel(false);
        let target_task = tokio::spawn(run_daemon_relay_connector_with_router(
            target_router.clone(),
            target_state.clone(),
            target_stopped,
        ));
        let source_task = tokio::spawn(run_daemon_relay_connector(
            source_app.clone(),
            source_state.clone(),
            source_stopped,
        ));
        wait_for_daemon_registration(registry.clone(), &target_config.daemon_id).await;
        wait_for_daemon_registration(registry.clone(), &source_config.daemon_id).await;
        let (mut client, _) = connect_async(&url).await.unwrap();
        let target = ClientTarget {
            daemon_id: Some(target_config.daemon_id.clone()),
            daemon_alias: None,
        };
        send_client_envelope(
            &mut client,
            &RelayEnvelope::ClientConnect {
                auth_token: "client".into(),
                target: target.clone(),
            },
        )
        .await;
        expect_client_connected(&mut client).await;
        let key = relay_crypto::generate_private_key_base64();
        let public_key = relay_crypto::public_key_from_private_key_base64(&key).unwrap();
        approve_test_terminal(&target_config, &public_key);
        send_client_envelope(
            &mut client,
            &RelayEnvelope::ClientSubscribe {
                request_id: "subscribe".into(),
                subscription_id: "waiting-room".into(),
                target: target.clone(),
                session_id: WAITING_ROOM_INVENTORY_SENTINEL_ID.into(),
                attachment_id: WAITING_ROOM_INVENTORY_SENTINEL_ID.into(),
                client_public_key: public_key,
                subscription_scope: Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE.into()),
                resume_from_event_id: None,
                heartbeat_interval_ms: None,
            },
        )
        .await;
        expect_json_client_response(&mut client, "subscribe", &key).await;
        expect_named_client_event(&mut client, &key, "waiting_room_rows_changed").await;

        // Renew through the production config path, then deliver an encrypted
        // multi-MB peer request. Its response exercises the offending continuity
        // check with a captured old token; the response itself must remain intact.
        target_router
            .runtime_state()
            .configure_relay(Some(url), Some("fresh".into()), false)
            .await
            .unwrap();
        let payload = "token-rollover-payload\n".repeat(100_000);
        let peer_result = send_peer_request_via_relay(
            &source_app,
            &source_state,
            target.clone(),
            RelayPeerRequest::Ping {
                value: payload.clone(),
            },
        )
        .await;
        let mut lost_route = false;
        for _ in 0..40 {
            sleep(Duration::from_millis(10)).await;
            lost_route |= !target_state.read().await.connected()
                || registry.read().await.subscription_count() != 1;
        }
        let mut renewed = false;
        if !lost_route {
            for _ in 0..1200 {
                sleep(Duration::from_millis(10)).await;
                let guard = registry.read().await;
                lost_route |= guard.subscription_count() != 1;
                renewed = guard
                    .daemon(&target_config.daemon_id)
                    .is_some_and(|d| d.auth_token == "fresh");
                if renewed || lost_route {
                    break;
                }
            }
        }
        let mut event_survived = false;
        if !lost_route && renewed {
            let worktree = crate::test_support::TestWorktree::new("rollover-inventory");
            let request = LocalDaemonRequest::CreateSession(worktree.session_request());
            let command =
                KernelCommand::from_local_request("rollover-session", None, None, &request);
            target_router.dispatch(command, request).await.unwrap();
            event_survived = timeout(
                Duration::from_secs(2),
                expect_named_client_event(&mut client, &key, "waiting_room_rows_changed"),
            )
            .await
            .is_ok();
        }
        // Settle all tasks even on RED before asserting the result.
        let _ = client.close(None).await;
        let _ = stop_target.send(true);
        let _ = stop_source.send(true);
        target_task.await.unwrap();
        source_task.await.unwrap();
        let _ = stop_server.send(());
        relay_task.await.unwrap();
        assert!(
            matches!(peer_result, Ok(RelayPeerResponse::Pong { value, .. }) if value == payload),
            "multi-MB peer response must arrive intact"
        );
        assert!(
            !lost_route,
            "token renewal dropped the live waiting-room subscription"
        );
        assert!(
            renewed,
            "relay must verify the renewed token on the existing socket"
        );
        assert!(
            event_survived,
            "same subscription must deliver inventory changes after renewal"
        );
    });
}
