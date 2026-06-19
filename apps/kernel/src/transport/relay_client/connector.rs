//! Relay socket connection lifecycle and heartbeat loop.

use std::collections::VecDeque;
use std::future::Future;

use futures_util::{Sink, SinkExt};

use super::*;

pub async fn run_daemon_relay_connector(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
) {
    run_daemon_relay_connector_inner(app, state, &mut shutdown, None).await;
}

pub async fn run_daemon_relay_connector_with_static_relay(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
    relay_url: String,
    relay_token: String,
) {
    run_daemon_relay_connector_inner(
        app,
        state,
        &mut shutdown,
        Some(StaticRelayConfig {
            relay_url,
            relay_token,
        }),
    )
    .await;
}

struct StaticRelayConfig {
    relay_url: String,
    relay_token: String,
}

const CLOUD_RELAY_TOKEN_REFRESH_TIMEOUT: Duration = Duration::from_secs(5);
const CLOUD_RELAY_PRESENCE_JITTER_SPREAD_MS: u64 = 5_000;
const RELAY_RECONNECT_BASE_DELAY_MS: u64 = 500;
const RELAY_RECONNECT_MAX_DELAY_MS: u64 = 5_000;
const RELAY_RECONNECT_JITTER_SPREAD_MS: u64 = 500;
const RELAY_EVENT_WRITE_COALESCE_MS: u64 = 33;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingRelayConfigDisposition {
    LocalIdle,
    CloudUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CloudRelayRefreshResult {
    Refreshed,
    Failed(String),
    TimedOut,
}

fn should_refresh_cloud_relay(config: &crate::config::DaemonConfig) -> bool {
    config.cloud_relay.is_some()
}

fn missing_relay_config_disposition(
    config: &crate::config::DaemonConfig,
) -> MissingRelayConfigDisposition {
    if config.cloud_relay.is_some() {
        MissingRelayConfigDisposition::CloudUnavailable
    } else {
        MissingRelayConfigDisposition::LocalIdle
    }
}

fn should_start_cloud_presence_publish(task: Option<&JoinHandle<()>>) -> bool {
    task.is_none_or(|task| task.is_finished())
}

fn should_start_cloud_token_refresh(task: Option<&JoinHandle<()>>) -> bool {
    task.is_none_or(|task| task.is_finished())
}

fn should_start_leased_projection_pump(task: Option<&JoinHandle<()>>) -> bool {
    task.is_none_or(|task| task.is_finished())
}

fn cloud_presence_refresh_interval(daemon_id: &str) -> Duration {
    CLOUD_RELAY_PRESENCE_REFRESH_INTERVAL
        + Duration::from_millis(stable_jitter_ms(
            daemon_id,
            CLOUD_RELAY_PRESENCE_JITTER_SPREAD_MS,
        ))
}

#[derive(Debug)]
struct CloudPresencePublishSchedule {
    next_publish_at: Instant,
    interval: Duration,
}

impl CloudPresencePublishSchedule {
    fn new(now: Instant, daemon_id: &str) -> Self {
        Self {
            next_publish_at: now + cloud_presence_refresh_interval(daemon_id),
            interval: cloud_presence_refresh_interval(daemon_id),
        }
    }

    #[cfg(test)]
    fn new_with_interval(now: Instant, interval: Duration) -> Self {
        Self {
            next_publish_at: now + interval,
            interval,
        }
    }

    fn claim_publish_if_due(&mut self, now: Instant, task: Option<&JoinHandle<()>>) -> bool {
        if now < self.next_publish_at {
            return false;
        }
        self.next_publish_at = now + self.interval;
        should_start_cloud_presence_publish(task)
    }
}

fn stable_jitter_ms(value: &str, spread_ms: u64) -> u64 {
    if spread_ms == 0 {
        return 0;
    }
    value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
        % spread_ms
}

async fn send_relay_envelope_frame<S>(writer: &mut S, envelope: RelayEnvelope) -> bool
where
    S: Sink<Message> + Unpin,
{
    let payload = match serde_json::to_string(&envelope) {
        Ok(payload) => payload,
        Err(_) => return false,
    };
    writer.send(Message::Text(payload.into())).await.is_ok()
}

#[derive(Debug)]
struct RelayEventWriteCoalescer<T> {
    delay_ms: u64,
    envelopes: VecDeque<T>,
    ready_at: Option<tokio::time::Instant>,
}

impl<T> RelayEventWriteCoalescer<T> {
    fn new(delay_ms: u64) -> Self {
        Self {
            delay_ms,
            envelopes: VecDeque::new(),
            ready_at: None,
        }
    }

    fn is_empty(&self) -> bool {
        self.envelopes.is_empty()
    }

    fn ready_at(&self) -> Option<tokio::time::Instant> {
        self.ready_at
    }

    fn push_event(&mut self, envelope: T, now: tokio::time::Instant) -> Option<T> {
        if self.delay_ms == 0 {
            return Some(envelope);
        }
        self.envelopes.push_back(envelope);
        if self.ready_at.is_none() {
            self.ready_at = Some(now + Duration::from_millis(self.delay_ms));
        }
        None
    }

    fn drain_ready(&mut self) -> Vec<T> {
        self.ready_at = None;
        self.envelopes.drain(..).collect()
    }
}

fn relay_reconnect_delay(daemon_id: &str, attempt: u32) -> Duration {
    let capped_attempt = attempt.min(4);
    let backoff_ms = RELAY_RECONNECT_BASE_DELAY_MS.saturating_mul(1_u64 << capped_attempt);
    let bounded_backoff_ms = backoff_ms.min(RELAY_RECONNECT_MAX_DELAY_MS);
    Duration::from_millis(
        bounded_backoff_ms + stable_jitter_ms(daemon_id, RELAY_RECONNECT_JITTER_SPREAD_MS),
    )
}

async fn wait_for_reconnect_delay(shutdown: &mut watch::Receiver<bool>, delay: Duration) -> bool {
    let wait = sleep(delay);
    tokio::pin!(wait);
    tokio::select! {
        changed = shutdown.changed() => changed.is_ok() && *shutdown.borrow(),
        _ = &mut wait => false,
    }
}

fn record_relay_reconnect(router: &CommandRouter, relay_url: &str, reason: &str, delay: Duration) {
    router
        .transport_health_store()
        .record_relay_reconnect_attempt(relay_url, reason, delay);
}

fn abort_leased_projection_pump_task(task: &mut Option<JoinHandle<()>>) {
    if let Some(handle) = task.take() {
        handle.abort();
    }
}

async fn bounded_cloud_relay_refresh<F, E>(
    refresh: F,
    refresh_timeout: Duration,
) -> CloudRelayRefreshResult
where
    F: Future<Output = Result<(), E>>,
    E: ToString,
{
    match timeout(refresh_timeout, refresh).await {
        Ok(Ok(())) => CloudRelayRefreshResult::Refreshed,
        Ok(Err(error)) => CloudRelayRefreshResult::Failed(error.to_string()),
        Err(_) => CloudRelayRefreshResult::TimedOut,
    }
}

fn spawn_cloud_token_refresh(router: Arc<CommandRouter>, relay_url: String) -> JoinHandle<()> {
    tokio::spawn(async move {
        match bounded_cloud_relay_refresh(
            router.ensure_cloud_relay_connection(),
            CLOUD_RELAY_TOKEN_REFRESH_TIMEOUT,
        )
        .await
        {
            CloudRelayRefreshResult::Refreshed => {
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "cloud relay token refresh completed",
                    serde_json::json!({
                        "relay_url": relay_url,
                    }),
                );
            }
            CloudRelayRefreshResult::Failed(error) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "failed to refresh cloud relay token",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "error": error,
                    }),
                );
            }
            CloudRelayRefreshResult::TimedOut => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "cloud relay token refresh timed out",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "timeout_ms": CLOUD_RELAY_TOKEN_REFRESH_TIMEOUT.as_millis(),
                    }),
                );
            }
        }
    })
}

fn spawn_leased_projection_pump(
    router: Arc<CommandRouter>,
    outgoing_tx: RelayOutgoingSender,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if timeout(
            RELAY_HEARTBEAT_APP_WORK_TIMEOUT,
            pump_leased_projection_events(&router, &outgoing_tx),
        )
        .await
        .is_err()
        {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "leased projection pump timed out",
                serde_json::json!({
                    "timeout_ms": RELAY_HEARTBEAT_APP_WORK_TIMEOUT.as_millis(),
                }),
            );
        }
    })
}

async fn disconnect_relay(
    router: &Arc<CommandRouter>,
    state: &Arc<RwLock<RelayClientState>>,
    reason: &str,
    publish_presence: bool,
) {
    if publish_presence {
        publish_offline_and_set_disconnected(router, state, reason).await;
    } else {
        crate::logging::warn_with_fields(
            "daemon.relay_client",
            "relay socket disconnected",
            serde_json::json!({
                "reason": reason,
            }),
        );
        super::connection_state::set_disconnected(state).await;
    }
}

async fn run_daemon_relay_connector_inner(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    shutdown: &mut watch::Receiver<bool>,
    static_relay: Option<StaticRelayConfig>,
) {
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        app,
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    let event_runtime = match RelayEventRuntime::new(router.relay_event_counter_path()) {
        Ok(runtime) => Arc::new(runtime),
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "failed to initialize relay event id allocator",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
            return;
        }
    };
    let command_result_cache_path = router
        .relay_event_counter_path()
        .with_file_name("relay-command-results.jsonl");
    let command_result_cache =
        match CommandResultCache::new_with_persistent_path(command_result_cache_path) {
            Ok(cache) => Arc::new(cache),
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "failed to initialize relay command result cache",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
    let command_sequence = Arc::new(AtomicU64::new(1));
    let mut missing_relay_config_reported = false;
    let mut reconnect_attempt = 0_u32;

    loop {
        if *shutdown.borrow() {
            disconnect_relay(
                &router,
                &state,
                "shutdown before relay connect",
                static_relay.is_none(),
            )
            .await;
            return;
        }

        if static_relay.is_none() {
            let config = router.relay_config_snapshot();
            if should_refresh_cloud_relay(&config) {
                let cloud_refresh_started = Instant::now();
                match bounded_cloud_relay_refresh(
                    router.ensure_cloud_relay_connection(),
                    CLOUD_RELAY_TOKEN_REFRESH_TIMEOUT,
                )
                .await
                {
                    CloudRelayRefreshResult::Refreshed => {
                        crate::logging::info_with_fields(
                            "daemon.startup",
                            "cloud relay profile hydrated",
                            serde_json::json!({
                                "refresh_ms": cloud_refresh_started.elapsed().as_millis(),
                            }),
                        );
                    }
                    CloudRelayRefreshResult::Failed(error) => {
                        crate::logging::warn_with_fields(
                            "daemon.relay_client",
                            "failed to refresh cloud relay token",
                            serde_json::json!({
                                "refresh_ms": cloud_refresh_started.elapsed().as_millis(),
                                "error": error,
                            }),
                        );
                    }
                    CloudRelayRefreshResult::TimedOut => {
                        crate::logging::warn_with_fields(
                            "daemon.relay_client",
                            "cloud relay token refresh timed out before connect",
                            serde_json::json!({
                                "refresh_ms": cloud_refresh_started.elapsed().as_millis(),
                                "timeout_ms": CLOUD_RELAY_TOKEN_REFRESH_TIMEOUT.as_millis(),
                            }),
                        );
                    }
                }
            }
        }

        let (relay_url, mut active_relay_token, heartbeat) = {
            let config = router.relay_config_snapshot();
            if let Some(static_relay) = static_relay.as_ref() {
                (
                    static_relay.relay_url.clone(),
                    static_relay.relay_token.clone(),
                    Duration::from_millis(config.relay_heartbeat_ms),
                )
            } else {
                match (config.relay_url.clone(), config.relay_token.clone()) {
                    (Some(relay_url), Some(relay_token)) => (
                        relay_url,
                        relay_token,
                        Duration::from_millis(config.relay_heartbeat_ms),
                    ),
                    _ => {
                        if !missing_relay_config_reported {
                            match missing_relay_config_disposition(&config) {
                                MissingRelayConfigDisposition::CloudUnavailable => {
                                    disconnect_relay(
                                        &router,
                                        &state,
                                        "relay configuration unavailable",
                                        true,
                                    )
                                    .await;
                                }
                                MissingRelayConfigDisposition::LocalIdle => {
                                    crate::logging::info_with_fields(
                                        "daemon.relay_client",
                                        "relay connector idle",
                                        serde_json::json!({
                                            "reason": "relay configuration unavailable",
                                        }),
                                    );
                                    super::connection_state::set_disconnected(&state).await;
                                }
                            }
                            missing_relay_config_reported = true;
                        } else {
                            super::connection_state::set_disconnected(&state).await;
                        }
                        let wait = sleep(Duration::from_secs(1));
                        tokio::pin!(wait);
                        tokio::select! {
                            changed = shutdown.changed() => {
                                if changed.is_ok() && *shutdown.borrow() {
                                    disconnect_relay(
                                        &router,
                                        &state,
                                        "shutdown while relay configuration unavailable",
                                        true,
                                    )
                                    .await;
                                    return;
                                }
                            }
                            _ = &mut wait => {}
                        }
                        continue;
                    }
                }
            }
        };
        missing_relay_config_reported = false;

        crate::logging::info_with_fields(
            "daemon.relay_client",
            "attempting relay connection",
            serde_json::json!({
                "relay_url": relay_url,
            }),
        );
        let connect_started = Instant::now();
        match timeout(RELAY_CONNECT_TIMEOUT, connect_async(&relay_url)).await {
            Err(_) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "relay socket connect timed out",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "connect_ms": connect_started.elapsed().as_millis(),
                        "timeout_ms": RELAY_CONNECT_TIMEOUT.as_millis(),
                    }),
                );
                disconnect_relay(
                    &router,
                    &state,
                    "relay connect timed out",
                    static_relay.is_none(),
                )
                .await;
                let daemon_id = router.relay_daemon_id();
                let delay = relay_reconnect_delay(&daemon_id, reconnect_attempt);
                record_relay_reconnect(&router, &relay_url, "relay connect timed out", delay);
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                if wait_for_reconnect_delay(shutdown, delay).await {
                    return;
                }
                continue;
            }
            Ok(Ok((socket, _))) => {
                let connect_ms = connect_started.elapsed().as_millis();
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "relay socket connected",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "connect_ms": connect_ms,
                    }),
                );
                let registration_started = Instant::now();
                let (mut writer, mut reader) = socket.split();
                let (outgoing_tx, mut priority_outgoing_rx, mut event_outgoing_rx) =
                    RelayOutgoingSender::channel(RELAY_OUTGOING_QUEUE_LIMIT);
                let (pong_tx, mut pong_rx) = mpsc::channel::<Vec<u8>>(RELAY_OUTGOING_QUEUE_LIMIT);
                let (writer_done_tx, mut writer_done_rx) = oneshot::channel::<()>();
                let writer_task = tokio::spawn(async move {
                    let mut priority_open = true;
                    let mut event_open = true;
                    let mut event_write_coalescer =
                        RelayEventWriteCoalescer::new(RELAY_EVENT_WRITE_COALESCE_MS);
                    'writer_loop: while priority_open
                        || event_open
                        || !event_write_coalescer.is_empty()
                    {
                        if let Some(ready_at) = event_write_coalescer.ready_at() {
                            tokio::select! {
                                biased;
                                Some(payload) = pong_rx.recv() => {
                                    if writer.send(Message::Pong(payload.into())).await.is_err() {
                                        break;
                                    }
                                }
                                envelope = priority_outgoing_rx.recv(), if priority_open => {
                                    match envelope {
                                        Some(envelope) => {
                                            if !send_relay_envelope_frame(&mut writer, envelope).await {
                                                break;
                                            }
                                        }
                                        None => priority_open = false,
                                    }
                                }
                                envelope = event_outgoing_rx.recv(), if event_open => {
                                    match envelope {
                                        Some(envelope) => {
                                            if let Some(envelope) = event_write_coalescer.push_event(envelope, tokio::time::Instant::now()) {
                                                if !send_relay_envelope_frame(&mut writer, envelope).await {
                                                    break;
                                                }
                                            }
                                        }
                                        None => event_open = false,
                                    }
                                }
                                _ = tokio::time::sleep_until(ready_at) => {
                                    for envelope in event_write_coalescer.drain_ready() {
                                        if !send_relay_envelope_frame(&mut writer, envelope).await {
                                            break 'writer_loop;
                                        }
                                    }
                                }
                            }
                            continue;
                        }

                        tokio::select! {
                            biased;
                            Some(payload) = pong_rx.recv() => {
                                if writer.send(Message::Pong(payload.into())).await.is_err() {
                                    break;
                                }
                            }
                            envelope = priority_outgoing_rx.recv(), if priority_open => {
                                match envelope {
                                    Some(envelope) => {
                                        if !send_relay_envelope_frame(&mut writer, envelope).await {
                                            break;
                                        }
                                    }
                                    None => priority_open = false,
                                }
                            }
                            envelope = event_outgoing_rx.recv(), if event_open => {
                                match envelope {
                                    Some(envelope) => {
                                        if let Some(envelope) = event_write_coalescer.push_event(envelope, tokio::time::Instant::now()) {
                                            if !send_relay_envelope_frame(&mut writer, envelope).await {
                                                break;
                                            }
                                        }
                                    }
                                    None => event_open = false,
                                }
                            }
                        }
                    }
                    let _ = writer_done_tx.send(());
                });
                let subscription_tasks: RelaySubscriptionTasks =
                    Arc::new(Mutex::new(BTreeMap::new()));
                let daemon_id = router.relay_daemon_id();
                let mut registration = router.relay_registration().await;
                if static_relay.is_some() {
                    registration.auth_token = active_relay_token.clone();
                }
                let register = RelayEnvelope::DaemonRegister { registration };
                if send_outgoing_envelope(&outgoing_tx, register).is_err() {
                    writer_task.abort();
                    clear_remote_inventory_projection(&router);
                    disconnect_relay(
                        &router,
                        &state,
                        "failed to send relay registration",
                        static_relay.is_none(),
                    )
                    .await;
                    let delay = relay_reconnect_delay(&daemon_id, reconnect_attempt);
                    record_relay_reconnect(
                        &router,
                        &relay_url,
                        "failed to send relay registration",
                        delay,
                    );
                    reconnect_attempt = reconnect_attempt.saturating_add(1);
                    if wait_for_reconnect_delay(shutdown, delay).await {
                        return;
                    }
                    continue;
                }
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "relay register sent",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "connect_ms": connect_ms,
                        "registration_send_ms": registration_started.elapsed().as_millis(),
                        "connect_to_registration_ms": connect_started.elapsed().as_millis(),
                    }),
                );
                set_connected(&state, outgoing_tx.clone(), relay_url.clone()).await;
                router
                    .transport_health_store()
                    .record_relay_connected(&relay_url);
                reconnect_attempt = 0;
                let mut cloud_presence_task = None;
                if static_relay.is_none() {
                    cloud_presence_task = Some(spawn_cloud_presence_publish(
                        Arc::clone(&router),
                        true,
                        "relay registration sent",
                    ));
                }
                let mut cloud_presence_schedule =
                    CloudPresencePublishSchedule::new(Instant::now(), &daemon_id);
                let mut inventory_refresh_task = static_relay
                    .is_none()
                    .then(|| spawn_remote_inventory_projection_refresh(Arc::clone(&router)));
                let mut heartbeat_interval = tokio::time::interval(heartbeat);
                heartbeat_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
                let mut token_refresh_interval =
                    tokio::time::interval(CLOUD_RELAY_TOKEN_REFRESH_CHECK_INTERVAL);
                token_refresh_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
                let mut token_refresh_task = None;
                let mut leased_projection_pump_task = None;
                let mut heartbeat_tick: u64 = 0;

                let reconnect_reason = loop {
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_ok() && *shutdown.borrow() {
                                if static_relay.is_none() {
                                    let _ = spawn_cloud_presence_publish(
                                        Arc::clone(&router),
                                        false,
                                        "daemon shutting down",
                                    );
                                }
                                let _ = send_outgoing_envelope(&outgoing_tx, RelayEnvelope::Close {
                                    reason: "daemon shutting down".to_string(),
                                });
                                sleep(Duration::from_millis(25)).await;
                                abort_leased_projection_pump_task(&mut leased_projection_pump_task);
                                abort_inventory_refresh_task(&mut inventory_refresh_task);
                                abort_subscription_tasks(&subscription_tasks).await;
                                writer_task.abort();
                                clear_remote_inventory_projection(&router);
                                disconnect_relay(&router, &state, "daemon shutting down", static_relay.is_none()).await;
                                return;
                            }
                        }
                        incoming = reader.next() => {
                            match incoming {
                                Some(Ok(Message::Text(payload))) => {
                                    if let Err(error) = handle_incoming_envelope(
                                        &router,
                                        &command_sequence,
                                        &state,
                                        &outgoing_tx,
                                        &subscription_tasks,
                                        &event_runtime,
                                        &command_result_cache,
                                        &payload,
                                    )
                                    .await
                                    {
                                        let disconnect_reason =
                                            format!("relay payload handling failed: {error}");
                                        abort_leased_projection_pump_task(
                                            &mut leased_projection_pump_task,
                                        );
                                        abort_inventory_refresh_task(&mut inventory_refresh_task);
                                        abort_subscription_tasks(&subscription_tasks).await;
                                        writer_task.abort();
                                        clear_remote_inventory_projection(&router);
                                        disconnect_relay(
                                            &router,
                                            &state,
                                            &disconnect_reason,
                                            static_relay.is_none(),
                                        )
                                        .await;
                                        break "relay payload handling failed";
                                    }
                                }
                                Some(Ok(Message::Close(_))) => {
                                    abort_leased_projection_pump_task(
                                        &mut leased_projection_pump_task,
                                    );
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&router);
                                    disconnect_relay(&router, &state, "relay close frame received", static_relay.is_none()).await;
                                    break "relay close frame received";
                                }
                                Some(Ok(Message::Ping(payload))) => {
                                    let _ = pong_tx.try_send(payload.to_vec());
                                }
                                Some(Ok(Message::Pong(_))) => {}
                                Some(Ok(_)) => {}
                                Some(Err(_)) | None => {
                                    abort_leased_projection_pump_task(
                                        &mut leased_projection_pump_task,
                                    );
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&router);
                                    disconnect_relay(&router, &state, "relay read failed or ended", static_relay.is_none()).await;
                                    break "relay read failed or ended";
                                }
                            }
                        }
                        writer_done = &mut writer_done_rx => {
                            let _ = writer_done;
                            abort_leased_projection_pump_task(&mut leased_projection_pump_task);
                            abort_inventory_refresh_task(&mut inventory_refresh_task);
                            abort_subscription_tasks(&subscription_tasks).await;
                            writer_task.abort();
                            clear_remote_inventory_projection(&router);
                            disconnect_relay(&router, &state, "relay writer ended", static_relay.is_none()).await;
                            break "relay writer ended";
                        }
                        _ = token_refresh_interval.tick() => {
                            if static_relay.is_some() {
                                continue;
                            }
                            if router.cloud_relay_token_refresh_due()
                                && should_start_cloud_token_refresh(token_refresh_task.as_ref())
                            {
                                token_refresh_task = Some(spawn_cloud_token_refresh(
                                    Arc::clone(&router),
                                    relay_url.clone(),
                                ));
                            }
                            match relay_config_continuity(
                                &relay_url,
                                &active_relay_token,
                                &router.relay_config_snapshot(),
                            ) {
                                RelayConfigContinuity::Continue => {}
                                RelayConfigContinuity::TokenRotated(next_token) => {
                                    active_relay_token = next_token;
                                    crate::logging::info_with_fields(
                                        "daemon.relay_client",
                                        "relay token rotated on active socket",
                                        serde_json::json!({
                                            "relay_url": relay_url,
                                        }),
                                    );
                                }
                                RelayConfigContinuity::Reconnect(reason) => {
                                    crate::logging::warn_with_fields(
                                        "daemon.relay_client",
                                        "relay socket reconnect requested",
                                        serde_json::json!({
                                            "relay_url": relay_url,
                                            "reason": reason,
                                            "phase": "token_refresh",
                                        }),
                                    );
                                    let _ = send_outgoing_envelope(&outgoing_tx, RelayEnvelope::Close {
                                        reason: "relay configuration changed".to_string(),
                                    });
                                    abort_leased_projection_pump_task(&mut leased_projection_pump_task);
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&router);
                                    disconnect_relay(&router, &state, "relay configuration changed", true).await;
                                    break "relay configuration changed";
                                }
                            }
                        }
                        _ = heartbeat_interval.tick() => {
                            heartbeat_tick = heartbeat_tick.wrapping_add(1);
                            if static_relay.is_none() {
                                match relay_config_continuity(
                                    &relay_url,
                                    &active_relay_token,
                                    &router.relay_config_snapshot(),
                                ) {
                                    RelayConfigContinuity::Continue => {}
                                    RelayConfigContinuity::TokenRotated(next_token) => {
                                        active_relay_token = next_token;
                                        crate::logging::info_with_fields(
                                            "daemon.relay_client",
                                            "relay token rotated on active socket",
                                            serde_json::json!({
                                                "relay_url": relay_url,
                                            }),
                                        );
                                    }
                                    RelayConfigContinuity::Reconnect(reason) => {
                                        crate::logging::warn_with_fields(
                                            "daemon.relay_client",
                                            "relay socket reconnect requested",
                                            serde_json::json!({
                                                "relay_url": relay_url,
                                                "reason": reason,
                                                "phase": "heartbeat",
                                            }),
                                        );
                                        let _ = send_outgoing_envelope(&outgoing_tx, RelayEnvelope::Close {
                                            reason: "relay configuration changed".to_string(),
                                        });
                                        abort_leased_projection_pump_task(
                                            &mut leased_projection_pump_task,
                                        );
                                        abort_inventory_refresh_task(&mut inventory_refresh_task);
                                        abort_subscription_tasks(&subscription_tasks).await;
                                        writer_task.abort();
                                        clear_remote_inventory_projection(&router);
                                        disconnect_relay(&router, &state, "relay configuration changed", true).await;
                                        break "relay configuration changed";
                                    }
                                }
                            }
                            if static_relay.is_none()
                                && heartbeat_tick.is_multiple_of(RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS)
                                && inventory_refresh_task
                                    .as_ref()
                                    .is_none_or(|task| task.is_finished())
                            {
                                inventory_refresh_task = Some(
                                    spawn_remote_inventory_projection_refresh(
                                        Arc::clone(&router),
                                    )
                                );
                            }
                            let heartbeat_frame = RelayEnvelope::DaemonHeartbeat {
                                daemon_id: daemon_id.clone(),
                                registration: None,
                            };
                            if send_outgoing_envelope(&outgoing_tx, heartbeat_frame).is_err() {
                                abort_leased_projection_pump_task(&mut leased_projection_pump_task);
                                abort_inventory_refresh_task(&mut inventory_refresh_task);
                                abort_subscription_tasks(&subscription_tasks).await;
                                writer_task.abort();
                                clear_remote_inventory_projection(&router);
                                disconnect_relay(&router, &state, "relay heartbeat send failed", static_relay.is_none()).await;
                                break "relay heartbeat send failed";
                            }
                            if should_start_leased_projection_pump(
                                leased_projection_pump_task.as_ref(),
                            ) {
                                leased_projection_pump_task = Some(spawn_leased_projection_pump(
                                    Arc::clone(&router),
                                    outgoing_tx.clone(),
                                ));
                            }
                            if static_relay.is_none()
                                && cloud_presence_schedule.claim_publish_if_due(
                                    Instant::now(),
                                    cloud_presence_task.as_ref(),
                                )
                            {
                                cloud_presence_task = Some(spawn_cloud_presence_publish(
                                    Arc::clone(&router),
                                    true,
                                    "relay heartbeat",
                                ));
                            }
                        }
                    }
                };
                let delay = relay_reconnect_delay(&daemon_id, reconnect_attempt);
                record_relay_reconnect(&router, &relay_url, reconnect_reason, delay);
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                if wait_for_reconnect_delay(shutdown, delay).await {
                    return;
                }
            }
            Ok(Err(error)) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "relay socket connect failed",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "connect_ms": connect_started.elapsed().as_millis(),
                        "error": error.to_string(),
                    }),
                );
                clear_remote_inventory_projection(&router);
                disconnect_relay(
                    &router,
                    &state,
                    "relay socket connect failed",
                    static_relay.is_none(),
                )
                .await;
                let daemon_id = router.relay_daemon_id();
                let delay = relay_reconnect_delay(&daemon_id, reconnect_attempt);
                record_relay_reconnect(&router, &relay_url, "relay socket connect failed", delay);
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                if wait_for_reconnect_delay(shutdown, delay).await {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
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

        assert_eq!(coalescer.drain_ready(), vec!["event-1", "event-2"]);
        assert_eq!(coalescer.ready_at(), None);
        assert!(coalescer.is_empty());
    }

    #[test]
    fn relay_event_writer_can_disable_event_coalescing_for_tests() {
        let now = TokioInstant::now();
        let mut coalescer = RelayEventWriteCoalescer::new(0);

        assert_eq!(coalescer.push_event("event-1", now), Some("event-1"));
        assert_eq!(coalescer.ready_at(), None);
        assert!(coalescer.drain_ready().is_empty());
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

        assert!(
            !schedule.claim_publish_if_due(start + Duration::from_millis(100), Some(&running_task))
        );
        assert!(
            !schedule.claim_publish_if_due(start + Duration::from_millis(101), Some(&running_task))
        );
        assert!(
            !schedule.claim_publish_if_due(start + Duration::from_millis(199), Some(&running_task))
        );

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
        let result = bounded_cloud_relay_refresh(
            async { Ok::<(), &'static str>(()) },
            Duration::from_secs(1),
        )
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
}
