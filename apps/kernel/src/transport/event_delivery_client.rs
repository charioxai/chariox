use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chariox_event_protocol::{
    AedsToKernelMessage, KernelToAedsMessage, EVENT_DELIVERY_PROTOCOL_VERSION,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::Message;

use crate::runtime::state::KernelRuntimeState;

#[derive(Debug, Clone)]
pub(crate) struct EventDeliveryClientConfig {
    pub url: Option<String>,
    pub token: Option<String>,
    pub kernel_id: String,
    pub environment_id: String,
    pub generator_management_targets:
        BTreeMap<String, crate::config::EventGeneratorManagementTarget>,
}

#[derive(Debug, Clone, Default)]
struct EventDeliveryConnectionHealth {
    connected: bool,
    last_connected_at_ms: Option<u64>,
    last_error: Option<String>,
}

static EVENT_DELIVERY_HEALTH: OnceLock<Mutex<EventDeliveryConnectionHealth>> = OnceLock::new();

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
    mut shutdown: watch::Receiver<bool>,
) {
    let Some(url) = config.url.clone() else {
        let _ = shutdown.changed().await;
        return;
    };
    let mut retry = Duration::from_secs(1);
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
        match connect_once(&runtime_state, &config, &url, shutdown.clone()).await {
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
    // Delivery acceptance performs synchronous durable-state work. Keep it off this
    // socket task so a slow database write cannot starve AEDS heartbeats and cause
    // the server to retry an event that is already being accepted.
    let (delivery_tx, mut delivery_rx) =
        mpsc::channel::<chariox_event_protocol::EventDeliveryEnvelope>(32);
    let (delivery_result_tx, mut delivery_result_rx) =
        mpsc::unbounded_channel::<(String, Duration, Result<(), String>)>();
    let delivery_worker_state = runtime_state.clone();
    tokio::spawn(async move {
        while let Some(delivery) = delivery_rx.recv().await {
            let delivery_id = delivery.delivery_id.clone();
            let started_at = Instant::now();
            let state = delivery_worker_state.clone();
            let result =
                tokio::task::spawn_blocking(move || state.accept_workflow_event_delivery(delivery))
                    .await
                    .map_err(|error| format!("event delivery worker failed: {error}"))
                    .and_then(|result| result.map(|_| ()).map_err(|error| error.to_string()));
            let _ = delivery_result_tx.send((delivery_id, started_at.elapsed(), result));
        }
    });
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
                    AedsToKernelMessage::HelloAccepted { protocol_version, .. } => {
                        if protocol_version != EVENT_DELIVERY_PROTOCOL_VERSION {
                            return Err(format!("AEDS negotiated unsupported protocol {protocol_version}"));
                        }
                        crate::logging::info_with_fields(
                            "daemon.event_delivery",
                            "AEDS connection established",
                            serde_json::json!({"url": url}),
                        );
                        record_connected();
                    }
                    AedsToKernelMessage::RoutesReconciled { conflicts, .. } => {
                        runtime_state.apply_event_route_conflicts(&conflicts);
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
                        if let Err(error) = delivery_tx.try_send(delivery.clone()) {
                            pending_delivery_ids.remove(&delivery.delivery_id);
                            match error {
                                tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                                    return Err("event delivery worker stopped".to_string());
                                }
                                tokio::sync::mpsc::error::TrySendError::Full(_) => {
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
        }
    }
}

async fn reconcile_aegs_subscriptions(
    runtime_state: &KernelRuntimeState,
    config: &EventDeliveryClientConfig,
) -> Result<(), String> {
    if config.generator_management_targets.is_empty() {
        return Ok(());
    }
    let mut claims = runtime_state.event_generator_subscription_claims();
    for (generator_id, target) in &config.generator_management_targets {
        let request = chariox_event_protocol::AegsSubscriptionReconcileRequest {
            owner_id: config.kernel_id.clone(),
            generator_id: generator_id.clone(),
            subscriptions: claims.remove(generator_id).unwrap_or_default(),
        };
        let target = target.clone();
        let generator_id = generator_id.clone();
        tokio::task::spawn_blocking(move || {
            let url = format!("{}/v1/subscriptions/reconcile", target.url);
            let encoded = serde_json::to_string(&request).map_err(|error| error.to_string())?;
            let response = ureq::put(&url)
                .set("authorization", &format!("Bearer {}", target.token))
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
