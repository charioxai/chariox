//! MP-08/MP-10/MP-11: connector socket regressions, supplementary to live slice drills.
use super::support::*;
use chariox_relay::auth::{
    RelayAction, RelayAuthVerifier, RelaySubjectKind, RelayTokenClaims, ScopedTokenVerifier,
};

fn bound_claim(config: &DaemonConfig, token_id: &str) -> RelayTokenClaims {
    RelayTokenClaims {
        issuer: "activation-test".into(),
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
        expires_at_ms: crate::session::unix_epoch_ms() + 60_000,
        token_id: token_id.into(),
        account_id: None,
        organization_id: None,
        user_id: None,
        device_id: None,
        machine_id: Some(config.host_machine_id.clone()),
        client_id: None,
        session_id: None,
        public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
            &config.relay_public_key,
        )),
        entitlements_version: None,
    }
}

#[test]
fn same_socket_activation_confirmation_after_heartbeat_reauthentication() {
    run_async_with_large_test_stack("activation-heartbeat", || async {
        exercise_activation(100).await
    });
}

#[test]
fn same_socket_activation_confirmation_after_token_refresh_reauthentication() {
    run_async_with_large_test_stack("activation-refresh", || async {
        exercise_activation(20_000).await
    });
}

#[test]
fn same_socket_activation_confirmation_when_token_already_authenticated() {
    run_async_with_large_test_stack("activation-staging-race", || async {
        exercise_activation(0).await
    });
}

async fn exercise_activation(heartbeat_ms: u64) {
    let _guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    crate::logging::init_process_logger("tokenroll-activation-test").unwrap();
    let mut owner = DaemonConfig::for_tests();
    owner.daemon_id = "activation-owner".into();
    owner.host_machine_id = "activation-owner-machine".into();
    owner.relay_token = Some("owner".into());
    let mut worker = DaemonConfig::for_tests();
    worker.daemon_id = "activation-worker".into();
    worker.host_machine_id = "activation-worker-machine".into();
    worker.relay_token = Some("bootstrap".into());
    worker.relay_heartbeat_ms = heartbeat_ms.max(100);
    let server = RelayServer::with_auth_verifier(
        RelayConfig {
            host: "127.0.0.1".into(),
            port: 0,
            shared_token: None,
        },
        RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(
            BTreeMap::from([
                ("owner".into(), bound_claim(&owner, "owner")),
                ("bootstrap".into(), bound_claim(&worker, "bootstrap")),
                ("runtime".into(), bound_claim(&worker, "runtime")),
            ]),
            BTreeMap::new(),
            None,
        )),
    );
    let listener = server.bind_listener().await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let registry = server.registry();
    let (stop_relay, relay_stopped) = oneshot::channel();
    let relay_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = relay_stopped.await;
            })
            .await
            .unwrap();
    });
    owner.relay_url = Some(url.clone());
    worker.relay_url = Some(url.clone());
    let owner_router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(DaemonApp::bootstrap(owner.clone()).unwrap())),
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    let worker_router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::new(Mutex::new(DaemonApp::bootstrap(worker.clone()).unwrap())),
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    let owner_state = Arc::new(RwLock::new(RelayClientState::default()));
    let worker_state = Arc::new(RwLock::new(RelayClientState::default()));
    let (stop_owner, owner_stopped) = watch::channel(false);
    let (stop_worker, worker_stopped) = watch::channel(false);
    let owner_task = tokio::spawn(run_daemon_relay_connector_with_router(
        owner_router,
        owner_state.clone(),
        owner_stopped,
    ));
    let worker_task = tokio::spawn(run_daemon_relay_connector_with_router(
        worker_router.clone(),
        worker_state.clone(),
        worker_stopped,
    ));
    wait_for_daemon_registration(registry.clone(), &owner.daemon_id).await;
    wait_for_daemon_registration(registry.clone(), &worker.daemon_id).await;
    // Do not stage before connect: that only exercises the already-working hook.
    sleep(Duration::from_millis(150)).await;
    owner_state
        .write()
        .await
        .begin_managed_slice_relay_activation(
            "slice-1".into(),
            worker.daemon_id.clone(),
            worker.daemon_id.clone(),
            worker.relay_public_key.clone(),
            "activation-1".into(),
        );
    worker_router
        .runtime_state()
        .configure_relay(Some(url), Some("runtime".into()), false)
        .await
        .unwrap();
    if heartbeat_ms == 0 {
        // Model install awaiting persistence: the connector may authenticate the
        // new token before peer_requests stages its confirmation.
        for _ in 0..100 {
            if registry
                .read()
                .await
                .daemon(&worker.daemon_id)
                .is_some_and(|peer| peer.auth_token == "runtime")
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
        assert!(registry
            .read()
            .await
            .daemon(&worker.daemon_id)
            .is_some_and(|peer| peer.auth_token == "runtime"));
    }
    worker_state
        .write()
        .await
        .stage_managed_slice_activation_confirmation(
            super::super::connection_state::PendingManagedSliceActivationConfirmation::new(
                "slice-1".into(),
                owner.daemon_id.clone(),
                owner.relay_public_key.clone(),
                worker.daemon_id.clone(),
                "activation-1".into(),
            ),
        );
    let mut confirmed = false;
    let mut renewed = false;
    let mut disconnected = false;
    for _ in 0..350 {
        sleep(Duration::from_millis(20)).await;
        disconnected |= !worker_state.read().await.connected();
        renewed |= registry
            .read()
            .await
            .daemon(&worker.daemon_id)
            .is_some_and(|peer| peer.auth_token == "runtime");
        confirmed = owner_state
            .read()
            .await
            .managed_slice_relay_activation_confirmed("slice-1", "activation-1");
        if confirmed
            && renewed
            && worker_state
                .read()
                .await
                .pending_managed_slice_activation_confirmation()
                .is_none()
        {
            break;
        }
    }
    let pending = worker_state
        .read()
        .await
        .pending_managed_slice_activation_confirmation()
        .is_some();
    let worker_requests = worker_state.read().await.next_peer_request_id;
    let reconnects = worker_router
        .transport_health_store()
        .snapshot(0, 0, 0)
        .relay_reconnect_attempts;
    let _ = stop_worker.send(true);
    let _ = stop_owner.send(true);
    worker_task.await.unwrap();
    owner_task.await.unwrap();
    let _ = stop_relay.send(());
    relay_task.await.unwrap();
    assert!(
        renewed,
        "runtime credential must authenticate on the real socket"
    );
    assert!(!disconnected, "activation must not reconnect the worker");
    assert_eq!(reconnects, 0, "even a reconnect between samples must fail");
    assert!(
        confirmed,
        "same-socket registration must dispatch pending activation confirmation (requests={worker_requests}, pending={pending})"
    );
    assert!(!pending, "successful confirmation must settle worker state");
}
