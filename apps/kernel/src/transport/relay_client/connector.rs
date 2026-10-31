//! Relay socket connection lifecycle and heartbeat loop.

use std::future::Future;

use super::*;
use crate::runtime::cloud_relay_control::cloud_relay_token_is_expired;

mod managed_slice_activation;
mod writer;
#[cfg(test)]
use managed_slice_activation::{
    managed_slice_token_confirmation_matches,
    spawn_pending_managed_slice_activation_confirmation_after_connect,
};
use managed_slice_activation::{socket_has_current_token, ManagedSliceActivationConfirmation};

#[cfg(test)]
pub async fn run_daemon_relay_connector(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        app,
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    run_daemon_relay_connector_inner(router, state, &mut shutdown, None).await;
}

pub async fn run_daemon_relay_connector_with_router(
    router: Arc<CommandRouter>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
) {
    run_daemon_relay_connector_inner(router, state, &mut shutdown, None).await;
}

#[cfg(test)]
pub async fn run_daemon_relay_connector_with_router_and_static_relay(
    router: Arc<CommandRouter>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
    relay_url: String,
    relay_token: String,
) {
    run_daemon_relay_connector_inner(
        router,
        state,
        &mut shutdown,
        Some(StaticRelayConfig {
            relay_url,
            relay_token,
        }),
    )
    .await;
}

pub async fn run_daemon_relay_connector_with_static_relay(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
    relay_url: String,
    relay_token: String,
) {
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        app,
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    run_daemon_relay_connector_inner(
        router,
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
const _: () = assert!(
    crate::runtime::cloud_api_client::CLOUD_RELAY_TOKEN_REQUEST_TIMEOUT.as_millis()
        < CLOUD_RELAY_TOKEN_REFRESH_TIMEOUT.as_millis()
);
const CLOUD_RELAY_PRESENCE_JITTER_SPREAD_MS: u64 = 5_000;
const RELAY_RECEIPT_STALL_LIMIT: Duration = Duration::from_secs(30);
const RELAY_RECONNECT_BASE_DELAY_MS: u64 = 500;
const RELAY_RECONNECT_MAX_DELAY_MS: u64 = 1_000;
const RELAY_RECONNECT_JITTER_SPREAD_MS: u64 = 500;
const CLOUD_TOKEN_REFRESH_RETRY_BASE_DELAY_MS: u64 = 1_000;
const CLOUD_TOKEN_REFRESH_RETRY_MAX_DELAY_MS: u64 = 15_000;

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
    crate::runtime::cloud_relay_control::cloud_relay_transport_selected(config)
}

fn missing_relay_config_disposition(
    config: &crate::config::DaemonConfig,
) -> MissingRelayConfigDisposition {
    if crate::runtime::cloud_relay_control::cloud_relay_transport_selected(config) {
        MissingRelayConfigDisposition::CloudUnavailable
    } else {
        MissingRelayConfigDisposition::LocalIdle
    }
}

fn should_start_cloud_token_refresh(task: Option<&JoinHandle<()>>) -> bool {
    task.is_none_or(|task| task.is_finished())
}

fn should_start_leased_projection_pump(task: Option<&JoinHandle<()>>) -> bool {
    task.is_none_or(|task| task.is_finished())
}

fn should_refresh_remote_inventory(dynamic_relay: bool, managed_slice_worker: bool) -> bool {
    dynamic_relay && !managed_slice_worker
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

    fn claim_publish_if_due(&mut self, now: Instant) -> bool {
        if now < self.next_publish_at {
            return false;
        }
        self.next_publish_at = now + self.interval;
        true
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

fn relay_reconnect_delay(daemon_id: &str, attempt: u32) -> Duration {
    let capped_attempt = attempt.min(4);
    let backoff_ms = RELAY_RECONNECT_BASE_DELAY_MS.saturating_mul(1_u64 << capped_attempt);
    let bounded_backoff_ms = backoff_ms.min(RELAY_RECONNECT_MAX_DELAY_MS);
    Duration::from_millis(
        bounded_backoff_ms + stable_jitter_ms(daemon_id, RELAY_RECONNECT_JITTER_SPREAD_MS),
    )
}

/// Pause before the next token refresh after one failed; each refresh already
/// spends its own timeout, so this grows past the socket reconnect delay.
fn cloud_token_refresh_retry_delay(daemon_id: &str, failures: u32) -> Duration {
    let backoff_ms = CLOUD_TOKEN_REFRESH_RETRY_BASE_DELAY_MS
        .saturating_mul(1_u64 << failures.min(4))
        .min(CLOUD_TOKEN_REFRESH_RETRY_MAX_DELAY_MS);
    Duration::from_millis(
        backoff_ms + stable_jitter_ms(daemon_id, RELAY_RECONNECT_JITTER_SPREAD_MS),
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

async fn dynamic_relay_heartbeat_registration<F, Fut>(
    active_relay_url: &str,
    active_relay_token: &mut String,
    config: &crate::config::DaemonConfig,
    registration: F,
) -> Result<Option<chariox_relay::protocol::DaemonRegistration>, &'static str>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = chariox_relay::protocol::DaemonRegistration>,
{
    match relay_config_continuity(active_relay_url, active_relay_token, config) {
        RelayConfigContinuity::Continue => Ok(None),
        RelayConfigContinuity::Reconnect(reason) => Err(reason),
        RelayConfigContinuity::Reauthenticate => {
            let mut registration = registration().await;
            // MP-08/MP-10/MP-11: registration may read a newer config while
            // awaiting runtime metadata. Authenticate the credential belonging
            // to this continuity snapshot, then pick up further renewal on the
            // next heartbeat. Never close a live socket for that harmless race.
            registration.auth_token = config.relay_token.clone().ok_or("relay token missing")?;
            *active_relay_token = registration.auth_token.clone();
            Ok(Some(registration))
        }
    }
}

/// A burst of inbound chunks may briefly outrun the writer; wait for it rather
/// than dropping a socket that is carrying a long transfer.
async fn queue_transport_receipt(tx: &mpsc::Sender<String>, receipt: String) -> bool {
    let Err(mpsc::error::TrySendError::Full(receipt)) = tx.try_send(receipt) else {
        return true;
    };
    crate::logging::warn_with_fields(
        "daemon.relay_client",
        "relay receipt queue full; waiting for the writer",
        serde_json::json!({ "capacity": tx.max_capacity() }),
    );
    matches!(
        timeout(RELAY_RECEIPT_STALL_LIMIT, tx.send(receipt)).await,
        Ok(Ok(()))
    )
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

fn spawn_managed_slice_token_refresh(
    router: Arc<CommandRouter>,
    state: Arc<RwLock<RelayClientState>>,
    relay_url: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let refresh = async {
            let identity = router.managed_slice_relay_identity().ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "refresh slice relay token",
                    message: "slice relay bootstrap identity is missing".to_string(),
                }
            })?;
            let slice_id = identity.slice_id;
            let owner_kernel_id = identity.owner_kernel_id;
            let owner_machine_id = identity.owner_machine_id;
            let config = router.relay_config_snapshot();
            let owner_public_key = state
                .read()
                .await
                .peer_public_key(&owner_kernel_id)
                .or_else(|| router.managed_slice_relay_owner_public_key())
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "refresh slice relay token",
                    message: "slice owner relay key is not known".to_string(),
                })?;
            let response = super::peer_client::send_peer_request_to_known_kernel_via_relay(
                &config,
                &state,
                chariox_relay::protocol::ClientTarget {
                    daemon_id: Some(owner_kernel_id.clone()),
                    daemon_alias: None,
                },
                &owner_public_key,
                crate::transport::relay_peer::RelayPeerRequest::RefreshManagedSliceRelayToken {
                    slice_id: slice_id.clone(),
                    owner_kernel_id: owner_kernel_id.clone(),
                    worker_kernel_id: config.daemon_id.clone(),
                },
            )
            .await?;
            match response {
                crate::transport::relay_peer::RelayPeerResponse::ManagedSliceRelayTokenRefreshed {
                    slice_id: refreshed_slice_id,
                    relay_token,
                    expires_at_ms,
                    relay_recovery_token,
                    recovery_expires_at_ms,
                    relay_peer_protocol_version,
                } if refreshed_slice_id == slice_id
                    && relay_peer_protocol_version
                        >= crate::transport::relay_peer::MINIMUM_RELAY_PEER_RUNTIME_VERSION =>
                {
                    router
                        .install_managed_slice_relay_token(
                            crate::runtime::router::ManagedSliceRelayTokenInstallRequest {
                                slice_id: slice_id.clone(),
                                owner_kernel_id: owner_kernel_id.clone(),
                                owner_machine_id: owner_machine_id.clone(),
                                relay_token: relay_token.into_inner(),
                                expires_at_ms,
                                relay_recovery_token: relay_recovery_token.into_inner(),
                                recovery_expires_at_ms,
                                owner_public_key,
                            },
                        )
                        .await
                }
                crate::transport::relay_peer::RelayPeerResponse::ManagedSliceRelayTokenFailed {
                    code,
                    retryable,
                } => Err(DaemonError::LocalTransport {
                    operation: "refresh slice relay token",
                    message: format!(
                        "slice owner rejected relay token refresh ({code}, retryable={retryable})"
                    ),
                }),
                _ => Err(DaemonError::LocalTransport {
                    operation: "refresh slice relay token",
                    message: "slice owner returned an incompatible relay token response"
                        .to_string(),
                }),
            }
        };
        match bounded_cloud_relay_refresh(refresh, CLOUD_RELAY_TOKEN_REFRESH_TIMEOUT).await {
            CloudRelayRefreshResult::Refreshed => {
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "managed slice relay token refresh completed",
                    serde_json::json!({ "relay_url": relay_url }),
                );
            }
            CloudRelayRefreshResult::Failed(error) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "failed to refresh managed slice relay token",
                    serde_json::json!({ "relay_url": relay_url, "error": error }),
                );
            }
            CloudRelayRefreshResult::TimedOut => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "managed slice relay token refresh timed out",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "timeout_ms": CLOUD_RELAY_TOKEN_REFRESH_TIMEOUT.as_millis(),
                    }),
                );
            }
        }
    })
}

// MP-08/MP-10/MP-11: reject an obsolete home before settling token confirmation.
fn spawn_leased_projection_pump(
    router: Arc<CommandRouter>,
    state: Arc<RwLock<RelayClientState>>,
    outgoing_tx: RelayOutgoingSender,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        await_leased_projection_pump(
            pump_leased_projection_events(&router, &state, &outgoing_tx),
            RELAY_HEARTBEAT_APP_WORK_SLOW_THRESHOLD,
        )
        .await;
    })
}

async fn await_leased_projection_pump<F>(pump: F, slow_threshold: Duration)
where
    F: Future<Output = ()>,
{
    let started = Instant::now();
    pump.await;
    let elapsed = started.elapsed();
    if elapsed >= slow_threshold {
        crate::logging::warn_with_fields(
            "daemon.relay_client",
            "leased projection pump completed slowly",
            serde_json::json!({
                "elapsed_ms": elapsed.as_millis(),
                "slow_threshold_ms": slow_threshold.as_millis(),
            }),
        );
    }
}

async fn disconnect_relay(
    state: &Arc<RwLock<RelayClientState>>,
    reason: &str,
    presence: Option<&CloudPresencePublisher>,
) {
    if let Some(presence) = presence {
        publish_offline_and_set_disconnected(presence, state, reason).await;
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
    router: Arc<CommandRouter>,
    state: Arc<RwLock<RelayClientState>>,
    shutdown: &mut watch::Receiver<bool>,
    static_relay: Option<StaticRelayConfig>,
) {
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
    let local_browser = LocalBrowserDirect::new(
        Arc::clone(&router),
        Arc::clone(&command_sequence),
        Arc::clone(&event_runtime),
        Arc::clone(&command_result_cache),
        shutdown.clone(),
    );
    let presence = static_relay
        .is_none()
        .then(|| CloudPresencePublisher::start(Arc::clone(&router)));
    let mut missing_relay_config_reported = false;
    let mut reconnect_attempt = 0_u32;
    let mut token_refresh_failures = 0_u32;

    loop {
        if *shutdown.borrow() {
            disconnect_relay(&state, "shutdown before relay connect", presence.as_ref()).await;
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
            let config = router.relay_config_snapshot();
            if cloud_relay_token_is_expired(&config, crate::session::unix_epoch_ms()) {
                let daemon_id = router.relay_daemon_id();
                let delay = cloud_token_refresh_retry_delay(&daemon_id, token_refresh_failures);
                let relay_url = config
                    .cloud_relay
                    .as_ref()
                    .map(|profile| profile.relay_url.clone())
                    .unwrap_or_default();
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "cloud relay token is expired and could not be renewed; retrying the renewal",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "retry_ms": delay.as_millis(),
                        "failures": token_refresh_failures,
                    }),
                );
                if token_refresh_failures == 0 {
                    disconnect_relay(&state, "cloud relay token expired", presence.as_ref()).await;
                }
                record_relay_reconnect(&router, &relay_url, "cloud relay token expired", delay);
                token_refresh_failures = token_refresh_failures.saturating_add(1);
                if wait_for_reconnect_delay(shutdown, delay).await {
                    return;
                }
                continue;
            }
            token_refresh_failures = 0;
            match router
                .activate_managed_slice_recovery_connection(crate::session::unix_epoch_ms())
                .await
            {
                Ok(true) => crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "activated managed slice relay recovery credential",
                    serde_json::json!({}),
                ),
                Ok(false) => {}
                Err(error) => crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "managed slice relay recovery credential is unavailable",
                    serde_json::json!({ "error": error.to_string() }),
                ),
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
                                        &state,
                                        "relay configuration unavailable",
                                        presence.as_ref(),
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
                                        &state,
                                        "shutdown while relay configuration unavailable",
                                        presence.as_ref(),
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
        match timeout(
            RELAY_CONNECT_TIMEOUT,
            crate::transport::relay_socket_connect::connect_binary_events(&relay_url),
        )
        .await
        {
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
                disconnect_relay(&state, "relay connect timed out", presence.as_ref()).await;
                let daemon_id = router.relay_daemon_id();
                let delay = relay_reconnect_delay(&daemon_id, reconnect_attempt);
                record_relay_reconnect(&router, &relay_url, "relay connect timed out", delay);
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                if wait_for_reconnect_delay(shutdown, delay).await {
                    return;
                }
                continue;
            }
            Ok(Ok((socket, handshake))) => {
                let binary_events = handshake
                    .headers()
                    .get(chariox_relay::binary_event::VERSION_HEADER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u32>().ok())
                    .is_some_and(|version| {
                        version >= chariox_relay::binary_event::PROTOCOL_VERSION
                    });
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
                let (writer, mut reader) = socket.split();
                let (outgoing_tx, priority_outgoing_rx, event_outgoing_rx) =
                    RelayOutgoingSender::channel(RELAY_OUTGOING_QUEUE_LIMIT);
                let reconnect_gate = Arc::new(RelayReconnectGate::default());
                let (pong_tx, pong_rx) = mpsc::channel::<Vec<u8>>(RELAY_OUTGOING_QUEUE_LIMIT);
                let (transport_receipt_tx, transport_receipt_rx) =
                    mpsc::channel::<String>(chariox_relay::frame_transport::MAX_UNACKED_FRAMES);
                let mut relay_frames = chariox_relay::frame_transport::FrameReceiver::default();
                let (writer_done_tx, mut writer_done_rx) = oneshot::channel::<()>();
                let writer_closing = Arc::clone(&outgoing_tx.closing);
                let writer_task = tokio::spawn(async move {
                    writer::run_relay_writer(
                        writer,
                        priority_outgoing_rx,
                        event_outgoing_rx,
                        pong_rx,
                        transport_receipt_rx,
                        binary_events,
                        writer_closing,
                    )
                    .await;
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
                        &state,
                        "failed to send relay registration",
                        presence.as_ref(),
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
                let replayed_display_tunnel_count =
                    match set_connected(&state, outgoing_tx.clone(), relay_url.clone()).await {
                        Ok(count) => count,
                        Err(error) => {
                            writer_task.abort();
                            clear_remote_inventory_projection(&router);
                            disconnect_relay(&state, &error, presence.as_ref()).await;
                            let delay = relay_reconnect_delay(&daemon_id, reconnect_attempt);
                            record_relay_reconnect(&router, &relay_url, &error, delay);
                            reconnect_attempt = reconnect_attempt.saturating_add(1);
                            if wait_for_reconnect_delay(shutdown, delay).await {
                                return;
                            }
                            continue;
                        }
                    };
                router
                    .transport_health_store()
                    .record_relay_connected(&relay_url);
                router
                    .runtime_state()
                    .retry_pending_slice_observation_revocations();
                let mut slice_activation = ManagedSliceActivationConfirmation::default();
                if static_relay.is_none()
                    && socket_has_current_token(
                        router.relay_config_snapshot().relay_token.as_deref(),
                        &active_relay_token,
                    )
                {
                    slice_activation.resume_pending(&router, &state).await;
                }
                if replayed_display_tunnel_count > 0 {
                    crate::logging::info_with_fields(
                        "daemon.relay_client",
                        "replayed display tunnels after relay reconnect",
                        serde_json::json!({
                            "relay_url": relay_url,
                            "display_tunnel_count": replayed_display_tunnel_count,
                        }),
                    );
                }
                reconnect_attempt = 0;
                if let Some(presence) = presence.as_ref() {
                    presence.publish(true, "relay registration sent");
                }
                let mut cloud_presence_schedule =
                    CloudPresencePublishSchedule::new(Instant::now(), &daemon_id);
                let refresh_remote_inventory = should_refresh_remote_inventory(
                    static_relay.is_none(),
                    router.managed_slice_relay_identity().is_some(),
                );
                let mut inventory_refresh_task = refresh_remote_inventory
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
                                // A hot reload is back within about a second: the relay
                                // tells this kernel's browsers to retry soon.
                                let _ = send_outgoing_envelope(&outgoing_tx, RelayEnvelope::Close {
                                    reason: "daemon shutting down".to_string(),
                                    kernel_reload: crate::hot_reload::handing_over(),
                                });
                                sleep(Duration::from_millis(25)).await;
                                abort_leased_projection_pump_task(&mut leased_projection_pump_task);
                                abort_inventory_refresh_task(&mut inventory_refresh_task);
                                abort_subscription_tasks(&router, &subscription_tasks).await;
                                writer_task.abort();
                                clear_remote_inventory_projection(&router);
                                disconnect_relay(&state, "daemon shutting down", presence.as_ref()).await;
                                return;
                            }
                        }
                        incoming = reader.next() => {
                            match incoming {
                                Some(Ok(Message::Text(payload))) => {
                                    let (payload, receipt) = relay_frames.receive(&payload).unwrap_or_else(|_| (
                                        Some("{\"kind\":\"close\",\"reason\":\"invalid relay transport frame\"}".to_string()), None));
                                    if let Some(receipt) = receipt {
                                        if !queue_transport_receipt(&transport_receipt_tx, receipt).await {
                                            abort_leased_projection_pump_task(
                                                &mut leased_projection_pump_task,
                                            );
                                            abort_inventory_refresh_task(&mut inventory_refresh_task);
                                            abort_subscription_tasks(&router, &subscription_tasks).await;
                                            writer_task.abort();
                                            clear_remote_inventory_projection(&router);
                                            disconnect_relay(&state, "relay receipt writer stalled", presence.as_ref()).await;
                                            break "relay receipt writer stalled";
                                        }
                                    }
                                    let Some(payload) = payload else { continue; };

                                    if let Err(error) = handle_incoming_envelope(
                                        IncomingEnvelopeContext {
                                            router: &router,
                                            command_sequence: &command_sequence,
                                            state: &state,
                                            outgoing_tx: &outgoing_tx,
                                            subscription_tasks: &subscription_tasks,
                                            event_runtime: &event_runtime,
                                            command_result_cache: &command_result_cache,
                                            reconnect_gate: &reconnect_gate,
                                            local_browser: &local_browser,
                                        },
                                        static_relay.is_none().then_some((
                                            relay_url.as_str(),
                                            active_relay_token.as_str(),
                                        )),
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
                                        abort_subscription_tasks(&router, &subscription_tasks).await;
                                        writer_task.abort();
                                        clear_remote_inventory_projection(&router);
                                        disconnect_relay(
                                            &state,
                                            &disconnect_reason,
                                            presence.as_ref(),
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
                                    abort_subscription_tasks(&router, &subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&router);
                                    disconnect_relay(&state, "relay close frame received", presence.as_ref()).await;
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
                                    abort_subscription_tasks(&router, &subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&router);
                                    disconnect_relay(&state, "relay read failed or ended", presence.as_ref()).await;
                                    break "relay read failed or ended";
                                }
                            }
                        }
                        writer_done = &mut writer_done_rx => {
                            let _ = writer_done;
                            abort_leased_projection_pump_task(&mut leased_projection_pump_task);
                            abort_inventory_refresh_task(&mut inventory_refresh_task);
                            abort_subscription_tasks(&router, &subscription_tasks).await;
                            writer_task.abort();
                            clear_remote_inventory_projection(&router);
                            disconnect_relay(&state, "relay writer ended", presence.as_ref()).await;
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
                            } else if router.managed_slice_relay_token_refresh_due()
                                && should_start_cloud_token_refresh(token_refresh_task.as_ref())
                            {
                                token_refresh_task = Some(spawn_managed_slice_token_refresh(
                                    Arc::clone(&router),
                                    Arc::clone(&state),
                                    relay_url.clone(),
                                ));
                            }
                            let config = router.relay_config_snapshot();
                            let continuity = dynamic_relay_heartbeat_registration(
                                &relay_url,
                                &mut active_relay_token,
                                &config,
                                || router.relay_registration(),
                            )
                            .await;
                            let reconnect_reason = match continuity {
                                Ok(None) => None,
                                Ok(Some(registration)) => {
                                    let reauthentication = RelayEnvelope::DaemonHeartbeat {
                                        daemon_id: daemon_id.clone(),
                                        registration: Some(registration),
                                    };
                                    if send_outgoing_envelope(&outgoing_tx, reauthentication)
                                        .is_err()
                                    {
                                        Some("relay reauthentication send failed")
                                    } else {
                                        crate::logging::info_with_fields(
                                            "daemon.relay_client",
                                            "relay socket reauthentication queued",
                                            serde_json::json!({
                                                "relay_url": relay_url,
                                                "phase": "token_refresh",
                                            }),
                                        );
                                        if let Some(presence) = presence.as_ref() {
                                            presence.publish(true, "relay reauthenticated");
                                        }
                                        None
                                    }
                                }
                                Err(reason) => Some(reason),
                            };
                            if let Some(reason) = reconnect_reason {
                                crate::logging::warn_with_fields(
                                    "daemon.relay_client",
                                    "relay socket reconnect requested",
                                    serde_json::json!({
                                        "relay_url": relay_url,
                                        "reason": reason,
                                        "phase": "token_refresh",
                                    }),
                                );
                                let _ = reconnect_gate.request_reconnect(&outgoing_tx, reason);
                            }
                        }
                        _ = heartbeat_interval.tick() => {
                            heartbeat_tick = heartbeat_tick.wrapping_add(1);
                            let mut heartbeat_registration = None;
                            if static_relay.is_none() {
                                let config = router.relay_config_snapshot();
                                match dynamic_relay_heartbeat_registration(
                                    &relay_url,
                                    &mut active_relay_token,
                                    &config,
                                    || router.relay_registration(),
                                )
                                .await
                                {
                                    Ok(registration) => {
                                        heartbeat_registration = registration;
                                    }
                                    Err(reason) => {
                                        crate::logging::warn_with_fields(
                                            "daemon.relay_client",
                                            "relay socket reconnect requested",
                                            serde_json::json!({
                                                "relay_url": relay_url,
                                                "reason": reason,
                                                "phase": "heartbeat",
                                            }),
                                        );
                                        // Keep heartbeats flowing while in-flight work drains.
                                        let _ = reconnect_gate.request_reconnect(&outgoing_tx, reason);
                                    }
                                }
                                if heartbeat_registration.is_some() {
                                    crate::logging::info_with_fields(
                                        "daemon.relay_client",
                                        "relay socket reauthentication queued",
                                        serde_json::json!({
                                            "relay_url": relay_url,
                                            "phase": "heartbeat",
                                        }),
                                    );
                                    if let Some(presence) = presence.as_ref() {
                                        presence.publish(true, "relay reauthenticated");
                                    }
                                }
                            }
                            if refresh_remote_inventory
                                && remote_inventory_refresh_due(
                                    heartbeat_tick,
                                    &mut inventory_refresh_task,
                                )
                            {
                                inventory_refresh_task = Some(
                                    spawn_remote_inventory_projection_refresh(
                                        Arc::clone(&router),
                                    )
                                );
                            }
                            let heartbeat_frame = RelayEnvelope::DaemonHeartbeat {
                                daemon_id: daemon_id.clone(),
                                registration: heartbeat_registration,
                            };
                            if send_outgoing_envelope(&outgoing_tx, heartbeat_frame).is_err() {
                                abort_leased_projection_pump_task(&mut leased_projection_pump_task);
                                abort_inventory_refresh_task(&mut inventory_refresh_task);
                                abort_subscription_tasks(&router, &subscription_tasks).await;
                                writer_task.abort();
                                clear_remote_inventory_projection(&router);
                                disconnect_relay(&state, "relay heartbeat send failed", presence.as_ref()).await;
                                break "relay heartbeat send failed";
                            }
                            // Same-identity token installation reauthenticates in place.
                            // Its pending confirmation must run after this heartbeat too,
                            // without waiting for a socket reconnect or duplicating retries.
                            // A token installed after this tick's snapshot waits for the
                            // next heartbeat, which carries it; confirm only on a socket
                            // authenticated with the token the owner is told to verify.
                            if static_relay.is_none()
                                && socket_has_current_token(
                                    router.relay_config_snapshot().relay_token.as_deref(),
                                    &active_relay_token,
                                )
                            {
                                slice_activation.resume_pending(&router, &state).await;
                            }
                            if should_start_leased_projection_pump(
                                leased_projection_pump_task.as_ref(),
                            ) {
                                leased_projection_pump_task = Some(spawn_leased_projection_pump(
                                    Arc::clone(&router),
                                    Arc::clone(&state),
                                    outgoing_tx.clone(),
                                ));
                            }
                            if let Some(presence) = presence.as_ref() {
                                if cloud_presence_schedule.claim_publish_if_due(Instant::now()) {
                                    presence.publish(true, "relay heartbeat");
                                }
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
                disconnect_relay(&state, "relay socket connect failed", presence.as_ref()).await;
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
mod tests;
