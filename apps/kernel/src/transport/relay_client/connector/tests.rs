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
