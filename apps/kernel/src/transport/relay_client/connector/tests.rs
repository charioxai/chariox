use super::*;

#[test]
fn missing_relay_config_without_cloud_profile_is_local_idle() {
    let config = crate::config::DaemonConfig::for_tests();

    assert!(!should_refresh_cloud_relay(&config));
    assert_eq!(
        missing_relay_config_disposition(&config),
        MissingRelayConfigDisposition::LocalIdle
    );
}

#[test]
fn missing_relay_config_with_cloud_profile_is_cloud_unavailable() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        kernel_id: None,
        kernel_credential: None,
        kernel_public_key_thumbprint: None,
        api_url: "https://cloud.example.test".to_string(),
        email: "user@example.test".to_string(),
        account_id: "account-1".to_string(),
        user_id: "user-1".to_string(),
        account_slug: "account".to_string(),
        realm_id: "realm-1".to_string(),
        relay_url: "wss://relay.example.test".to_string(),
        issuer_id: "issuer-1".to_string(),
        client_id: None,
        client_alias: None,
        machine_id: None,
        machine_alias: None,
        machine_credential: Some("machine-credential".to_string()),
        cloud_session_token: None,
        cloud_session_expires_at_ms: None,
        token_expires_at_ms: None,
    });

    assert!(should_refresh_cloud_relay(&config));
    assert_eq!(
        missing_relay_config_disposition(&config),
        MissingRelayConfigDisposition::CloudUnavailable
    );
}

#[test]
fn managed_slice_worker_does_not_request_remote_inventory_with_daemon_token() {
    assert!(should_refresh_remote_inventory(true, false));
    assert!(!should_refresh_remote_inventory(true, true));
    assert!(!should_refresh_remote_inventory(false, false));
}

#[tokio::test]
async fn dynamic_relay_token_rotation_returns_one_registration_heartbeat() {
    let relay_url = "wss://relay.example.test";
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(relay_url.to_string());
    config.relay_token = Some("new-token".to_string());
    let mut app = DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap");
    let registration = app.relay_registration();
    let mut active_token = "old-token".to_string();

    let heartbeat_registration =
        dynamic_relay_heartbeat_registration(relay_url, &mut active_token, &config, || {
            std::future::ready(registration.clone())
        })
        .await
        .expect("rotation should preserve the socket")
        .expect("rotation should return an authenticated heartbeat registration");

    assert_eq!(active_token, "new-token");
    assert_eq!(heartbeat_registration.auth_token, "new-token");

    let steady_state =
        dynamic_relay_heartbeat_registration(relay_url, &mut active_token, &config, || async {
            panic!("steady state must not rebuild registration")
        })
        .await
        .expect("steady state should preserve the socket");

    assert!(steady_state.is_none());
    assert_eq!(active_token, "new-token");
}

#[tokio::test]
async fn token_rollover_during_registration_authenticates_snapshot_without_disconnect() {
    // MP-08/MP-10/MP-11: refreshing the config between the snapshot and
    // registration must not turn a harmless renewal race into a reconnect.
    let url = "wss://relay.example.test";
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(url.into());
    config.relay_token = Some("first-renewal".into());
    let mut next_config = config.clone();
    next_config.relay_token = Some("second-renewal".into());
    let mut app = DaemonApp::bootstrap(next_config.clone()).unwrap();
    let registration = app.relay_registration();
    let mut active = "old".to_string();
    let result = dynamic_relay_heartbeat_registration(url, &mut active, &config, || {
        std::future::ready(registration.clone())
    })
    .await;
    assert!(
        matches!(result, Ok(Some(ref r)) if r.auth_token == "first-renewal"),
        "renewal race must authenticate the snapshot on the same socket"
    );
    assert_eq!(
        active, "first-renewal",
        "track the registration actually queued"
    );
    let result = dynamic_relay_heartbeat_registration(url, &mut active, &next_config, || {
        std::future::ready(registration)
    })
    .await;
    assert!(matches!(result, Ok(Some(_))));
    assert_eq!(active, "second-renewal");
}

#[tokio::test]
async fn cloud_token_refresh_gate_skips_running_task() {
    let (_tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = rx.await;
    });

    assert!(!should_start_cloud_token_refresh(Some(&task)));

    task.abort();
}

#[tokio::test]
async fn cloud_token_refresh_gate_allows_finished_or_missing_task() {
    let task = tokio::spawn(async {});
    while !task.is_finished() {
        tokio::task::yield_now().await;
    }

    assert!(should_start_cloud_token_refresh(None));
    assert!(should_start_cloud_token_refresh(Some(&task)));
}

#[tokio::test]
async fn leased_projection_pump_gate_skips_running_task() {
    let (_tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = rx.await;
    });

    assert!(!should_start_leased_projection_pump(Some(&task)));

    task.abort();
}

#[tokio::test]
async fn leased_projection_pump_gate_allows_finished_or_missing_task() {
    let task = tokio::spawn(async {});
    while !task.is_finished() {
        tokio::task::yield_now().await;
    }

    assert!(should_start_leased_projection_pump(None));
    assert!(should_start_leased_projection_pump(Some(&task)));
}

#[tokio::test]
async fn leased_projection_pump_does_not_cancel_slow_work() {
    let (completed_tx, completed_rx) = oneshot::channel();

    await_leased_projection_pump(
        async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = completed_tx.send(());
        },
        Duration::from_millis(1),
    )
    .await;

    assert!(completed_rx.await.is_ok());
}

#[test]
fn cloud_presence_refresh_interval_is_stable_and_bounded() {
    let first = cloud_presence_refresh_interval("daemon-a");
    let second = cloud_presence_refresh_interval("daemon-a");

    assert_eq!(first, second);
    assert!(first >= CLOUD_RELAY_PRESENCE_REFRESH_INTERVAL);
    assert!(
        first
            < CLOUD_RELAY_PRESENCE_REFRESH_INTERVAL
                + Duration::from_millis(CLOUD_RELAY_PRESENCE_JITTER_SPREAD_MS)
    );
}

#[test]
fn cloud_presence_schedule_claims_due_idle_publish_once_per_interval() {
    let start = Instant::now();
    let mut schedule =
        CloudPresencePublishSchedule::new_with_interval(start, Duration::from_millis(100));

    assert!(!schedule.claim_publish_if_due(start + Duration::from_millis(99)));
    assert!(schedule.claim_publish_if_due(start + Duration::from_millis(100)));
    assert!(!schedule.claim_publish_if_due(start + Duration::from_millis(101)));
    assert!(schedule.claim_publish_if_due(start + Duration::from_millis(200)));
}

#[tokio::test]
async fn failed_inventory_refresh_retries_on_the_next_heartbeat() {
    let mut task = Some(tokio::spawn(async { false }));
    while !task.as_ref().unwrap().is_finished() {
        tokio::task::yield_now().await;
    }
    assert!(remote_inventory_refresh_due(1, &mut task));
    assert!(task.is_none());

    let mut task = Some(tokio::spawn(async { true }));
    while !task.as_ref().unwrap().is_finished() {
        tokio::task::yield_now().await;
    }
    assert!(!remote_inventory_refresh_due(1, &mut task));
    assert!(!remote_inventory_refresh_due(2, &mut task));
    assert!(remote_inventory_refresh_due(
        RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS,
        &mut task
    ));

    let (_tx, rx) = oneshot::channel::<()>();
    let mut running = Some(tokio::spawn(async move { rx.await.is_ok() }));
    assert!(!remote_inventory_refresh_due(
        RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS,
        &mut running
    ));
    running.unwrap().abort();
}

#[test]
fn relay_reconnect_delay_is_stable_jittered_and_bounded() {
    let first = relay_reconnect_delay("daemon-a", 0);
    let second = relay_reconnect_delay("daemon-a", 0);

    assert_eq!(first, second);
    assert!(first >= Duration::from_millis(RELAY_RECONNECT_BASE_DELAY_MS));
    assert!(
        first
            < Duration::from_millis(
                RELAY_RECONNECT_BASE_DELAY_MS + RELAY_RECONNECT_JITTER_SPREAD_MS,
            )
    );

    let capped = relay_reconnect_delay("daemon-a", 99);
    assert!(capped >= Duration::from_millis(RELAY_RECONNECT_MAX_DELAY_MS));
    assert!(
        capped
            < Duration::from_millis(
                RELAY_RECONNECT_MAX_DELAY_MS + RELAY_RECONNECT_JITTER_SPREAD_MS,
            )
    );
}

#[test]
fn relay_reconnect_after_a_long_outage_waits_at_most_two_seconds() {
    // A person's link that comes back after seconds must not wait out a long backoff (V1-INT-15).
    for daemon in ["daemon-a", "daemon-b", "0123456789abcdef"] {
        for attempt in [4, 20, 99] {
            assert!(relay_reconnect_delay(daemon, attempt) <= Duration::from_secs(2));
        }
    }
}

#[tokio::test]
async fn reconnect_delay_returns_false_after_delay() {
    let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);

    assert!(!wait_for_reconnect_delay(&mut shutdown_rx, Duration::from_millis(1)).await);
}

#[tokio::test]
async fn reconnect_delay_exits_on_shutdown() {
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let wait = tokio::spawn(async move {
        wait_for_reconnect_delay(&mut shutdown_rx, Duration::from_secs(30)).await
    });

    shutdown_tx.send(true).expect("shutdown should send");

    assert!(timeout(Duration::from_secs(1), wait)
        .await
        .expect("wait should observe shutdown")
        .expect("wait task should finish"));
}

#[test]
fn stable_jitter_ms_honors_zero_spread() {
    assert_eq!(stable_jitter_ms("daemon-a", 0), 0);
}

#[tokio::test]
async fn bounded_cloud_relay_refresh_reports_success() {
    let result =
        bounded_cloud_relay_refresh(async { Ok::<(), &'static str>(()) }, Duration::from_secs(1))
            .await;

    assert_eq!(result, CloudRelayRefreshResult::Refreshed);
}

#[tokio::test]
async fn bounded_cloud_relay_refresh_reports_failure() {
    let result = bounded_cloud_relay_refresh(
        async { Err::<(), &'static str>("cloud unavailable") },
        Duration::from_secs(1),
    )
    .await;

    assert_eq!(
        result,
        CloudRelayRefreshResult::Failed("cloud unavailable".to_string())
    );
}

#[tokio::test]
async fn bounded_cloud_relay_refresh_times_out() {
    let result = bounded_cloud_relay_refresh(
        std::future::pending::<Result<(), &'static str>>(),
        Duration::from_millis(1),
    )
    .await;

    assert_eq!(result, CloudRelayRefreshResult::TimedOut);
}

#[tokio::test]
async fn post_connect_hook_confirms_through_owner_handler_and_clears_worker_pending() {
    let (owner_confirmed, worker_pending, attempts) =
        exercise_post_connect_confirmation(true, false, false).await;
    assert!(owner_confirmed);
    assert!(!worker_pending);
    assert_eq!(attempts, 1);
}

#[tokio::test]
async fn post_connect_hook_clears_worker_pending_after_terminal_owner_rejection() {
    let (owner_confirmed, worker_pending, attempts) =
        exercise_post_connect_confirmation(false, false, false).await;
    assert!(!owner_confirmed);
    assert!(!worker_pending);
    assert_eq!(attempts, 1);
}

#[tokio::test]
async fn post_connect_hook_retries_retryable_rejection_on_same_connection() {
    let (owner_confirmed, worker_pending, attempts) =
        exercise_post_connect_confirmation(true, true, false).await;
    assert!(owner_confirmed);
    assert!(!worker_pending);
    assert_eq!(attempts, 2);
}

#[tokio::test]
async fn mp08_mp10_mp11_retained_socket_confirms_install_after_registration_heartbeat() {
    let (owner_confirmed, worker_pending, attempts) =
        exercise_post_connect_confirmation(true, false, true).await;
    assert!(owner_confirmed);
    assert!(!worker_pending);
    assert_eq!(attempts, 1);
}

async fn exercise_post_connect_confirmation(
    owner_accepts_worker_key: bool,
    reject_first_attempt_as_retryable: bool,
    retained_socket: bool,
) -> (bool, bool, usize) {
    let mut worker_config = crate::config::DaemonConfig::for_tests();
    worker_config.relay_url = Some("wss://relay.example.test".to_string());
    worker_config.relay_token = Some("runtime-token".to_string());
    let worker_public_key = worker_config.relay_public_key.clone();
    let worker_kernel_id = worker_config.daemon_id.clone();
    let worker_app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(worker_config).expect("worker daemon should bootstrap"),
    ));
    let worker_router = Arc::new(CommandRouter::with_interactive_capacity(worker_app, 1));
    let worker_state = Arc::new(RwLock::new(RelayClientState::default()));
    let (worker_outgoing, mut worker_priority_rx, _worker_event_rx) =
        RelayOutgoingSender::channel(4);

    let mut owner_config = crate::config::DaemonConfig::for_tests();
    owner_config.daemon_id = "owner-1".to_string();
    let owner_public_key = owner_config.relay_public_key.clone();
    let owner_private_key = owner_config.relay_private_key.clone();
    let owner_app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(owner_config).expect("owner daemon should bootstrap"),
    ));
    let owner_router = Arc::new(CommandRouter::with_interactive_capacity(owner_app, 1));
    let owner_state = Arc::new(RwLock::new(RelayClientState::default()));
    owner_state
        .write()
        .await
        .begin_managed_slice_relay_activation(
            "slice-1".to_string(),
            worker_kernel_id.clone(),
            "slice:dev".to_string(),
            if owner_accepts_worker_key {
                worker_public_key.clone()
            } else {
                "replacement-key".to_string()
            },
            "activation-1".to_string(),
        );
    let (owner_outgoing, _owner_priority_rx, _owner_event_rx) = RelayOutgoingSender::channel(1);

    set_connected(
        &worker_state,
        worker_outgoing.clone(),
        "wss://relay.example.test".to_string(),
    )
    .await
    .expect("worker relay should become connected");
    if retained_socket {
        assert!(
            spawn_pending_managed_slice_activation_confirmation_after_connect(
                Arc::clone(&worker_router),
                Arc::clone(&worker_state),
            )
            .await
            .is_none(),
            "the initial connection predates token installation"
        );
    }
    worker_state
        .write()
        .await
        .stage_managed_slice_activation_confirmation(
            super::super::connection_state::PendingManagedSliceActivationConfirmation::new(
                "slice-1".to_string(),
                "owner-1".to_string(),
                owner_public_key,
                worker_kernel_id.clone(),
                "activation-1".to_string(),
            ),
        );
    if retained_socket {
        send_outgoing_envelope(
            &worker_outgoing,
            RelayEnvelope::DaemonHeartbeat {
                daemon_id: worker_kernel_id.clone(),
                registration: Some(worker_router.relay_registration().await),
            },
        )
        .expect("queue renewed registration before confirmation");
        assert!(matches!(
            worker_priority_rx.recv().await,
            Some(RelayEnvelope::DaemonHeartbeat {
                registration: Some(_),
                ..
            })
        ));
    }
    let task = spawn_pending_managed_slice_activation_confirmation_after_connect(
        Arc::clone(&worker_router),
        Arc::clone(&worker_state),
    )
    .await
    .expect("post-connect hook should claim pending confirmation");

    let expected_attempts = if reject_first_attempt_as_retryable {
        2
    } else {
        1
    };
    for attempt in 0..expected_attempts {
        let envelope = timeout(Duration::from_secs(1), worker_priority_rx.recv())
            .await
            .expect("confirmation should be queued")
            .expect("confirmation channel should stay open");
        let RelayEnvelope::DaemonPeerRequest {
            request_id,
            target,
            encrypted_request,
        } = envelope
        else {
            panic!("expected a peer confirmation request")
        };
        assert_eq!(target.daemon_id.as_deref(), Some("owner-1"));
        let encrypted_response = if reject_first_attempt_as_retryable && attempt == 0 {
            let response =
                crate::transport::relay_peer::RelayPeerResponse::ManagedSliceRelayTokenFailed {
                    code: "owner_temporarily_unavailable".to_string(),
                    retryable: true,
                };
            relay_crypto::encrypt_payload_for_peer(
                &owner_private_key,
                &worker_public_key,
                &serde_json::to_vec(&response).expect("retryable response should encode"),
            )
            .expect("retryable response should encrypt")
        } else {
            let caller_identity = chariox_relay::protocol::RelayCallerIdentity {
                realm_id: "realm-1".to_string(),
                subject: "slice:dev".to_string(),
                subject_kind: chariox_relay::auth::RelaySubjectKind::Kernel,
                expires_at_ms: u64::MAX,
                token_id: Some("runtime-token-1".to_string()),
                user_id: Some("user-1".to_string()),
                public_key_thumbprint: Some(
                    crate::runtime::terminal_pairings::public_key_thumbprint(&worker_public_key),
                ),
            };
            let outcome = handle_daemon_peer_request(
                &owner_router,
                &owner_state,
                &owner_outgoing,
                &worker_kernel_id,
                Some(caller_identity),
                encrypted_request,
            )
            .await;
            assert!(outcome.error.is_none());
            outcome
                .encrypted_response
                .expect("owner should return an encrypted activation result")
        };
        resolve_pending_peer_response_for_test(
            &worker_state,
            request_id,
            "owner-1".to_string(),
            encrypted_response,
        )
        .await;
    }

    timeout(Duration::from_secs(1), task)
        .await
        .expect("confirmation task should finish")
        .expect("confirmation task should not panic");
    let owner_confirmed = owner_state
        .read()
        .await
        .managed_slice_relay_activation_confirmed("slice-1", "activation-1");
    let worker_pending = worker_state
        .read()
        .await
        .pending_managed_slice_activation_confirmation()
        .is_some();
    (owner_confirmed, worker_pending, expected_attempts)
}

#[test]
fn mp08_mp10_mp11_browser_artifact_peer_current_hosted_confirmation_gate() {
    let home = crate::config::DaemonConfig::for_tests();
    let worker = crate::config::DaemonConfig::for_tests();
    let minimum = crate::transport::relay_peer::MINIMUM_RELAY_PEER_RUNTIME_VERSION;
    for version in [
        minimum - 1,
        minimum,
        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
    ] {
        for (slice_id, nonce) in [
            ("slice-1", "nonce-1"),
            ("other", "nonce-1"),
            ("slice-1", "other"),
        ] {
            let response =
                crate::transport::relay_peer::RelayPeerResponse::ManagedSliceRelayTokenActivated {
                    slice_id: slice_id.into(),
                    activation_nonce: nonce.into(),
                    relay_peer_protocol_version: version,
                };
            let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(
                &home.relay_private_key,
                &worker.relay_public_key,
                &serde_json::to_vec(&response).unwrap(),
            )
            .unwrap();
            let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
                &worker.relay_private_key,
                &encrypted,
            )
            .unwrap();
            assert_eq!(decrypted.sender_public_key, home.relay_public_key);
            let crate::transport::relay_peer::RelayPeerResponse::ManagedSliceRelayTokenActivated {
                slice_id,
                activation_nonce,
                relay_peer_protocol_version,
            } = serde_json::from_slice(&decrypted.plaintext).unwrap()
            else {
                panic!("activation receipt");
            };
            assert_eq!(
                managed_slice_token_confirmation_matches(
                    &slice_id,
                    &activation_nonce,
                    relay_peer_protocol_version,
                    "slice-1",
                    "nonce-1",
                ),
                version >= minimum && slice_id == "slice-1" && activation_nonce == "nonce-1"
            );
        }
    }
}

// MP-08 / MP-10 / MP-11: a burst of inbound transfer chunks may briefly outrun the
// writer. The reader waits for it; it never drops a socket carrying a transfer.
#[tokio::test]
async fn transport_receipts_wait_for_a_busy_writer_instead_of_dropping_the_socket() {
    let (tx, mut rx) = mpsc::channel::<String>(8);
    for index in 0..8 {
        assert!(queue_transport_receipt(&tx, index.to_string()).await);
    }
    let drain = tokio::spawn(async move {
        sleep(Duration::from_millis(50)).await;
        let mut received = Vec::new();
        while received.len() < 9 {
            received.push(rx.recv().await.unwrap());
        }
        received
    });
    assert!(queue_transport_receipt(&tx, "8".into()).await);
    assert_eq!(
        drain.await.unwrap(),
        (0..9).map(|index| index.to_string()).collect::<Vec<_>>()
    );
}

#[tokio::test(start_paused = true)]
async fn transport_receipts_give_up_only_when_the_writer_is_stuck() {
    let (tx, _rx) = mpsc::channel::<String>(8);
    for index in 0..8 {
        assert!(queue_transport_receipt(&tx, index.to_string()).await);
    }
    assert!(!queue_transport_receipt(&tx, "stuck".into()).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn mp08_mp10_mp11_in_place_activation_starts_confirmation_without_socket_reconnect() {
    use chariox_relay::{RelayConfig, RelayServer};
    let server = Arc::new(RelayServer::new(RelayConfig {
        host: "127.0.0.1".into(),
        port: 0,
        shared_token: Some("activation-fixture".into()),
    }));
    let listener = server.bind_listener().await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (server_stop, server_rx) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = server_rx.await;
            })
            .await
            .unwrap();
    });
    let mut config = crate::config::DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://{addr}"));
    config.relay_token = Some("activation-fixture".into());
    config.relay_heartbeat_ms = 50;
    let worker_id = config.daemon_id.clone();
    let app = Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap()));
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (stop, stop_rx) = watch::channel(false);
    let connector = tokio::spawn(run_daemon_relay_connector(app, Arc::clone(&state), stop_rx));
    timeout(Duration::from_secs(2), async {
        while !state.read().await.connected() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // Installation occurs on the already connected socket. No reconnect hook
    // follows same-identity token renewal. The absent owner makes the real
    // relay return a bounded routing refusal; starting the confirmation is
    // the seam under test, not successful activation or provider acceptance.
    let owner = crate::config::DaemonConfig::for_tests();
    state
        .write()
        .await
        .stage_managed_slice_activation_confirmation(
            super::super::connection_state::PendingManagedSliceActivationConfirmation::new(
                "slice-in-place".into(),
                owner.daemon_id,
                owner.relay_public_key,
                worker_id,
                "nonce-in-place".into(),
            ),
        );
    let attempted = timeout(Duration::from_secs(3), async {
        loop {
            // A claim consumes an attempt; after the bounded attempt cap the
            // next claim returns None. Observe through the existing state API.
            let pending = state
                .read()
                .await
                .pending_managed_slice_activation_confirmation();
            if pending.is_none() {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let _ = stop.send(true);
    connector.await.unwrap();
    let _ = server_stop.send(());
    server_task.await.unwrap();
    assert!(
        attempted.is_ok(),
        "pending activation was stranded on the healthy socket"
    );
}

#[test]
fn mp08_mp10_mp11_activation_confirms_only_on_a_socket_with_the_current_token() {
    use super::managed_slice_activation::socket_has_current_token;
    assert!(socket_has_current_token(Some("renewed"), "renewed"));
    assert!(!socket_has_current_token(Some("renewed"), "previous"));
    assert!(!socket_has_current_token(None, "previous"));
}

// MP-08 / MP-10 / MP-11: one ready event must not wait for a render-frame timer.
#[tokio::test]
async fn relay_event_writer_delivers_ready_events_without_a_frame_wait() {
    use std::task::Context;
    let (priority_tx, priority_rx) = mpsc::channel(2);
    let (event_tx, event_rx) = mpsc::channel(2);
    let (pong_tx, pong_rx) = mpsc::channel(2);
    let (receipt_tx, receipt_rx) = mpsc::channel(2);
    let (sent_tx, mut sent_rx) = mpsc::unbounded_channel();
    let sink = Box::pin(futures_util::sink::unfold(
        sent_tx,
        |tx, message| async move {
            tx.send(message).unwrap();
            Ok::<_, std::convert::Infallible>(tx)
        },
    ));
    event_tx
        .try_send(RelayEnvelope::Close {
            reason: "accepted-turn-event".into(),
        })
        .unwrap();
    let mut writing = Box::pin(super::writer::run_relay_writer(
        sink,
        priority_rx,
        event_rx,
        pong_rx,
        receipt_rx,
        false,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ));
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);
    assert!(writing.as_mut().poll(&mut context).is_pending());
    assert!(
        sent_rx.try_recv().is_ok(),
        "ready event is still held behind the frame coalescer"
    );
    drop((priority_tx, event_tx, pong_tx, receipt_tx));
}

// MP-08 / MP-10 / MP-11: bounded lanes, control priority, and subscription FIFO survive the timer removal.
#[tokio::test]
async fn relay_event_writer_preserves_controls_bounded_events_and_fifo() {
    use std::task::Context;
    let (priority_tx, priority_rx) = mpsc::channel(2);
    let (event_tx, event_rx) = mpsc::channel(2);
    let (pong_tx, pong_rx) = mpsc::channel(2);
    let (receipt_tx, receipt_rx) = mpsc::channel(2);
    let (sent_tx, mut sent_rx) = mpsc::unbounded_channel();
    let sink = Box::pin(futures_util::sink::unfold(
        sent_tx,
        |tx, message| async move {
            tx.send(message).unwrap();
            Ok::<_, std::convert::Infallible>(tx)
        },
    ));
    for reason in ["event-1", "event-2"] {
        event_tx
            .try_send(RelayEnvelope::Close {
                reason: reason.into(),
            })
            .unwrap();
    }
    assert!(matches!(
        event_tx.try_send(RelayEnvelope::Close {
            reason: "over-capacity".into()
        }),
        Err(mpsc::error::TrySendError::Full(_))
    ));
    priority_tx
        .try_send(RelayEnvelope::Close {
            reason: "priority".into(),
        })
        .unwrap();
    pong_tx.try_send(vec![1]).unwrap();
    receipt_tx.try_send("public-receipt".into()).unwrap();
    let mut writing = Box::pin(super::writer::run_relay_writer(
        sink,
        priority_rx,
        event_rx,
        pong_rx,
        receipt_rx,
        false,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ));
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);
    assert!(writing.as_mut().poll(&mut context).is_pending());
    let sent: Vec<_> = std::iter::from_fn(|| sent_rx.try_recv().ok()).collect();
    assert_eq!(sent.len(), 5);
    assert_eq!(sent[0].to_text().unwrap(), "public-receipt");
    assert!(matches!(&sent[1], Message::Pong(bytes) if bytes.as_ref() == [1]));
    let reasons: Vec<_> = sent[2..]
        .iter()
        .map(|message| {
            serde_json::from_str::<serde_json::Value>(message.to_text().unwrap()).unwrap()["reason"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(reasons, ["priority", "event-1", "event-2"]);
}

#[test]
fn retained_cloud_private_relay_does_not_hydrate_cloud_transport() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        kernel_id: None,
        kernel_credential: None,
        kernel_public_key_thumbprint: None,
        api_url: "https://cloud.example.test".to_string(),
        email: "user@example.test".to_string(),
        account_id: "account-1".to_string(),
        user_id: "user-1".to_string(),
        account_slug: "account".to_string(),
        realm_id: "realm-1".to_string(),
        relay_url: "wss://relay.example.test".to_string(),
        issuer_id: "issuer-1".to_string(),
        client_id: None,
        client_alias: None,
        machine_id: None,
        machine_alias: None,
        machine_credential: Some("machine-credential".to_string()),
        cloud_session_token: None,
        cloud_session_expires_at_ms: None,
        token_expires_at_ms: None,
    });

    config.relay_url = Some("ws://127.0.0.1:47000".into());
    config.relay_token = Some("synthetic-private-relay".into());
    assert!(
        !should_refresh_cloud_relay(&config),
        "MP-08 / MP-10 / MP-11 restart must retain the active private override"
    );
}

#[test]
fn cloud_retry_hint_reads_the_seconds_cloud_asks_for() {
    assert_eq!(
        cloud_retry_hint("cloud relay request failed with 429: cloud_api_code=rate_limited: Too many requests, retry in 20 seconds"),
        Some(Duration::from_secs(20))
    );
    assert_eq!(
        cloud_retry_hint("retry in 1 second"),
        Some(Duration::from_secs(1))
    );
    assert_eq!(cloud_retry_hint("retry in a while"), None);
    assert_eq!(cloud_retry_hint("cloud unavailable"), None);
    assert_eq!(
        cloud_retry_hint("retry in 99999 seconds"),
        Some(Duration::from_secs(3_600))
    );
}

#[test]
fn token_renewal_waits_at_least_the_hinted_time_and_at_most_a_minute_of_backoff() {
    let hinted =
        token_renewal_retry_delay("daemon-a", 0, Some("429 rate_limited, retry in 45 seconds"));
    assert!(hinted >= Duration::from_secs(45));
    for failures in [6, 20, 99] {
        let delay = token_renewal_retry_delay("daemon-a", failures, None);
        assert!(delay >= Duration::from_secs(60));
        assert!(delay <= Duration::from_secs(61));
    }
    assert!(token_renewal_retry_delay("daemon-a", 0, None) < Duration::from_secs(2));
}

#[test]
fn relay_wording_for_a_refused_token_is_recognised() {
    for message in [
        "relay payload handling failed: relay token is expired",
        "relay token expired",
        "relay token has been revoked",
    ] {
        assert!(relay_refused_token(message), "{message}");
    }
    assert!(!relay_refused_token("relay connection idle timeout"));
    assert!(!relay_refused_token("cloud token refresh timed out"));
}

#[test]
fn only_a_link_that_stayed_up_resets_the_drop_backoff() {
    let mut backoff = DropBackoff::default();
    assert_eq!(backoff.delay("daemon-a"), Duration::ZERO);
    for expected_secs in [1, 2, 4, 8] {
        backoff.after_connection(Duration::from_secs(2));
        assert!(backoff.delay("daemon-a") >= Duration::from_secs(expected_secs));
    }
    backoff.after_connection(STABLE_RELAY_CONNECTION);
    assert_eq!(backoff.delay("daemon-a"), Duration::ZERO);
}
