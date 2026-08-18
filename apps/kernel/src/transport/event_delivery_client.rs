use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chariox_event_protocol::{
    AedsToKernelMessage, KernelToAedsMessage, EVENT_DELIVERY_PROTOCOL_VERSION,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::Message;

use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::runtime::state::KernelRuntimeState;

#[derive(Clone)]
pub(crate) struct EventDeliveryClientConfig {
    pub url: Option<String>,
    pub token: Option<String>,
    pub kernel_id: String,
    pub environment_id: String,
    pub generator_management_targets:
        BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
    pub config_projection: DaemonConfigProjectionStore,
}

#[derive(Debug, Clone, Default)]
struct EventDeliveryConnectionHealth {
    connected: bool,
    last_connected_at_ms: Option<u64>,
    last_error: Option<String>,
}

static EVENT_DELIVERY_HEALTH: OnceLock<Mutex<EventDeliveryConnectionHealth>> = OnceLock::new();

struct DeliveryAcceptanceRequest {
    delivery: chariox_event_protocol::EventDeliveryEnvelope,
    result_tx: mpsc::UnboundedSender<DeliveryAcceptanceResult>,
}

type DeliveryAcceptanceResult = (String, Duration, Result<(), String>);
type DeliveryAcceptor =
    Arc<dyn Fn(chariox_event_protocol::EventDeliveryEnvelope) -> Result<(), String> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryQueueError {
    Closed,
    Full,
}

#[derive(Clone)]
struct DeliveryAcceptanceQueue {
    sender: mpsc::Sender<DeliveryAcceptanceRequest>,
}

impl DeliveryAcceptanceQueue {
    fn new(runtime_state: KernelRuntimeState) -> Self {
        Self::with_acceptor(Arc::new(move |delivery| {
            runtime_state
                .accept_workflow_event_delivery(delivery)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }))
    }

    fn with_acceptor(acceptor: DeliveryAcceptor) -> Self {
        let (sender, mut receiver) = mpsc::channel::<DeliveryAcceptanceRequest>(32);
        tokio::spawn(async move {
            while let Some(request) = receiver.recv().await {
                let delivery_id = request.delivery.delivery_id.clone();
                let started_at = Instant::now();
                let acceptor = acceptor.clone();
                let result = tokio::task::spawn_blocking(move || acceptor(request.delivery))
                    .await
                    .map_err(|error| format!("event delivery worker failed: {error}"))
                    .and_then(|result| result);
                let _ = request
                    .result_tx
                    .send((delivery_id, started_at.elapsed(), result));
            }
        });
        Self { sender }
    }

    fn try_enqueue(
        &self,
        delivery: chariox_event_protocol::EventDeliveryEnvelope,
        result_tx: mpsc::UnboundedSender<DeliveryAcceptanceResult>,
    ) -> Result<(), DeliveryQueueError> {
        self.sender
            .try_send(DeliveryAcceptanceRequest {
                delivery,
                result_tx,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Closed(_) => DeliveryQueueError::Closed,
                mpsc::error::TrySendError::Full(_) => DeliveryQueueError::Full,
            })
    }
}

pub(crate) fn event_delivery_status(
    runtime_state: &KernelRuntimeState,
    config: &crate::config::DaemonConfig,
) -> crate::local::EventDeliveryStatus {
    let health = EVENT_DELIVERY_HEALTH
        .get_or_init(|| Mutex::new(EventDeliveryConnectionHealth::default()))
        .lock()
        .expect("event delivery health lock should not be poisoned")
        .clone();
    crate::local::EventDeliveryStatus {
        configured: config.event_delivery_url.is_some(),
        connected: health.connected,
        aeds_url: config.event_delivery_url.clone(),
        last_connected_at_ms: health.last_connected_at_ms,
        last_error: health.last_error,
        active_route_count: runtime_state
            .active_event_route_claims(&config.daemon_id)
            .len(),
    }
}

pub(crate) async fn run_event_delivery_connector(
    runtime_state: KernelRuntimeState,
    config: EventDeliveryClientConfig,
    shutdown: watch::Receiver<bool>,
) {
    let delivery_queue = DeliveryAcceptanceQueue::new(runtime_state.clone());
    run_event_delivery_connector_with_queue(runtime_state, config, shutdown, delivery_queue).await;
}

async fn run_event_delivery_connector_with_queue(
    runtime_state: KernelRuntimeState,
    config: EventDeliveryClientConfig,
    mut shutdown: watch::Receiver<bool>,
    delivery_queue: DeliveryAcceptanceQueue,
) {
    let Some(url) = config.url.clone() else {
        crate::logging::info_with_fields(
            "daemon.event_delivery",
            "event delivery is ready with no AEDS configured",
            serde_json::json!({
                "phase": "event_ready",
                "configured": false,
            }),
        );
        let _ = shutdown.changed().await;
        return;
    };
    let mut retry = Duration::from_secs(1);
    // This worker is deliberately owned by the connector, not by an individual
    // WebSocket connection. A reconnect must not create a second acceptance
    // worker that can race receipt persistence for the same delivery.
    let mut aegs_reconciliation = tokio::time::interval(Duration::from_secs(30));
    aegs_reconciliation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if *shutdown.borrow() {
            return;
        }
        if let Err(error) = reconcile_aegs_subscriptions(&runtime_state, &config).await {
            crate::logging::warn_with_fields(
                "daemon.event_generation",
                "AEGS subscription reconciliation failed",
                serde_json::json!({"error": error}),
            );
        }
        match connect_once(
            &runtime_state,
            &config,
            &url,
            shutdown.clone(),
            &delivery_queue,
        )
        .await
        {
            Ok(()) => retry = Duration::from_secs(1),
            Err(error) => {
                record_disconnected(Some(error.clone()));
                crate::logging::warn_with_fields(
                    "daemon.event_delivery",
                    "AEDS connection ended",
                    serde_json::json!({
                        "url": url,
                        "error": error,
                        "retry_ms": retry.as_millis(),
                    }),
                );
            }
        }
        tokio::select! {
            _ = shutdown.changed() => return,
            _ = tokio::time::sleep(retry) => {}
            _ = aegs_reconciliation.tick() => {
                if let Err(error) = reconcile_aegs_subscriptions(&runtime_state, &config).await {
                    crate::logging::warn_with_fields(
                        "daemon.event_generation",
                        "AEGS subscription reconciliation failed",
                        serde_json::json!({"error": error}),
                    );
                }
            }
        }
        retry = (retry * 2).min(Duration::from_secs(30));
    }
}

async fn connect_once(
    runtime_state: &KernelRuntimeState,
    config: &EventDeliveryClientConfig,
    url: &str,
    mut shutdown: watch::Receiver<bool>,
    delivery_queue: &DeliveryAcceptanceQueue,
) -> Result<(), String> {
    let mut request = url
        .into_client_request()
        .map_err(|error| format!("invalid AEDS URL: {error}"))?;
    if let Some(token) = config.token.as_deref() {
        request.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .map_err(|error| format!("invalid AEDS token header: {error}"))?,
        );
    }
    let (websocket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|error| format!("failed to connect to AEDS: {error}"))?;
    let (mut sink, mut stream) = websocket.split();
    let mut environments =
        runtime_state.event_delivery_resumes(&config.kernel_id, &config.environment_id);
    let mut route_signature =
        serde_json::to_string(&environments).map_err(|error| error.to_string())?;
    send(
        &mut sink,
        &KernelToAedsMessage::Hello {
            protocol_version: EVENT_DELIVERY_PROTOCOL_VERSION,
            kernel_id: config.kernel_id.clone(),
            environments: environments.clone(),
        },
    )
    .await?;
    let mut reconciliation = tokio::time::interval(Duration::from_secs(5));
    reconciliation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut aegs_reconciliation = tokio::time::interval(Duration::from_secs(30));
    aegs_reconciliation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut kernel_heartbeat = tokio::time::interval(Duration::from_secs(5));
    kernel_heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut aeds_ready = false;
    let mut event_ready_logged = false;
    let (delivery_result_tx, mut delivery_result_rx) =
        mpsc::unbounded_channel::<(String, Duration, Result<(), String>)>();
    let mut pending_delivery_ids = std::collections::BTreeSet::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => return Ok(()),
            message = stream.next() => {
                let Some(message) = message else {
                    return Err("AEDS closed the connection".to_string());
                };
                let message = message.map_err(|error| error.to_string())?;
                if message.is_close() {
                    return Err("AEDS closed the connection".to_string());
                }
                let text = message.to_text().map_err(|error| error.to_string())?;
                let message: AedsToKernelMessage =
                    serde_json::from_str(text).map_err(|error| format!("invalid AEDS message: {error}"))?;
                match message {
                    AedsToKernelMessage::HelloAccepted { protocol_version, heartbeat_interval_ms } => {
                        if protocol_version != EVENT_DELIVERY_PROTOCOL_VERSION {
                            return Err(format!("AEDS negotiated unsupported protocol {protocol_version}"));
                        }
                        let server_interval = Duration::from_millis(heartbeat_interval_ms.max(1_000));
                        let client_interval = (server_interval / 3).clamp(
                            Duration::from_secs(1),
                            Duration::from_secs(10),
                        );
                        kernel_heartbeat = tokio::time::interval(client_interval);
                        kernel_heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                        kernel_heartbeat.reset();
                        aeds_ready = true;
                        crate::logging::info_with_fields(
                            "daemon.event_delivery",
                            "AEDS connection established",
                            serde_json::json!({"url": url}),
                        );
                        record_connected();
                    }
                    AedsToKernelMessage::RoutesReconciled { conflicts, .. } => {
                        runtime_state.apply_event_route_conflicts(&conflicts);
                        if !event_ready_logged {
                            event_ready_logged = true;
                            crate::logging::info_with_fields(
                                "daemon.event_delivery",
                                "event delivery routes reconciled",
                                serde_json::json!({
                                    "phase": "event_ready",
                                    "configured": true,
                                    "url": url,
                                }),
                            );
                        }
                        for conflict in conflicts {
                            crate::logging::warn_with_fields(
                                "daemon.event_delivery",
                                "AEDS rejected a duplicate environment route",
                                serde_json::json!({
                                    "environment_id": conflict.environment_id,
                                    "event_interest_key": conflict.event_interest_key,
                                    "requested_binding_id": conflict.requested_binding_id,
                                    "existing_binding_id": conflict.existing_binding_id,
                                    "existing_publication_id": conflict.existing_publication_id,
                                }),
                            );
                        }
                    }
                    AedsToKernelMessage::Delivery { delivery } => {
                        if !pending_delivery_ids.insert(delivery.delivery_id.clone()) {
                            crate::logging::debug_with_fields(
                                "daemon.event_delivery",
                                "ignoring duplicate in-flight AEDS delivery",
                                serde_json::json!({"delivery_id": delivery.delivery_id}),
                            );
                            continue;
                        }
                        if let Err(error) = delivery_queue
                            .try_enqueue(delivery.clone(), delivery_result_tx.clone())
                        {
                            pending_delivery_ids.remove(&delivery.delivery_id);
                            match error {
                                DeliveryQueueError::Closed => {
                                    return Err("event delivery worker stopped".to_string());
                                }
                                DeliveryQueueError::Full => {
                                    crate::logging::warn_with_fields(
                                        "daemon.event_delivery",
                                        "event delivery acceptance queue is full; leaving delivery unacknowledged",
                                        serde_json::json!({
                                            "delivery_id": delivery.delivery_id,
                                        }),
                                    );
                                }
                            }
                        }
                    }
                    AedsToKernelMessage::Heartbeat { at_ms } => {
                        send(&mut sink, &KernelToAedsMessage::Heartbeat { at_ms }).await?;
                    }
                    AedsToKernelMessage::Error { code, message, retryable } => {
                        if !retryable {
                            return Err(format!("AEDS error {code}: {message}"));
                        }
                        crate::logging::warn_with_fields(
                            "daemon.event_delivery",
                            "retryable AEDS error",
                            serde_json::json!({"code": code, "message": message}),
                        );
                    }
                }
            }
            Some((delivery_id, elapsed, result)) = delivery_result_rx.recv() => {
                pending_delivery_ids.remove(&delivery_id);
                match result {
                    Ok(_) => {
                        crate::logging::info_with_fields(
                            "daemon.event_delivery",
                            "event delivery durably accepted",
                            serde_json::json!({
                                "delivery_id": delivery_id,
                                "acceptance_ms": elapsed.as_millis(),
                            }),
                        );
                        send(&mut sink, &KernelToAedsMessage::Ack {
                            delivery_id,
                        }).await?;
                    }
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "daemon.event_delivery",
                            "AEDS delivery was not acknowledged",
                            serde_json::json!({
                                "delivery_id": delivery_id,
                                "error": error,
                                "acceptance_ms": elapsed.as_millis(),
                            }),
                        );
                    }
                }
            }
            _ = reconciliation.tick() => {
                environments = runtime_state.event_delivery_resumes(
                    &config.kernel_id,
                    &config.environment_id,
                );
                let next_signature =
                    serde_json::to_string(&environments).map_err(|error| error.to_string())?;
                if next_signature != route_signature {
                    send(
                        &mut sink,
                        &KernelToAedsMessage::ReconcileRoutes {
                            environments: environments.clone(),
                        },
                    )
                    .await?;
                    route_signature = next_signature;
                }
            }
            _ = aegs_reconciliation.tick() => {
                if let Err(error) = reconcile_aegs_subscriptions(runtime_state, config).await {
                    crate::logging::warn_with_fields(
                        "daemon.event_generation",
                        "AEGS subscription reconciliation failed",
                        serde_json::json!({"error": error}),
                    );
                }
            }
            _ = kernel_heartbeat.tick(), if aeds_ready => {
                send(&mut sink, &KernelToAedsMessage::Heartbeat {
                    at_ms: crate::session::unix_epoch_ms(),
                }).await?;
            }
        }
    }
}

async fn reconcile_aegs_subscriptions(
    runtime_state: &KernelRuntimeState,
    config: &EventDeliveryClientConfig,
) -> Result<(), String> {
    let mut generator_management_targets = config.generator_management_targets.clone();
    let generator_ids = runtime_state
        .event_generator_subscription_claims()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for generator_id in generator_ids {
        let current_config = config.config_projection.snapshot();
        let request = crate::local::LocalDaemonRequest::ListEventConnections(
            crate::local::ListEventConnectionsRequest {
                generator_id: Some(generator_id),
                cursor: None,
                limit: 1,
            },
        );
        let targets =
            crate::runtime::event_catalog_control::resolve_event_generator_management_targets(
                runtime_state,
                &config.config_projection,
                &current_config,
                current_config
                    .cloud_relay
                    .as_ref()
                    .map(|profile| profile.user_id.as_str())
                    .unwrap_or("kernel"),
                &request,
            )
            .await
            .map_err(|error| error.to_string())?;
        generator_management_targets.extend(targets);
    }
    if generator_management_targets.is_empty() {
        return Ok(());
    }
    let kernel_owner_id = config.kernel_id.clone();
    let mut claims = runtime_state.event_generator_subscription_claims();
    for generator_id in generator_management_targets.keys() {
        let request = chariox_event_protocol::AegsSubscriptionReconcileRequest {
            owner_id: config.kernel_id.clone(),
            generator_id: generator_id.clone(),
            subscriptions: claims.remove(generator_id).unwrap_or_default(),
        };
        let target =
            crate::runtime::event_catalog_control::select_event_generator_management_target(
                &generator_management_targets,
                generator_id,
                &request.owner_id,
            )
            .map_err(|error| error.to_string())?;
        let generator_id = generator_id.clone();
        let kernel_owner_id = kernel_owner_id.clone();
        tokio::task::spawn_blocking(move || {
            let url = format!("{}/v1/subscriptions/reconcile", target.url);
            let encoded = serde_json::to_string(&request).map_err(|error| error.to_string())?;
            let response =
                crate::runtime::event_catalog_control::aegs_management_agent_builder(&target)
                    .timeout_connect(Duration::from_secs(3))
                    .timeout_read(Duration::from_secs(10))
                    .timeout_write(Duration::from_secs(10))
                    .build()
                    .put(&url)
                    .set("authorization", &format!("Bearer {}", target.token))
                    .set("x-chariox-owner-id", &kernel_owner_id)
                    .set("content-type", "application/json")
                    .send_string(&encoded)
                    .map_err(|error| {
                        format!("AEGS {generator_id} reconciliation request failed: {error}")
                    })?;
            let response_body = response.into_string().map_err(|error| error.to_string())?;
            let response: chariox_event_protocol::AegsSubscriptionReconcileResponse =
                serde_json::from_str(&response_body)
                    .map_err(|error| format!("AEGS {generator_id} response is invalid: {error}"))?;
            if !response.authoritative {
                return Err(format!(
                    "AEGS {generator_id} did not accept authoritative reconciliation"
                ));
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|error| error.to_string())??;
    }
    Ok(())
}

fn record_connected() {
    let mut health = EVENT_DELIVERY_HEALTH
        .get_or_init(|| Mutex::new(EventDeliveryConnectionHealth::default()))
        .lock()
        .expect("event delivery health lock should not be poisoned");
    health.connected = true;
    health.last_connected_at_ms = Some(crate::session::unix_epoch_ms());
    health.last_error = None;
}

fn record_disconnected(error: Option<String>) {
    let mut health = EVENT_DELIVERY_HEALTH
        .get_or_init(|| Mutex::new(EventDeliveryConnectionHealth::default()))
        .lock()
        .expect("event delivery health lock should not be poisoned");
    health.connected = false;
    health.last_error = error;
}

async fn send<S>(sink: &mut S, message: &KernelToAedsMessage) -> Result<(), String>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let encoded = serde_json::to_string(message).map_err(|error| error.to_string())?;
    sink.send(Message::Text(encoded.into()))
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery(delivery_id: &str) -> chariox_event_protocol::EventDeliveryEnvelope {
        chariox_event_protocol::EventDeliveryEnvelope {
            delivery_id: delivery_id.to_string(),
            binding_id: "binding-1".to_string(),
            event_type: "test.event".to_string(),
            event_type_version: 1,
            occurrence_id: format!("occurrence-{delivery_id}"),
            occurred_at: "2026-08-15T00:00:00Z".to_string(),
            prompt: "test".to_string(),
            artifacts: Vec::new(),
            metadata: serde_json::Value::Null,
            reply_context: None,
            expires_at_ms: u64::MAX,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn acceptance_worker_serializes_reconnect_retry_and_ack_after_first_acceptance() {
        let calls = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let persisted_delivery_count = Arc::new(AtomicUsize::new(0));
        let accepted_delivery_ids =
            Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new()));
        let acceptor: DeliveryAcceptor = {
            let calls = calls.clone();
            let active = active.clone();
            let max_active = max_active.clone();
            let persisted_delivery_count = persisted_delivery_count.clone();
            let accepted_delivery_ids = accepted_delivery_ids.clone();
            Arc::new(move |delivery| {
                calls.fetch_add(1, Ordering::SeqCst);
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_active.fetch_max(current, Ordering::SeqCst);
                if calls.load(Ordering::SeqCst) == 1 {
                    std::thread::sleep(Duration::from_millis(100));
                }
                if accepted_delivery_ids
                    .lock()
                    .expect("accepted delivery set should not be poisoned")
                    .insert(delivery.delivery_id)
                {
                    // Model the receipt-plus-enqueue transaction that the real
                    // acceptance function commits before its caller can ACK.
                    persisted_delivery_count.fetch_add(1, Ordering::SeqCst);
                }
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
        };
        let queue = DeliveryAcceptanceQueue::with_acceptor(acceptor);

        let (first_result_tx, first_result_rx) = mpsc::unbounded_channel();
        queue
            .try_enqueue(delivery("first"), first_result_tx)
            .expect("first delivery should enqueue");
        drop(first_result_rx);
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        let (second_result_tx, mut second_result_rx) = mpsc::unbounded_channel();
        queue
            .try_enqueue(delivery("first"), second_result_tx)
            .expect("reconnect retry should enqueue");
        let (delivery_id, _, result) =
            tokio::time::timeout(Duration::from_secs(2), second_result_rx.recv())
                .await
                .expect("second delivery should complete")
                .expect("acceptance result should be sent");

        assert_eq!(delivery_id, "first");
        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(max_active.load(Ordering::SeqCst), 1);
        assert_eq!(persisted_delivery_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn connector_reconnects_and_acks_only_after_serialized_acceptance() {
        let runtime_state = runtime_state_from_app(
            crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
                .expect("test daemon should bootstrap"),
        );
        let started = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let release = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let persisted = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new()));
        let second_delivery_sent = Arc::new(tokio::sync::Notify::new());
        let early_ack_checked = Arc::new(tokio::sync::Notify::new());
        let early_ack = Arc::new(AtomicUsize::new(0));
        let ack_received = Arc::new(tokio::sync::Notify::new());
        let acceptor: DeliveryAcceptor = {
            let started = started.clone();
            let release = release.clone();
            let persisted = persisted.clone();
            let seen = seen.clone();
            Arc::new(move |delivery| {
                {
                    let (lock, cv) = &*started;
                    *lock.lock().expect("started lock should not be poisoned") = true;
                    cv.notify_all();
                }
                {
                    let (lock, cv) = &*release;
                    let mut released = lock.lock().expect("release lock should not be poisoned");
                    while !*released {
                        released = cv
                            .wait(released)
                            .expect("release wait should not be poisoned");
                    }
                }
                if seen
                    .lock()
                    .expect("delivery set should not be poisoned")
                    .insert(delivery.delivery_id)
                {
                    persisted.fetch_add(1, Ordering::SeqCst);
                }
                Ok(())
            })
        };
        let delivery_queue = DeliveryAcceptanceQueue::with_acceptor(acceptor);
        let delivery = delivery("reconnect");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind AEDS fixture");
        let address = listener.local_addr().expect("resolve AEDS fixture");
        let server_delivery = delivery.clone();
        let server_second_delivery_sent = second_delivery_sent.clone();
        let server_early_ack_checked = early_ack_checked.clone();
        let server_early_ack = early_ack.clone();
        let server_ack_received = ack_received.clone();
        let server = tokio::spawn(async move {
            for connection_index in 0..2 {
                let (stream, _) = listener.accept().await.expect("accept kernel connection");
                let mut socket = tokio_tungstenite::accept_async(stream)
                    .await
                    .expect("upgrade AEDS fixture connection");
                socket
                    .next()
                    .await
                    .expect("kernel should send hello")
                    .expect("hello should be readable");
                socket
                    .send(Message::Text(
                        serde_json::to_string(&AedsToKernelMessage::HelloAccepted {
                            protocol_version: EVENT_DELIVERY_PROTOCOL_VERSION,
                            heartbeat_interval_ms: 15_000,
                        })
                        .expect("encode hello acceptance")
                        .into(),
                    ))
                    .await
                    .expect("send hello acceptance");
                socket
                    .send(Message::Text(
                        serde_json::to_string(&AedsToKernelMessage::Delivery {
                            delivery: server_delivery.clone(),
                        })
                        .expect("encode delivery")
                        .into(),
                    ))
                    .await
                    .expect("send delivery");
                if connection_index == 0 {
                    // Simulate the AEDS socket dying while acceptance is still blocked.
                    drop(socket);
                    continue;
                }
                server_second_delivery_sent.notify_one();
                if tokio::time::timeout(Duration::from_millis(200), socket.next())
                    .await
                    .is_ok()
                {
                    server_early_ack.fetch_add(1, Ordering::SeqCst);
                }
                server_early_ack_checked.notify_one();
                loop {
                    let message = socket
                        .next()
                        .await
                        .expect("kernel should acknowledge retry")
                        .expect("ack should be readable");
                    let text = message.to_text().expect("ack should be text");
                    if matches!(
                        serde_json::from_str::<KernelToAedsMessage>(text)
                            .expect("decode kernel message"),
                        KernelToAedsMessage::Ack { .. }
                    ) {
                        server_ack_received.notify_one();
                        break;
                    }
                }
            }
        });
        let config = EventDeliveryClientConfig {
            url: Some(format!("ws://{address}")),
            token: None,
            kernel_id: "kernel-test".to_string(),
            environment_id: "environment-test".to_string(),
            generator_management_targets: BTreeMap::new(),
            config_projection: DaemonConfigProjectionStore::new(
                crate::config::DaemonConfig::for_tests(),
            ),
        };
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector = tokio::spawn(run_event_delivery_connector_with_queue(
            runtime_state,
            config,
            shutdown_rx,
            delivery_queue,
        ));
        second_delivery_sent.notified().await;
        {
            let (lock, cv) = &*started;
            let mut has_started = lock.lock().expect("started lock should not be poisoned");
            while !*has_started {
                has_started = cv
                    .wait(has_started)
                    .expect("started wait should not be poisoned");
            }
        }
        early_ack_checked.notified().await;
        assert_eq!(early_ack.load(Ordering::SeqCst), 0);
        {
            let (lock, cv) = &*release;
            *lock.lock().expect("release lock should not be poisoned") = true;
            cv.notify_all();
        }
        ack_received.notified().await;
        assert_eq!(persisted.load(Ordering::SeqCst), 1);
        shutdown_tx.send(true).expect("stop connector");
        connector.await.expect("join connector");
        server.await.expect("join AEDS fixture");
        assert_eq!(persisted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn connector_sends_proactive_heartbeat_after_handshake() {
        let runtime_state = runtime_state_from_app(
            crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
                .expect("test daemon should bootstrap"),
        );
        let delivery_queue = DeliveryAcceptanceQueue::with_acceptor(Arc::new(|_| Ok(())));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind AEDS fixture");
        let address = listener.local_addr().expect("resolve AEDS fixture");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept kernel connection");
            let mut socket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("upgrade AEDS fixture connection");
            let hello = socket
                .next()
                .await
                .expect("kernel should send hello")
                .expect("hello should be readable");
            assert!(matches!(
                serde_json::from_str::<KernelToAedsMessage>(
                    hello.to_text().expect("hello should be text")
                )
                .expect("decode hello"),
                KernelToAedsMessage::Hello { .. }
            ));
            socket
                .send(Message::Text(
                    serde_json::to_string(&AedsToKernelMessage::HelloAccepted {
                        protocol_version: EVENT_DELIVERY_PROTOCOL_VERSION,
                        heartbeat_interval_ms: 3_000,
                    })
                    .expect("encode hello acceptance")
                    .into(),
                ))
                .await
                .expect("send hello acceptance");
            loop {
                let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
                    .await
                    .expect("kernel should send a heartbeat")
                    .expect("kernel should keep the connection open")
                    .expect("heartbeat should be readable");
                let message = serde_json::from_str::<KernelToAedsMessage>(
                    message.to_text().expect("heartbeat should be text"),
                )
                .expect("decode heartbeat");
                if matches!(message, KernelToAedsMessage::Heartbeat { .. }) {
                    return;
                }
            }
        });
        let config = EventDeliveryClientConfig {
            url: Some(format!("ws://{address}")),
            token: None,
            kernel_id: "kernel-heartbeat-test".to_string(),
            environment_id: "environment-heartbeat-test".to_string(),
            generator_management_targets: BTreeMap::new(),
            config_projection: DaemonConfigProjectionStore::new(
                crate::config::DaemonConfig::for_tests(),
            ),
        };
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector = tokio::spawn(run_event_delivery_connector_with_queue(
            runtime_state,
            config,
            shutdown_rx,
            delivery_queue,
        ));
        server.await.expect("heartbeat fixture should complete");
        shutdown_tx.send(true).expect("stop connector");
        connector.await.expect("join connector");
    }

    fn runtime_state_from_app(app: crate::app::DaemonApp) -> KernelRuntimeState {
        let config_projection = app.config_projection_store();
        let session_store = app.session_state_store();
        let agent_store = app.agents().clone();
        let attachment_store = app.attachments().clone();
        let provider_store = app.providers().clone();
        let provider_process_tracking = app.provider_process_tracking_store();
        let slice_store = app.slices();
        let session_projection = app.session_state_projection_store();
        let provider_run_projection = app.provider_run_projection_store();
        let operational_history_store = app.operational_history_store();
        let durable_state_store = app.durable_state_store();
        let prompt_state_owner = app.prompt_state_owner();
        let active_turns = app.active_turn_store();
        let prompt_activity = app.prompt_activity_store();
        let prompt_workspace_claims = app.prompt_workspace_claim_store();
        let structured_output_records = app.structured_output_record_store();
        let terminal_stream = app.terminal_stream_store();
        let workflow_design_events = app.workflow_design_event_store();
        let metaagent_events = app.metaagent_event_store();
        let workspace_coordinator = app.workspace_coordinator();
        KernelRuntimeState::new_with_owned_state(
            Arc::new(tokio::sync::Mutex::new(app)),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }
}
