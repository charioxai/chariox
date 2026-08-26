use super::writer::{RelayEventWriteCoalescer, RELAY_EVENT_WRITE_COALESCE_MS};
use super::*;
use tokio::time::Instant as TokioInstant;

#[test]
fn relay_event_writer_coalesces_event_lane_with_stable_deadline() {
    let now = TokioInstant::now();
    let mut coalescer = RelayEventWriteCoalescer::new(RELAY_EVENT_WRITE_COALESCE_MS);

    assert!(coalescer.push_event("event-1", now).is_none());
    assert_eq!(
        coalescer.ready_at(),
        Some(now + Duration::from_millis(RELAY_EVENT_WRITE_COALESCE_MS))
    );
    assert!(coalescer
        .push_event("event-2", now + Duration::from_millis(10))
        .is_none());
    assert_eq!(
        coalescer.ready_at(),
        Some(now + Duration::from_millis(RELAY_EVENT_WRITE_COALESCE_MS))
    );

    assert_eq!(
        coalescer.pop_ready(now + Duration::from_millis(RELAY_EVENT_WRITE_COALESCE_MS)),
        Some("event-1")
    );
    assert_eq!(
        coalescer.ready_at(),
        Some(now + Duration::from_millis(RELAY_EVENT_WRITE_COALESCE_MS))
    );
    assert_eq!(
        coalescer.pop_ready(now + Duration::from_millis(RELAY_EVENT_WRITE_COALESCE_MS)),
        Some("event-2")
    );
    assert_eq!(coalescer.ready_at(), None);
    assert!(coalescer.is_empty());
}

#[test]
fn relay_event_writer_can_disable_event_coalescing_for_tests() {
    let now = TokioInstant::now();
    let mut coalescer = RelayEventWriteCoalescer::new(0);

    assert_eq!(coalescer.push_event("event-1", now), Some("event-1"));
    assert_eq!(coalescer.ready_at(), None);
    assert!(coalescer.pop_ready(now).is_none());
}

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
async fn cloud_presence_publish_gate_skips_running_task() {
    let (_tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = rx.await;
    });

    assert!(!should_start_cloud_presence_publish(Some(&task)));

    task.abort();
}

#[tokio::test]
async fn cloud_presence_publish_gate_allows_finished_or_missing_task() {
    let task = tokio::spawn(async {});
    while !task.is_finished() {
        tokio::task::yield_now().await;
    }

    assert!(should_start_cloud_presence_publish(None));
    assert!(should_start_cloud_presence_publish(Some(&task)));
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

    assert!(!schedule.claim_publish_if_due(start + Duration::from_millis(99), None));
    assert!(schedule.claim_publish_if_due(start + Duration::from_millis(100), None));
    assert!(!schedule.claim_publish_if_due(start + Duration::from_millis(101), None));
    assert!(schedule.claim_publish_if_due(start + Duration::from_millis(200), None));
}

#[tokio::test]
async fn cloud_presence_schedule_does_not_retry_every_heartbeat_while_task_runs() {
    let start = Instant::now();
    let mut schedule =
        CloudPresencePublishSchedule::new_with_interval(start, Duration::from_millis(100));
    let (_tx, rx) = oneshot::channel::<()>();
    let running_task = tokio::spawn(async move {
        let _ = rx.await;
    });

    assert!(!schedule.claim_publish_if_due(start + Duration::from_millis(100), Some(&running_task)));
    assert!(!schedule.claim_publish_if_due(start + Duration::from_millis(101), Some(&running_task)));
    assert!(!schedule.claim_publish_if_due(start + Duration::from_millis(199), Some(&running_task)));

    running_task.abort();
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
        exercise_post_connect_confirmation(true, false).await;
    assert!(owner_confirmed);
    assert!(!worker_pending);
    assert_eq!(attempts, 1);
}

#[tokio::test]
async fn post_connect_hook_clears_worker_pending_after_terminal_owner_rejection() {
    let (owner_confirmed, worker_pending, attempts) =
        exercise_post_connect_confirmation(false, false).await;
    assert!(!owner_confirmed);
    assert!(!worker_pending);
    assert_eq!(attempts, 1);
}

#[tokio::test]
async fn post_connect_hook_retries_retryable_rejection_on_same_connection() {
    let (owner_confirmed, worker_pending, attempts) =
        exercise_post_connect_confirmation(true, true).await;
    assert!(owner_confirmed);
    assert!(!worker_pending);
    assert_eq!(attempts, 2);
}

async fn exercise_post_connect_confirmation(
    owner_accepts_worker_key: bool,
    reject_first_attempt_as_retryable: bool,
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
    set_connected(
        &worker_state,
        worker_outgoing,
        "wss://relay.example.test".to_string(),
    )
    .await
    .expect("worker relay should become connected");
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
