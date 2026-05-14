use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, MissedTickBehavior};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use arroba_relay::protocol::{
    ClientTarget, EncryptedRelayPayload, RelayCallerIdentity, RelayEnvelope, RelayError,
};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::runtime::event_log::{EventLog, ReplayOutcome};
use crate::runtime::projection::SessionSnapshotProjection;
use crate::runtime::router::{CommandRouter, INTERACTIVE_COMMAND_QUEUE_LIMIT};
use crate::runtime_transport::{WatchResult, RECENT_EVENT_LIMIT, WATCH_INTERVAL_MS};
use crate::transport::kernel_protocol::{
    event_is_relevant_to_attachment, subscription_event_stream_id, KernelEvent,
    WAITING_ROOM_INVENTORY_SENTINEL_ID, WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
};
use crate::transport::relay_crypto;
use crate::transport::relay_discovery;
use crate::transport::relay_peer::{RelayPeerEvent, RelayPeerRequest, RelayPeerResponse};

mod connection_config;
mod events;
mod peer_client;
mod remote_inventory;
mod request_errors;
mod subscriptions;
use connection_config::{relay_config_continuity, RelayConfigContinuity};
use events::{emit_relay_event, replay_recent_relay_events, RelayEventRuntime};
#[cfg(test)]
pub use peer_client::send_peer_request_via_relay;
pub use peer_client::send_peer_request_via_temporary_connection;
use peer_client::{resolve_pending_peer_response, RelayPeerResponseEnvelope};
pub(crate) use remote_inventory::refresh_remote_inventory_projection_for_app_with_relay_state;
use remote_inventory::{
    abort_inventory_refresh_task, clear_remote_inventory_projection,
    spawn_remote_inventory_projection_refresh,
};
use request_errors::{map_relay_error, relay_error, relay_request_kind};
use subscriptions::{
    abort_subscription_tasks, relay_subscription_task_key,
    remove_relay_subscription_task_by_relay_id, run_relay_subscription_loop, RelaySubscriptionTask,
    RelaySubscriptionTasks,
};

#[allow(dead_code)]
#[derive(Debug)]
pub struct RelayClientState {
    connected: bool,
    outgoing_tx: Option<mpsc::UnboundedSender<RelayEnvelope>>,
    pending_peer_requests: BTreeMap<String, oneshot::Sender<RelayPeerResponseEnvelope>>,
    next_peer_request_id: u64,
}

impl RelayClientState {
    pub fn connected(&self) -> bool {
        self.connected
    }
}

impl Default for RelayClientState {
    fn default() -> Self {
        Self {
            connected: false,
            outgoing_tx: None,
            pending_peer_requests: BTreeMap::new(),
            next_peer_request_id: 0,
        }
    }
}

const RELAY_HEARTBEAT_INTERVAL_TICKS: u64 = 20;
const RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS: u64 = 50;
const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CLOUD_RELAY_TOKEN_REFRESH_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const CLOUD_RELAY_PRESENCE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const RELAY_HEARTBEAT_APP_WORK_TIMEOUT: Duration = Duration::from_millis(25);
const REMOTE_INVENTORY_RELAY_TIMEOUT_MS: u64 = 10_000;
const REMOTE_INVENTORY_KERNEL_PROBE_TIMEOUT_MS: u64 = 5_000;

pub async fn run_daemon_relay_connector(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let (provider_runtime_lanes, relay_event_counter_path) = {
        let app = app.lock().await;
        (
            app.provider_run_operation_lanes(),
            app.config().kernel_relay_event_counter_path(),
        )
    };
    let event_runtime = match RelayEventRuntime::new(relay_event_counter_path) {
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
    let router = Arc::new(CommandRouter::with_interactive_capacity_and_provider_lanes(
        Arc::clone(&app),
        INTERACTIVE_COMMAND_QUEUE_LIMIT,
        provider_runtime_lanes,
    ));
    let command_sequence = Arc::new(AtomicU64::new(1));

    loop {
        if *shutdown.borrow() {
            publish_offline_and_set_disconnected(&router, &state, "shutdown before relay connect")
                .await;
            return;
        }

        if let Err(error) = router.ensure_cloud_relay_connection().await {
            crate::logging::warn_with_fields(
                "daemon.relay_client",
                "failed to refresh cloud relay token",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
        }

        let (relay_url, mut active_relay_token, heartbeat) = {
            let config = router.relay_config_snapshot();
            match (config.relay_url.clone(), config.relay_token.clone()) {
                (Some(relay_url), Some(relay_token)) => (
                    relay_url,
                    relay_token,
                    Duration::from_millis(config.relay_heartbeat_ms),
                ),
                _ => {
                    publish_offline_and_set_disconnected(
                        &router,
                        &state,
                        "relay configuration unavailable",
                    )
                    .await;
                    let wait = sleep(Duration::from_secs(1));
                    tokio::pin!(wait);
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_ok() && *shutdown.borrow() {
                                publish_offline_and_set_disconnected(
                                    &router,
                                    &state,
                                    "shutdown while relay configuration unavailable",
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
        };

        crate::logging::info_with_fields(
            "daemon.relay_client",
            "attempting relay connection",
            serde_json::json!({
                "relay_url": relay_url,
            }),
        );
        match timeout(RELAY_CONNECT_TIMEOUT, connect_async(&relay_url)).await {
            Err(_) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "relay socket connect timed out",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "timeout_ms": RELAY_CONNECT_TIMEOUT.as_millis(),
                    }),
                );
                publish_offline_and_set_disconnected(&router, &state, "relay connect timed out")
                    .await;
                sleep(Duration::from_secs(1)).await;
                continue;
            }
            Ok(Ok((socket, _))) => {
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "relay socket connected",
                    serde_json::json!({
                        "relay_url": relay_url,
                    }),
                );
                let (mut writer, mut reader) = socket.split();
                let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<RelayEnvelope>();
                let (writer_done_tx, mut writer_done_rx) = oneshot::channel::<()>();
                let writer_task = tokio::spawn(async move {
                    while let Some(envelope) = outgoing_rx.recv().await {
                        let payload = match serde_json::to_string(&envelope) {
                            Ok(payload) => payload,
                            Err(_) => break,
                        };
                        if writer.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    let _ = writer_done_tx.send(());
                });
                let subscription_tasks: RelaySubscriptionTasks =
                    Arc::new(Mutex::new(BTreeMap::new()));
                let daemon_id = router.relay_daemon_id();
                let register = {
                    RelayEnvelope::DaemonRegister {
                        registration: router.relay_registration().await,
                    }
                };
                if outgoing_tx.send(register).is_err() {
                    writer_task.abort();
                    clear_remote_inventory_projection(&app).await;
                    publish_offline_and_set_disconnected(
                        &router,
                        &state,
                        "failed to send relay registration",
                    )
                    .await;
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                crate::logging::info_with_fields(
                    "daemon.relay_client",
                    "relay register sent",
                    serde_json::json!({
                        "relay_url": relay_url,
                    }),
                );
                set_connected(&state, outgoing_tx.clone()).await;
                publish_cloud_presence(&router, true, "relay registration sent").await;
                let mut last_cloud_presence_publish = Instant::now();
                let mut inventory_refresh_task = Some(spawn_remote_inventory_projection_refresh(
                    Arc::clone(&app),
                    Arc::clone(&state),
                ));
                let mut heartbeat_interval = tokio::time::interval(heartbeat);
                heartbeat_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
                let mut token_refresh_interval =
                    tokio::time::interval(CLOUD_RELAY_TOKEN_REFRESH_CHECK_INTERVAL);
                token_refresh_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
                let mut heartbeat_tick: u64 = 0;

                loop {
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_ok() && *shutdown.borrow() {
                                publish_cloud_presence(&router, false, "daemon shutting down").await;
                                let _ = outgoing_tx.send(RelayEnvelope::Close {
                                    reason: "daemon shutting down".to_string(),
                                });
                                sleep(Duration::from_millis(25)).await;
                                abort_inventory_refresh_task(&mut inventory_refresh_task);
                                abort_subscription_tasks(&subscription_tasks).await;
                                writer_task.abort();
                                clear_remote_inventory_projection(&app).await;
                                publish_offline_and_set_disconnected(&router, &state, "daemon shutting down").await;
                                return;
                            }
                        }
                        incoming = reader.next() => {
                            match incoming {
                                Some(Ok(Message::Text(payload))) => {
                                    if handle_incoming_envelope(
                                        &router,
                                        &app,
                                        &command_sequence,
                                        &state,
                                        &outgoing_tx,
                                        &subscription_tasks,
                                        &event_runtime,
                                        &payload,
                                    )
                                    .await
                                    .is_err()
                                    {
                                        abort_inventory_refresh_task(&mut inventory_refresh_task);
                                        abort_subscription_tasks(&subscription_tasks).await;
                                        writer_task.abort();
                                        clear_remote_inventory_projection(&app).await;
                                        publish_offline_and_set_disconnected(&router, &state, "relay payload handling failed").await;
                                        break;
                                    }
                                }
                                Some(Ok(Message::Close(_))) => {
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&app).await;
                                    publish_offline_and_set_disconnected(&router, &state, "relay close frame received").await;
                                    break;
                                }
                                Some(Ok(_)) => {}
                                Some(Err(_)) | None => {
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&app).await;
                                    publish_offline_and_set_disconnected(&router, &state, "relay read failed or ended").await;
                                    break;
                                }
                            }
                        }
                        writer_done = &mut writer_done_rx => {
                            let _ = writer_done;
                            abort_inventory_refresh_task(&mut inventory_refresh_task);
                            abort_subscription_tasks(&subscription_tasks).await;
                            writer_task.abort();
                            clear_remote_inventory_projection(&app).await;
                            publish_offline_and_set_disconnected(&router, &state, "relay writer ended").await;
                            break;
                        }
                        _ = token_refresh_interval.tick() => {
                            if router.cloud_relay_token_refresh_due() {
                                if let Err(error) = router.ensure_cloud_relay_connection().await {
                                    crate::logging::warn_with_fields(
                                        "daemon.relay_client",
                                        "failed to refresh cloud relay token",
                                        serde_json::json!({
                                            "relay_url": relay_url,
                                            "error": error.to_string(),
                                        }),
                                    );
                                }
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
                                    let _ = outgoing_tx.send(RelayEnvelope::Close {
                                        reason: "relay configuration changed".to_string(),
                                    });
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&app).await;
                                    publish_offline_and_set_disconnected(&router, &state, "relay configuration changed").await;
                                    break;
                                }
                            }
                        }
                        _ = heartbeat_interval.tick() => {
                            heartbeat_tick = heartbeat_tick.wrapping_add(1);
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
                                    let _ = outgoing_tx.send(RelayEnvelope::Close {
                                        reason: "relay configuration changed".to_string(),
                                    });
                                    abort_inventory_refresh_task(&mut inventory_refresh_task);
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    clear_remote_inventory_projection(&app).await;
                                    publish_offline_and_set_disconnected(&router, &state, "relay configuration changed").await;
                                    break;
                                }
                            }
                            if heartbeat_tick.is_multiple_of(RELAY_WAITING_ROOM_INVENTORY_INTERVAL_TICKS)
                                && inventory_refresh_task
                                    .as_ref()
                                    .is_none_or(|task| task.is_finished())
                            {
                                inventory_refresh_task = Some(
                                    spawn_remote_inventory_projection_refresh(
                                        Arc::clone(&app),
                                        Arc::clone(&state),
                                    )
                                );
                            }
                            let heartbeat_frame = RelayEnvelope::DaemonHeartbeat {
                                daemon_id: daemon_id.clone(),
                                registration: None,
                            };
                            if outgoing_tx.send(heartbeat_frame).is_err() {
                                abort_inventory_refresh_task(&mut inventory_refresh_task);
                                abort_subscription_tasks(&subscription_tasks).await;
                                writer_task.abort();
                                clear_remote_inventory_projection(&app).await;
                                publish_offline_and_set_disconnected(&router, &state, "relay heartbeat send failed").await;
                                break;
                            }
                            let _ = timeout(
                                RELAY_HEARTBEAT_APP_WORK_TIMEOUT,
                                pump_leased_projection_events(&router, &outgoing_tx),
                            )
                            .await;
                            if last_cloud_presence_publish.elapsed()
                                >= CLOUD_RELAY_PRESENCE_REFRESH_INTERVAL
                            {
                                publish_cloud_presence(&router, true, "relay heartbeat").await;
                                last_cloud_presence_publish = Instant::now();
                            }
                        }
                    }
                }
            }
            Ok(Err(error)) => {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "relay socket connect failed",
                    serde_json::json!({
                        "relay_url": relay_url,
                        "error": error.to_string(),
                    }),
                );
                clear_remote_inventory_projection(&app).await;
                publish_offline_and_set_disconnected(
                    &router,
                    &state,
                    "relay socket connect failed",
                )
                .await;
                let reconnect_delay = sleep(Duration::from_secs(1));
                tokio::pin!(reconnect_delay);
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_ok() && *shutdown.borrow() {
                            return;
                        }
                    }
                    _ = &mut reconnect_delay => {}
                }
            }
        }
    }
}

async fn handle_incoming_envelope(
    router: &Arc<CommandRouter>,
    app: &Arc<Mutex<DaemonApp>>,
    command_sequence: &Arc<AtomicU64>,
    state: &Arc<RwLock<RelayClientState>>,
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    subscription_tasks: &RelaySubscriptionTasks,
    event_runtime: &Arc<RelayEventRuntime>,
    payload: &str,
) -> Result<(), DaemonError> {
    let envelope = serde_json::from_str::<RelayEnvelope>(payload).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "parse relay envelope",
            message: error.to_string(),
        }
    })?;
    match envelope {
        RelayEnvelope::DaemonRequest {
            relay_request_id,
            caller_identity,
            encrypted_request,
        } => {
            let relay_response =
                handle_daemon_request(router, command_sequence, caller_identity, encrypted_request)
                    .await;
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: relay_response.encrypted_response,
                    error: relay_response.error,
                },
            )?;
        }
        RelayEnvelope::DaemonIncomingPeerRequest {
            relay_request_id,
            from_daemon_id: _,
            caller_identity: _,
            encrypted_request,
        } => {
            let router = Arc::clone(router);
            let outgoing_tx = outgoing_tx.clone();
            tokio::spawn(async move {
                let relay_response =
                    handle_daemon_peer_request(&router, &outgoing_tx, encrypted_request).await;
                if let Err(error) = send_outgoing_envelope(
                    &outgoing_tx,
                    RelayEnvelope::DaemonIncomingPeerResponse {
                        relay_request_id,
                        encrypted_response: relay_response.encrypted_response,
                        error: relay_response.error,
                    },
                ) {
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "failed to send async daemon peer response",
                        serde_json::json!({
                            "error": error.to_string(),
                        }),
                    );
                }
            });
        }
        RelayEnvelope::DaemonPeerResponse {
            request_id,
            from_daemon_id,
            encrypted_response,
            error,
        } => {
            resolve_pending_peer_response(
                state,
                request_id,
                RelayPeerResponseEnvelope {
                    from_daemon_id,
                    encrypted_response,
                    error,
                },
            )
            .await;
        }
        RelayEnvelope::DaemonIncomingPeerEvent {
            from_daemon_id: _,
            caller_identity: _,
            encrypted_event,
        } => {
            if let Err(error) = handle_daemon_peer_event(router, encrypted_event).await {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "failed to handle relay peer event",
                    serde_json::json!({
                        "error": error.to_string(),
                    }),
                );
            }
        }
        RelayEnvelope::DaemonSubscribe {
            relay_request_id,
            relay_subscription_id,
            caller_identity: _,
            session_id,
            attachment_id,
            client_public_key,
            subscription_scope,
            resume_from_event_id,
        } => {
            let is_inventory_subscription =
                subscription_scope.as_deref() == Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE);
            crate::logging::info_with_fields(
                "daemon.relay_client",
                "relay subscription request received",
                serde_json::json!({
                    "relay_request_id": relay_request_id,
                    "relay_subscription_id": relay_subscription_id,
                    "session_id": session_id,
                    "attachment_id": attachment_id,
                    "subscription_scope": subscription_scope,
                    "resume_from_event_id": resume_from_event_id,
                    "is_waiting_room_inventory_subscription": is_inventory_subscription,
                }),
            );
            if !is_inventory_subscription
                && (session_id == WAITING_ROOM_INVENTORY_SENTINEL_ID
                    || attachment_id == WAITING_ROOM_INVENTORY_SENTINEL_ID)
            {
                crate::logging::warn_with_fields(
                    "daemon.relay_client",
                    "waiting-room inventory sentinel arrived without subscription scope",
                    serde_json::json!({
                        "relay_request_id": relay_request_id,
                        "relay_subscription_id": relay_subscription_id,
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                        "subscription_scope": subscription_scope,
                        "diagnosis": "relay or client likely dropped subscription_scope=waiting_room_inventory",
                    }),
                );
            }
            if !is_inventory_subscription {
                if let Err(error) = router
                    .ensure_relay_subscription_attachment(&session_id, &attachment_id)
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "relay subscription attachment validation failed",
                        serde_json::json!({
                            "relay_request_id": relay_request_id,
                            "relay_subscription_id": relay_subscription_id,
                            "session_id": session_id,
                            "attachment_id": attachment_id,
                            "subscription_scope": subscription_scope,
                            "error": error.to_string(),
                        }),
                    );
                    send_outgoing_envelope(
                        outgoing_tx,
                        RelayEnvelope::DaemonResponse {
                            relay_request_id,
                            encrypted_response: None,
                            error: Some(map_relay_error(&error)),
                        },
                    )?;
                    return Ok(());
                }
            }
            let task_key = relay_subscription_task_key(
                &session_id,
                &attachment_id,
                subscription_scope.as_deref(),
            );
            if let Some(existing) = subscription_tasks.lock().await.remove(&task_key) {
                existing.handle.abort();
            }
            let ack = match encrypt_json_response(
                router,
                &client_public_key,
                serde_json::json!({
                    "ok": true,
                    "resumed_from_event_id": resume_from_event_id,
                }),
            )
            .await
            {
                Ok(ack) => ack,
                Err(error) => {
                    send_outgoing_envelope(
                        outgoing_tx,
                        RelayEnvelope::DaemonResponse {
                            relay_request_id,
                            encrypted_response: None,
                            error: Some(map_relay_error(&error)),
                        },
                    )?;
                    return Ok(());
                }
            };
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: Some(ack),
                    error: None,
                },
            )?;
            if !is_inventory_subscription {
                if let Err(error) = replay_recent_relay_events(
                    event_runtime,
                    router,
                    app,
                    outgoing_tx,
                    &relay_subscription_id,
                    &client_public_key,
                    &session_id,
                    &attachment_id,
                    resume_from_event_id,
                )
                .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.relay_client",
                        "failed to replay relay subscription events",
                        serde_json::json!({
                            "relay_subscription_id": relay_subscription_id,
                            "session_id": session_id,
                            "attachment_id": attachment_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
            let task = tokio::spawn(run_relay_subscription_loop(
                Arc::clone(router),
                outgoing_tx.clone(),
                relay_subscription_id.clone(),
                client_public_key.clone(),
                session_id.clone(),
                attachment_id.clone(),
                subscription_scope.clone(),
                Arc::clone(event_runtime),
            ));
            subscription_tasks.lock().await.insert(
                task_key,
                RelaySubscriptionTask {
                    relay_subscription_id: relay_subscription_id.clone(),
                    handle: task,
                },
            );
        }
        RelayEnvelope::DaemonUnsubscribe {
            relay_request_id,
            relay_subscription_id,
            caller_identity: _,
            client_public_key,
        } => {
            let existing = remove_relay_subscription_task_by_relay_id(
                subscription_tasks,
                &relay_subscription_id,
            )
            .await;
            if let Some(task) = existing {
                task.handle.abort();
            }
            let ack = match encrypt_json_response(
                router,
                &client_public_key,
                serde_json::json!({ "ok": true }),
            )
            .await
            {
                Ok(ack) => ack,
                Err(error) => {
                    send_outgoing_envelope(
                        outgoing_tx,
                        RelayEnvelope::DaemonResponse {
                            relay_request_id,
                            encrypted_response: None,
                            error: Some(map_relay_error(&error)),
                        },
                    )?;
                    return Ok(());
                }
            };
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: Some(ack),
                    error: None,
                },
            )?;
        }
        RelayEnvelope::ClientMetadataResponse { .. } => {}
        RelayEnvelope::Close { reason } => {
            return Err(DaemonError::LocalTransport {
                operation: "relay closed connection",
                message: reason,
            });
        }
        _ => {}
    }
    Ok(())
}

async fn pump_leased_projection_events(
    router: &Arc<CommandRouter>,
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
) {
    let events = match router.relay_pump_leased_runtime_projections().await {
        Ok(events) => events,
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.relay",
                "failed to pump leased runtime projections",
                serde_json::json!({
                    "error": error.to_string(),
                }),
            );
            return;
        }
    };
    for (target_daemon_id, event) in events {
        let config = router.relay_config_snapshot();
        let target_kernel = match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            relay_discovery::get_live_kernel(&config, &target_daemon_id),
        )
        .await
        {
            Ok(Ok(kernel)) => kernel,
            Ok(Err(error)) => {
                crate::logging::warn_with_fields(
                    "daemon.relay",
                    "failed to resolve leased runtime projection target",
                    serde_json::json!({
                        "target_daemon_id": target_daemon_id,
                        "error": error.to_string(),
                    }),
                );
                continue;
            }
            Err(_) => {
                crate::logging::warn_with_fields(
                    "daemon.relay",
                    "timed out resolving leased runtime projection target",
                    serde_json::json!({
                        "target_daemon_id": target_daemon_id,
                    }),
                );
                continue;
            }
        };
        let encrypted_event = match encrypt_peer_payload(
            &config.relay_private_key,
            &target_kernel.public_key,
            &event,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.relay",
                    "failed to encrypt leased runtime projection event",
                    serde_json::json!({
                        "target_daemon_id": target_daemon_id,
                        "error": error.to_string(),
                    }),
                );
                continue;
            }
        };
        if let Err(error) = send_outgoing_envelope(
            outgoing_tx,
            RelayEnvelope::DaemonPeerEvent {
                target: ClientTarget {
                    daemon_id: Some(target_daemon_id.clone()),
                    daemon_alias: None,
                },
                encrypted_event,
            },
        ) {
            crate::logging::warn_with_fields(
                "daemon.relay",
                "failed to send leased runtime projection event",
                serde_json::json!({
                    "target_daemon_id": target_daemon_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

#[derive(Debug, Clone)]
struct RelayRequestOutcome {
    encrypted_response: Option<EncryptedRelayPayload>,
    error: Option<RelayError>,
}

async fn handle_daemon_peer_request(
    router: &Arc<CommandRouter>,
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    encrypted_request: EncryptedRelayPayload,
) -> RelayRequestOutcome {
    let (request, requester_public_key, daemon_private_key, daemon_id) = {
        let daemon_private_key = router.relay_private_key();
        let daemon_id = router.relay_daemon_id();
        let decrypted = match relay_crypto::decrypt_payload_for_private_key(
            &daemon_private_key,
            &encrypted_request,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay peer request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        let request = match serde_json::from_slice::<RelayPeerRequest>(&decrypted.plaintext) {
            Ok(request) => request,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay peer request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        (
            request,
            decrypted.sender_public_key,
            daemon_private_key,
            daemon_id,
        )
    };

    let response = match request {
        RelayPeerRequest::Ping { value } => RelayPeerResponse::Pong { value, daemon_id },
        RelayPeerRequest::CreateExecutionLease {
            home_kernel_id,
            home_session_id,
            home_agent_id,
            owner_user_id,
        } => {
            let lease = router
                .relay_create_execution_lease(
                    &home_kernel_id,
                    &home_session_id,
                    &home_agent_id,
                    &owner_user_id,
                )
                .await;
            match lease {
                Ok(lease) => RelayPeerResponse::ExecutionLeaseCreated { lease },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::DestroyExecutionLease { lease_id } => {
            let destroyed = router.relay_destroy_execution_lease(&lease_id).await;
            match destroyed {
                Ok(_) => RelayPeerResponse::ExecutionLeaseDestroyed { lease_id },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::SpawnLeasedAgent {
            lease_id,
            provider,
            model,
            effort,
            execution_mode,
            permission_level,
            worktree_id,
            worktree_placement,
        } => {
            let leased_agent = router
                .relay_create_leased_agent(
                    &lease_id,
                    &provider,
                    model,
                    effort,
                    execution_mode,
                    permission_level,
                    worktree_id,
                    worktree_placement,
                )
                .await;
            match leased_agent {
                Ok(leased_agent) => RelayPeerResponse::LeasedAgentSpawned { leased_agent },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::DestroyLeasedAgent { leased_agent_id } => {
            let destroyed = router.relay_destroy_leased_agent(&leased_agent_id).await;
            match destroyed {
                Ok(_) => RelayPeerResponse::LeasedAgentDestroyed { leased_agent_id },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::UpdateLeasedAgentConfig {
            leased_agent_id,
            execution_mode,
            permission_level,
        } => {
            let updated = router
                .relay_update_leased_agent_config(
                    &leased_agent_id,
                    execution_mode,
                    permission_level,
                )
                .await;
            match updated {
                Ok(leased_agent) => RelayPeerResponse::LeasedAgentConfigUpdated { leased_agent },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::SubmitLeasedPrompt {
            leased_agent_id,
            prompt,
            attachments,
            workflow_context,
            git_context,
            required_mcps,
        } => {
            let submitted = router
                .relay_submit_leased_prompt(
                    &leased_agent_id,
                    &prompt,
                    attachments,
                    workflow_context,
                    git_context,
                    required_mcps,
                )
                .await;
            match submitted {
                Ok((provider_run_id, outcome)) => {
                    if let Err(error) = emit_leased_projection_event(
                        router,
                        outgoing_tx,
                        &leased_agent_id,
                        &provider_run_id,
                        true,
                    )
                    .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.relay",
                            "failed to emit leased runtime projection after submit",
                            serde_json::json!({
                                "leased_agent_id": leased_agent_id,
                                "provider_run_id": provider_run_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    RelayPeerResponse::LeasedPromptSubmitted {
                        provider_run_id,
                        outcome,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::CompleteLeasedPrompt { leased_agent_id } => {
            let completion = router.relay_complete_leased_prompt(&leased_agent_id).await;
            match completion {
                Ok(completion) => {
                    let provider_run_id = router
                        .relay_leased_agent_provider_run_id(&leased_agent_id)
                        .await
                        .ok()
                        .flatten();
                    let provider_diagnostic =
                        if let Some(provider_run_id) = provider_run_id.as_deref() {
                            router
                                .relay_provider_run_terminal_diagnostic(provider_run_id)
                                .await
                                .ok()
                                .flatten()
                        } else {
                            None
                        };
                    let git_observations = if let Some(provider_run_id) = provider_run_id.as_deref()
                    {
                        router
                            .relay_observe_leased_git_after(&leased_agent_id, provider_run_id)
                            .await
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    RelayPeerResponse::LeasedPromptCompleted {
                        provider_run_id,
                        provider_diagnostic,
                        git_observations,
                        completion,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::CancelLeasedPrompt { leased_agent_id } => {
            let cancellation = router.relay_cancel_leased_prompt(&leased_agent_id).await;
            match cancellation {
                Ok(cancellation) => RelayPeerResponse::LeasedPromptCancelled { cancellation },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardWorkflowRuntimeTool {
            context,
            tool_name,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_workflow_runtime_tool_call(context, tool_name, arguments)
                .await;
            match handled {
                Ok(result) => RelayPeerResponse::WorkflowRuntimeToolHandled { result },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardWorkflowProviderFailure { context, message } => {
            let handled = router
                .dispatch_forwarded_workflow_provider_failure(context, message)
                .await;
            match handled {
                Ok(()) => RelayPeerResponse::WorkflowProviderFailureHandled,
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardManagedIoRuntimeTool {
            context,
            tool_name,
            arguments,
            artifact_states,
        } => {
            let handled = router
                .dispatch_forwarded_managed_io_runtime_tool_call(
                    context,
                    tool_name,
                    arguments,
                    artifact_states,
                )
                .await;
            match handled {
                Ok((result, final_artifact_states)) => {
                    RelayPeerResponse::ManagedIoRuntimeToolHandled {
                        result,
                        final_artifact_states,
                    }
                }
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardCapabilityRuntimeTool {
            context,
            tool_name,
            arguments,
        } => {
            let handled = router
                .dispatch_forwarded_capability_runtime_tool_call(context, tool_name, arguments)
                .await;
            match handled {
                Ok((result, skill_package)) => RelayPeerResponse::CapabilityRuntimeToolHandled {
                    result,
                    skill_package,
                },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::ForwardNativeInteraction {
            context,
            interaction,
        } => {
            let handled = router
                .relay_forward_native_interaction(context, interaction)
                .await;
            match handled {
                Ok(resolution) => RelayPeerResponse::NativeInteractionResolved { resolution },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::EnsureRemoteSkillPackages { context, packages } => {
            let ensured = router
                .relay_ensure_remote_skill_packages(context, packages)
                .await;
            match ensured {
                Ok(materialized) => RelayPeerResponse::RemoteSkillPackagesEnsured { materialized },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
        RelayPeerRequest::CheckRemoteMcpAvailability {
            context,
            required_mcps,
        } => {
            let checked = router
                .relay_check_remote_mcp_availability(context, required_mcps)
                .await;
            match checked {
                Ok(results) => RelayPeerResponse::RemoteMcpAvailabilityChecked { results },
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(map_relay_error(&error)),
                    };
                }
            }
        }
    };
    let plaintext = match serde_json::to_vec(&response) {
        Ok(bytes) => bytes,
        Err(error) => {
            return RelayRequestOutcome {
                encrypted_response: None,
                error: Some(relay_error(
                    "relay_request_failed",
                    &format!("failed to serialize relay peer response: {error}"),
                    false,
                )),
            };
        }
    };
    match relay_crypto::encrypt_payload_for_peer(
        &daemon_private_key,
        &requester_public_key,
        &plaintext,
    ) {
        Ok(encrypted_response) => RelayRequestOutcome {
            encrypted_response: Some(encrypted_response),
            error: None,
        },
        Err(error) => RelayRequestOutcome {
            encrypted_response: None,
            error: Some(relay_error(
                "relay_request_failed",
                &format!("failed to encrypt relay peer response: {error}"),
                false,
            )),
        },
    }
}

async fn handle_daemon_peer_event(
    router: &Arc<CommandRouter>,
    encrypted_event: EncryptedRelayPayload,
) -> Result<(), DaemonError> {
    let daemon_private_key = router.relay_private_key();
    let decrypted =
        relay_crypto::decrypt_payload_for_private_key(&daemon_private_key, &encrypted_event)?;
    let event =
        serde_json::from_slice::<RelayPeerEvent>(&decrypted.plaintext).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "decode relay peer event",
                message: error.to_string(),
            }
        })?;
    match event {
        RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id,
            home_agent_id,
            provider_run_id,
            output_chunks,
            notices,
            completions,
        } => {
            router
                .relay_project_remote_runtime_projection(
                    &home_session_id,
                    &home_agent_id,
                    &provider_run_id,
                    output_chunks,
                    notices,
                    completions,
                )
                .await?;
        }
    }
    Ok(())
}

async fn emit_leased_projection_event(
    router: &Arc<CommandRouter>,
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    leased_agent_id: &str,
    provider_run_id: &str,
    pump_output: bool,
) -> Result<(), DaemonError> {
    let config = router.relay_config_snapshot();
    let Some((target_daemon_id, event)) = router
        .relay_drain_leased_runtime_projection(leased_agent_id, provider_run_id, pump_output)
        .await?
    else {
        return Ok(());
    };
    let target_kernel = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        relay_discovery::get_live_kernel(&config, &target_daemon_id),
    )
    .await
    .map_err(|_| DaemonError::LocalTransport {
        operation: "resolve relay peer event target",
        message: format!("timed out resolving relay target kernel `{target_daemon_id}`"),
    })??;
    let encrypted_event =
        encrypt_peer_payload(&config.relay_private_key, &target_kernel.public_key, &event)?;
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonPeerEvent {
            target: ClientTarget {
                daemon_id: Some(target_daemon_id),
                daemon_alias: None,
            },
            encrypted_event,
        },
    )
}

async fn handle_daemon_request(
    router: &CommandRouter,
    command_sequence: &AtomicU64,
    caller_identity: Option<RelayCallerIdentity>,
    encrypted_request: EncryptedRelayPayload,
) -> RelayRequestOutcome {
    let (request, client_public_key, daemon_private_key) = {
        let daemon_private_key = router.relay_private_key();
        let decrypted = match relay_crypto::decrypt_payload_for_private_key(
            &daemon_private_key,
            &encrypted_request,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        let request = match serde_json::from_slice::<LocalDaemonRequest>(&decrypted.plaintext) {
            Ok(request) => request,
            Err(error) => {
                return RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "invalid_request",
                        &format!("invalid relay request payload: {error}"),
                        false,
                    )),
                };
            }
        };
        (request, decrypted.sender_public_key, daemon_private_key)
    };
    let request_kind = relay_request_kind(&request);
    crate::logging::info_with_fields(
        "daemon.relay_client",
        "relay daemon request dispatching",
        serde_json::json!({
            "request_kind": request_kind,
        }),
    );
    let result =
        dispatch_relay_client_request(router, command_sequence, caller_identity, request).await;
    match result {
        Ok(response) => {
            crate::logging::info_with_fields(
                "daemon.relay_client",
                "relay daemon request dispatched",
                serde_json::json!({
                    "request_kind": request_kind,
                }),
            );
            let plaintext = match serde_json::to_vec(&response) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return RelayRequestOutcome {
                        encrypted_response: None,
                        error: Some(relay_error(
                            "relay_request_failed",
                            &format!("failed to serialize relay response: {error}"),
                            false,
                        )),
                    };
                }
            };
            crate::logging::info_with_fields(
                "daemon.relay_client",
                "relay daemon response serialized",
                serde_json::json!({
                    "request_kind": request_kind,
                    "byte_len": plaintext.len(),
                }),
            );
            match relay_crypto::encrypt_payload_for_peer(
                &daemon_private_key,
                &client_public_key,
                &plaintext,
            ) {
                Ok(encrypted_response) => {
                    crate::logging::info_with_fields(
                        "daemon.relay_client",
                        "relay daemon response encrypted",
                        serde_json::json!({
                            "request_kind": request_kind,
                            "byte_len": plaintext.len(),
                        }),
                    );
                    RelayRequestOutcome {
                        encrypted_response: Some(encrypted_response),
                        error: None,
                    }
                }
                Err(error) => RelayRequestOutcome {
                    encrypted_response: None,
                    error: Some(relay_error(
                        "relay_request_failed",
                        &format!("failed to encrypt relay response: {error}"),
                        false,
                    )),
                },
            }
        }
        Err(error) => RelayRequestOutcome {
            encrypted_response: None,
            error: Some(map_relay_error(&error)),
        },
    }
}

async fn dispatch_relay_client_request(
    router: &CommandRouter,
    command_sequence: &AtomicU64,
    caller_identity: Option<RelayCallerIdentity>,
    request: LocalDaemonRequest,
) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
    let sequence = command_sequence.fetch_add(1, Ordering::Relaxed);
    let command_id = format!(
        "relay-client-{}-{sequence}",
        crate::session::unix_epoch_ms()
    );
    let command = KernelCommand::from_local_request_with_caller(
        command_id,
        KernelCommandSource::RelayClient,
        caller_identity
            .map(KernelCaller::from_relay_identity)
            .unwrap_or_else(|| KernelCaller::for_source(&KernelCommandSource::RelayClient)),
        None,
        None,
        &request,
    );
    router.dispatch(command, request).await
}

async fn encrypt_json_response(
    router: &Arc<CommandRouter>,
    client_public_key: &str,
    value: serde_json::Value,
) -> Result<EncryptedRelayPayload, DaemonError> {
    let daemon_private_key = router.relay_private_key();
    let plaintext = serde_json::to_vec(&value).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay response",
        message: error.to_string(),
    })?;
    relay_crypto::encrypt_payload_for_peer(&daemon_private_key, client_public_key, &plaintext)
}

fn encrypt_peer_payload<T: serde::Serialize>(
    sender_private_key: &str,
    peer_public_key: &str,
    value: &T,
) -> Result<EncryptedRelayPayload, DaemonError> {
    let plaintext = serde_json::to_vec(value).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay peer payload",
        message: error.to_string(),
    })?;
    relay_crypto::encrypt_payload_for_peer(sender_private_key, peer_public_key, &plaintext)
}

fn send_outgoing_envelope(
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    envelope: RelayEnvelope,
) -> Result<(), DaemonError> {
    outgoing_tx
        .send(envelope)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "send relay envelope",
            message: error.to_string(),
        })
}

async fn set_connected(
    state: &Arc<RwLock<RelayClientState>>,
    outgoing_tx: mpsc::UnboundedSender<RelayEnvelope>,
) {
    let mut guard = state.write().await;
    guard.connected = true;
    guard.outgoing_tx = Some(outgoing_tx);
}

async fn publish_cloud_presence(router: &Arc<CommandRouter>, online: bool, reason: &str) {
    if let Err(error) = router.publish_cloud_kernel_presence(online).await {
        crate::logging::warn_with_fields(
            "daemon.relay_client",
            "failed to publish cloud relay presence",
            serde_json::json!({
                "online": online,
                "reason": reason,
                "error": error.to_string(),
            }),
        );
    }
}

async fn publish_offline_and_set_disconnected(
    router: &Arc<CommandRouter>,
    state: &Arc<RwLock<RelayClientState>>,
    reason: &str,
) {
    crate::logging::warn_with_fields(
        "daemon.relay_client",
        "relay socket disconnected",
        serde_json::json!({
            "reason": reason,
        }),
    );
    publish_cloud_presence(router, false, reason).await;
    set_disconnected(state).await;
}

async fn set_disconnected(state: &Arc<RwLock<RelayClientState>>) {
    let pending_peer = {
        let mut guard = state.write().await;
        guard.connected = false;
        guard.outgoing_tx = None;
        std::mem::take(&mut guard.pending_peer_requests)
    };
    for (_, sender) in pending_peer {
        let _ = sender.send(RelayPeerResponseEnvelope {
            from_daemon_id: String::new(),
            encrypted_response: None,
            error: Some(relay_error(
                "relay_disconnected",
                "relay connection closed before peer response arrived",
                true,
            )),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use arroba_relay::{protocol::ClientTarget, RelayConfig, RelayServer};
    use tokio::sync::oneshot;
    use tokio::time::{sleep, Duration};

    use crate::agent::CreateAgentRequest;
    use crate::app::RemoteLeaseRuntime;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::DaemonConfig;
    use crate::local::{
        AttachToSessionRequest, DetachFromSessionRequest, FocusAgentRequest,
        GetSessionStateRequest, ListSessionsRequest, LocalDaemonResponse, ResizeTerminalRequest,
        ResolveSessionRequest, RespondToInteractionRequest, UpdateSessionConfigRequest,
        ValidateWorkflowOutputRequest,
    };
    use crate::runtime::command::KernelCommand;
    use crate::session::CreateSessionRequest;
    use crate::transport::relay_crypto;
    use crate::transport::relay_discovery;
    use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    async fn relay_client_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().await
    }

    fn create_test_session(app: &mut DaemonApp, workspace: &str, worktree: &str) -> String {
        crate::app::KernelSessionService::new(app)
            .create_session(CreateSessionRequest::new(workspace, worktree))
            .expect("session should be created")
            .0
            .id()
            .to_string()
    }

    fn create_test_session_with_alias(
        app: &mut DaemonApp,
        workspace: &str,
        worktree: &str,
        alias: &str,
    ) -> (String, String) {
        let (session, agent) = crate::app::KernelSessionService::new(app)
            .create_session(CreateSessionRequest::new(workspace, worktree).with_alias(alias))
            .expect("session should be created");
        (session.id().to_string(), agent.id().to_string())
    }

    fn attach_test_client(
        app: &mut DaemonApp,
        session_id: &str,
        client_id: &str,
        capability_level: ClientCapabilityLevel,
    ) -> String {
        crate::app::KernelSessionService::new(app)
            .attach(AttachRequest::new(session_id, client_id, capability_level))
            .expect("session should attach")
            .id()
            .to_string()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_connector_registers_with_relay() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config.relay_token = Some("secret".to_string());
        config.relay_heartbeat_ms = 50;
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
        ));
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector_task = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app),
            Arc::clone(&state),
            shutdown_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

        {
            let guard = registry.read().await;
            assert!(guard.daemon(&config.daemon_id).is_some());
        }
        assert!(state.read().await.connected);

        let _ = shutdown_tx.send(true);
        connector_task.await.expect("connector task should join");
        sleep(Duration::from_millis(50)).await;
        {
            let guard = registry.read().await;
            assert!(guard.daemon(&config.daemon_id).is_none());
        }
        assert!(!state.read().await.connected);

        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn proxied_peer_requests_are_handled_through_relay() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config_a = DaemonConfig::for_tests();
        config_a.daemon_id = "daemon-a".to_string();
        config_a.daemon_alias = Some("alpha".to_string());
        config_a.host_machine_id = "machine-a".to_string();
        config_a.host_machine_alias = Some("machine-alpha".to_string());
        config_a.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_a.relay_token = Some("secret".to_string());
        config_a.relay_heartbeat_ms = 50;
        let app_a = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_a.clone()).expect("daemon A should bootstrap"),
        ));
        let state_a = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_a_tx, shutdown_a_rx) = watch::channel(false);
        let connector_a = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_a),
            Arc::clone(&state_a),
            shutdown_a_rx,
        ));

        let mut config_b = DaemonConfig::for_tests();
        config_b.daemon_id = "daemon-b".to_string();
        config_b.daemon_alias = Some("beta".to_string());
        config_b.host_machine_id = "machine-b".to_string();
        config_b.host_machine_alias = Some("machine-beta".to_string());
        config_b.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_b.relay_token = Some("secret".to_string());
        config_b.relay_heartbeat_ms = 50;
        let app_b = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_b.clone()).expect("daemon B should bootstrap"),
        ));
        let state_b = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_b_tx, shutdown_b_rx) = watch::channel(false);
        let connector_b = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_b),
            Arc::clone(&state_b),
            shutdown_b_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config_a.daemon_id).await;
        wait_for_daemon_registration(registry.clone(), &config_b.daemon_id).await;

        let kernel = relay_discovery::get_live_kernel(&config_a, "beta")
            .await
            .expect("live kernel lookup should succeed");
        assert_eq!(kernel.kernel_id, config_b.daemon_id);
        assert_eq!(kernel.public_key, config_b.relay_public_key);

        let response = send_peer_request_via_relay(
            &app_a,
            &state_a,
            ClientTarget {
                daemon_id: None,
                daemon_alias: Some("beta".to_string()),
            },
            RelayPeerRequest::Ping {
                value: "hello-remote-kernel".to_string(),
            },
        )
        .await
        .expect("peer request should succeed");
        assert_eq!(
            response,
            RelayPeerResponse::Pong {
                value: "hello-remote-kernel".to_string(),
                daemon_id: config_b.daemon_id.clone(),
            }
        );

        let _ = shutdown_a_tx.send(true);
        let _ = shutdown_b_tx.send(true);
        connector_a.await.expect("connector A should join");
        connector_b.await.expect("connector B should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execution_leases_are_managed_through_peer_transport() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config_a = DaemonConfig::for_tests();
        config_a.daemon_id = "daemon-home".to_string();
        config_a.daemon_alias = Some("home".to_string());
        config_a.host_machine_id = "machine-home".to_string();
        config_a.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_a.relay_token = Some("secret".to_string());
        config_a.relay_heartbeat_ms = 50;
        let app_a = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_a.clone()).expect("home daemon should bootstrap"),
        ));
        let state_a = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_a_tx, shutdown_a_rx) = watch::channel(false);
        let connector_a = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_a),
            Arc::clone(&state_a),
            shutdown_a_rx,
        ));

        let mut config_b = DaemonConfig::for_tests();
        config_b.daemon_id = "daemon-worker".to_string();
        config_b.daemon_alias = Some("worker".to_string());
        config_b.host_machine_id = "machine-worker".to_string();
        config_b.host_machine_alias = Some("remote-builder".to_string());
        config_b.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_b.relay_token = Some("secret".to_string());
        config_b.relay_heartbeat_ms = 50;
        config_b.accept_remote_leases = true;
        let app_b = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_b.clone()).expect("worker daemon should bootstrap"),
        ));
        let state_b = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_b_tx, shutdown_b_rx) = watch::channel(false);
        let connector_b = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_b),
            Arc::clone(&state_b),
            shutdown_b_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config_a.daemon_id).await;
        wait_for_daemon_registration(registry.clone(), &config_b.daemon_id).await;

        let created = send_peer_request_via_relay(
            &app_a,
            &state_a,
            ClientTarget {
                daemon_id: None,
                daemon_alias: Some("worker".to_string()),
            },
            RelayPeerRequest::CreateExecutionLease {
                home_kernel_id: config_a.daemon_id.clone(),
                home_session_id: "session-remote-1".to_string(),
                home_agent_id: "agent-remote-1".to_string(),
                owner_user_id: "user-home".to_string(),
            },
        )
        .await
        .expect("execution lease should be created remotely");
        let lease = match created {
            RelayPeerResponse::ExecutionLeaseCreated { lease } => lease,
            other => panic!("unexpected peer response: {other:?}"),
        };
        assert_eq!(lease.home_kernel_id, config_a.daemon_id);
        assert_eq!(lease.worker_kernel_id, config_b.daemon_id);
        assert_eq!(lease.machine_id, config_b.host_machine_id);
        {
            let mut app = app_b.lock().await;
            assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 1);
        }

        let destroyed = send_peer_request_via_relay(
            &app_a,
            &state_a,
            ClientTarget {
                daemon_id: Some(config_b.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::DestroyExecutionLease {
                lease_id: lease.id.clone(),
            },
        )
        .await
        .expect("execution lease should be destroyed remotely");
        assert_eq!(
            destroyed,
            RelayPeerResponse::ExecutionLeaseDestroyed {
                lease_id: lease.id.clone(),
            }
        );
        {
            let mut app = app_b.lock().await;
            assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 0);
        }

        let _ = shutdown_a_tx.send(true);
        let _ = shutdown_b_tx.send(true);
        connector_a.await.expect("connector A should join");
        connector_b.await.expect("connector B should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn leased_agents_are_spawned_and_destroyed_through_peer_transport() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config_a = DaemonConfig::for_tests();
        config_a.daemon_id = "daemon-home".to_string();
        config_a.daemon_alias = Some("home".to_string());
        config_a.host_machine_id = "machine-home".to_string();
        config_a.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_a.relay_token = Some("secret".to_string());
        config_a.relay_heartbeat_ms = 50;
        let app_a = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_a.clone()).expect("home daemon should bootstrap"),
        ));
        let (home_session_id, home_agent_id) = {
            let mut app = app_a.lock().await;
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("home session should be created");
            (session.id().to_string(), agent.id().to_string())
        };
        let state_a = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_a_tx, shutdown_a_rx) = watch::channel(false);
        let connector_a = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_a),
            Arc::clone(&state_a),
            shutdown_a_rx,
        ));

        let mut config_b = DaemonConfig::for_tests();
        config_b.daemon_id = "daemon-worker".to_string();
        config_b.daemon_alias = Some("worker".to_string());
        config_b.host_machine_id = "machine-worker".to_string();
        config_b.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_b.relay_token = Some("secret".to_string());
        config_b.relay_heartbeat_ms = 50;
        config_b.accept_remote_leases = true;
        let app_b = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_b.clone()).expect("worker daemon should bootstrap"),
        ));
        let state_b = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_b_tx, shutdown_b_rx) = watch::channel(false);
        let connector_b = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_b),
            Arc::clone(&state_b),
            shutdown_b_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config_a.daemon_id).await;
        wait_for_daemon_registration(registry.clone(), &config_b.daemon_id).await;

        let lease = match send_peer_request_via_relay(
            &app_a,
            &state_a,
            ClientTarget {
                daemon_id: Some(config_b.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::CreateExecutionLease {
                home_kernel_id: config_a.daemon_id.clone(),
                home_session_id: home_session_id.clone(),
                home_agent_id: home_agent_id.clone(),
                owner_user_id: "user-home".to_string(),
            },
        )
        .await
        .expect("execution lease should be created remotely")
        {
            RelayPeerResponse::ExecutionLeaseCreated { lease } => lease,
            other => panic!("unexpected peer response: {other:?}"),
        };

        let leased_agent = match send_peer_request_via_relay(
            &app_a,
            &state_a,
            ClientTarget {
                daemon_id: None,
                daemon_alias: Some("worker".to_string()),
            },
            RelayPeerRequest::SpawnLeasedAgent {
                lease_id: lease.id.clone(),
                provider: "opencode".to_string(),
                model: Some("kimi2.5".to_string()),
                effort: Some("medium".to_string()),
                execution_mode: None,
                permission_level: None,
                worktree_id: None,
                worktree_placement: None,
            },
        )
        .await
        .expect("leased agent should be spawned remotely")
        {
            RelayPeerResponse::LeasedAgentSpawned { leased_agent } => leased_agent,
            other => panic!("unexpected peer response: {other:?}"),
        };
        assert_eq!(leased_agent.lease_id, lease.id);
        assert_eq!(leased_agent.provider, "opencode");
        {
            let mut app = app_b.lock().await;
            assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 1);
        }

        let destroyed = send_peer_request_via_relay(
            &app_a,
            &state_a,
            ClientTarget {
                daemon_id: Some(config_b.daemon_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::DestroyLeasedAgent {
                leased_agent_id: leased_agent.id.clone(),
            },
        )
        .await
        .expect("leased agent should be destroyed remotely");
        assert_eq!(
            destroyed,
            RelayPeerResponse::LeasedAgentDestroyed {
                leased_agent_id: leased_agent.id.clone(),
            }
        );
        {
            let mut app = app_b.lock().await;
            assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 0);
        }

        let _ = shutdown_a_tx.send(true);
        let _ = shutdown_b_tx.send(true);
        connector_a.await.expect("connector A should join");
        connector_b.await.expect("connector B should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agents_can_be_spawned_on_a_remote_machine_and_cleaned_up() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config_home = DaemonConfig::for_tests();
        config_home.daemon_id = "daemon-home".to_string();
        config_home.daemon_alias = Some("home".to_string());
        config_home.host_machine_id = "machine-home".to_string();
        config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_home.relay_token = Some("secret".to_string());
        config_home.relay_heartbeat_ms = 50;
        let app_home = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
        ));
        let state_home = {
            let app = app_home.lock().await;
            app.relay_client_state()
        };
        let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
        let connector_home = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_home),
            Arc::clone(&state_home),
            shutdown_home_rx,
        ));
        let mut config_worker = DaemonConfig::for_tests();
        config_worker.daemon_id = "daemon-worker".to_string();
        config_worker.daemon_alias = Some("worker".to_string());
        config_worker.host_machine_id = "machine-worker".to_string();
        config_worker.host_machine_alias = Some("builder-west".to_string());
        config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_worker.relay_token = Some("secret".to_string());
        config_worker.relay_heartbeat_ms = 50;
        config_worker.accept_remote_leases = true;
        let app_worker = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
        ));
        let state_worker = {
            let app = app_worker.lock().await;
            app.relay_client_state()
        };
        let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
        let connector_worker = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_worker),
            Arc::clone(&state_worker),
            shutdown_worker_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
        wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

        let worker_kernels =
            relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
                .await
                .expect("worker kernels should be discoverable");
        let provider = worker_kernels
            .first()
            .and_then(|kernel| {
                kernel
                    .available_providers
                    .iter()
                    .find(|provider| provider.as_str() == "managed-dev-stub")
            })
            .cloned()
            .expect("worker should advertise managed-dev-stub");
        refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
            .await
            .expect("home remote inventory should refresh");

        let session_id = {
            let mut app = app_home.lock().await;
            let (session, _) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("home session should be created");
            session.id().to_string()
        };

        let remote_agent = {
            let mut app = app_home.lock().await;
            crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(
                    CreateAgentRequest::new(&session_id, &provider)
                        .with_alias("remote-reviewer")
                        .with_model("default")
                        .with_effort("medium")
                        .with_kernel(&config_worker.daemon_id),
                )
                .expect("remote agent should spawn")
        };

        let remote_execution = remote_agent
            .remote_execution()
            .cloned()
            .expect("remote binding should be present");
        assert_eq!(remote_execution.worker_kernel_id, config_worker.daemon_id);
        assert_eq!(
            remote_execution.worker_machine_id,
            config_worker.host_machine_id
        );

        {
            let mut app = app_worker.lock().await;
            assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 1);
            assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 1);
        }

        {
            let mut app = app_home.lock().await;
            let destroyed = crate::app::KernelSessionService::new(&mut app)
                .destroy_agent(remote_agent.id())
                .expect("remote agent should destroy");
            assert_eq!(destroyed.id(), remote_agent.id());
        }

        {
            let mut app = app_worker.lock().await;
            assert_eq!(RemoteLeaseRuntime::new(&mut app).execution_lease_count(), 0);
            assert_eq!(RemoteLeaseRuntime::new(&mut app).leased_agent_count(), 0);
        }

        let _ = shutdown_home_tx.send(true);
        let _ = shutdown_worker_tx.send(true);
        connector_home.await.expect("home connector should join");
        connector_worker
            .await
            .expect("worker connector should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remote_machine_agents_execute_prompts_through_the_home_session() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config_home = DaemonConfig::for_tests();
        config_home.daemon_id = "daemon-home".to_string();
        config_home.daemon_alias = Some("home".to_string());
        config_home.host_machine_id = "machine-home".to_string();
        config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_home.relay_token = Some("secret".to_string());
        config_home.relay_heartbeat_ms = 50;
        let mut config_worker = DaemonConfig::for_tests();
        config_worker.daemon_id = "daemon-worker".to_string();
        config_worker.daemon_alias = Some("worker".to_string());
        config_worker.host_machine_id = "machine-worker".to_string();
        config_worker.host_machine_alias = Some("builder-west".to_string());
        config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_worker.relay_token = Some("secret".to_string());
        config_worker.relay_heartbeat_ms = 50;
        config_worker.accept_remote_leases = true;
        let app_worker = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
        ));
        let state_worker = {
            let app = app_worker.lock().await;
            app.relay_client_state()
        };
        let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
        let connector_worker = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_worker),
            Arc::clone(&state_worker),
            shutdown_worker_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

        let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
            .await
            .expect("worker kernels should be discoverable")
            .first()
            .and_then(|kernel| {
                kernel
                    .available_providers
                    .iter()
                    .find(|provider| provider.as_str() == "managed-dev-stub")
            })
            .cloned()
            .expect("worker should advertise managed-dev-stub");

        let app_home = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
        ));
        let state_home = {
            let app = app_home.lock().await;
            app.relay_client_state()
        };
        let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
        let connector_home = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_home),
            Arc::clone(&state_home),
            shutdown_home_rx,
        ));
        wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
        refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
            .await
            .expect("home remote inventory should refresh");

        let (session_id, attachment_id) = {
            let mut app_home = app_home.lock().await;
            let (session, _) = crate::app::KernelSessionService::new(&mut app_home)
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("home session should be created");
            let attachment = crate::app::KernelSessionService::new(&mut app_home)
                .attach(AttachRequest::new(
                    session.id(),
                    "home-client",
                    ClientCapabilityLevel::InteractiveStructured,
                ))
                .expect("home attachment should attach");
            (session.id().to_string(), attachment.id().to_string())
        };

        let remote_agent_id = {
            let mut app_home = app_home.lock().await;
            crate::app::KernelSessionService::new(&mut app_home)
                .spawn_agent(
                    CreateAgentRequest::new(&session_id, &provider)
                        .with_alias("remote-reviewer")
                        .with_model("default")
                        .with_effort("medium")
                        .with_kernel(&config_worker.daemon_id),
                )
                .expect("remote agent should spawn")
                .id()
                .to_string()
        };

        let outcome = app_home
            .lock()
            .await
            .submit_prompt(
                &session_id,
                &attachment_id,
                Some(&remote_agent_id),
                "remote prompt over home session\n",
                Vec::new(),
            )
            .expect("remote prompt should submit");
        assert!(matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Started { .. }
        ));

        let completion = app_home
            .lock()
            .await
            .complete_active_prompt(&session_id, &remote_agent_id, None)
            .expect("remote prompt should complete");
        assert_eq!(completion.completed.target_agent_id(), remote_agent_id);
        assert_eq!(
            completion.completed.prompt(),
            "remote prompt over home session\n"
        );

        let _ = shutdown_home_tx.send(true);
        let _ = shutdown_worker_tx.send(true);
        connector_home.await.expect("home connector should join");
        connector_worker
            .await
            .expect("worker connector should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remote_machine_agents_materialize_file_attachments_on_the_worker() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config_home = DaemonConfig::for_tests();
        config_home.daemon_id = "daemon-home".to_string();
        config_home.daemon_alias = Some("home".to_string());
        config_home.host_machine_id = "machine-home".to_string();
        config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_home.relay_token = Some("secret".to_string());
        config_home.relay_heartbeat_ms = 50;
        let mut config_worker = DaemonConfig::for_tests();
        config_worker.daemon_id = "daemon-worker".to_string();
        config_worker.daemon_alias = Some("worker".to_string());
        config_worker.host_machine_id = "machine-worker".to_string();
        config_worker.host_machine_alias = Some("builder-west".to_string());
        config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_worker.relay_token = Some("secret".to_string());
        config_worker.relay_heartbeat_ms = 50;
        config_worker.accept_remote_leases = true;
        let app_worker = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
        ));
        let state_worker = {
            let app = app_worker.lock().await;
            app.relay_client_state()
        };
        let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
        let connector_worker = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_worker),
            Arc::clone(&state_worker),
            shutdown_worker_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

        let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
            .await
            .expect("worker kernels should be discoverable")
            .first()
            .and_then(|kernel| {
                kernel
                    .available_providers
                    .iter()
                    .find(|provider| provider.as_str() == "managed-dev-stub")
            })
            .cloned()
            .expect("worker should advertise managed-dev-stub");

        let app_home = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
        ));
        let state_home = {
            let app = app_home.lock().await;
            app.relay_client_state()
        };
        let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
        let connector_home = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_home),
            Arc::clone(&state_home),
            shutdown_home_rx,
        ));
        wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
        refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
            .await
            .expect("home remote inventory should refresh");
        let (session_id, attachment_id, remote_agent_id, remote_leased_agent_id) = {
            let mut app_home = app_home.lock().await;
            let (session, _) = crate::app::KernelSessionService::new(&mut app_home)
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("home session should be created");
            let attachment = crate::app::KernelSessionService::new(&mut app_home)
                .attach(AttachRequest::new(
                    session.id(),
                    "home-client",
                    ClientCapabilityLevel::InteractiveStructured,
                ))
                .expect("home attachment should attach");
            let remote_agent = crate::app::KernelSessionService::new(&mut app_home)
                .spawn_agent(
                    CreateAgentRequest::new(session.id(), &provider)
                        .with_alias("remote-reviewer")
                        .with_kernel(&config_worker.daemon_id),
                )
                .expect("remote agent should spawn");
            let leased_agent_id = remote_agent
                .remote_execution()
                .expect("remote binding should exist")
                .leased_agent_id
                .clone();
            (
                session.id().to_string(),
                attachment.id().to_string(),
                remote_agent.id().to_string(),
                leased_agent_id,
            )
        };

        let source_path = std::env::temp_dir().join(format!(
            "arroba-remote-attachment-{}.txt",
            crate::session::unix_epoch_ms()
        ));
        std::fs::write(&source_path, b"remote attachment body")
            .expect("source attachment should be written");

        let outcome = app_home
            .lock()
            .await
            .submit_prompt(
                &session_id,
                &attachment_id,
                Some(&remote_agent_id),
                "prompt with attachment\n",
                vec![crate::session::PromptAttachment::new(
                    format!("file://{}", source_path.display()),
                    "text/plain",
                    Some("note.txt".to_string()),
                )],
            )
            .expect("remote prompt should submit");
        assert!(matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Started { .. }
        ));

        let worker_attachments = {
            let mut app = app_worker.lock().await;
            RemoteLeaseRuntime::new(&mut app)
                .leased_agent_active_prompt_attachments(&remote_leased_agent_id)
                .expect("worker prompt attachments should be available")
        };
        assert_eq!(worker_attachments.len(), 1);
        let materialized = &worker_attachments[0];
        assert_eq!(materialized.filename(), Some("note.txt"));
        assert_eq!(materialized.mime(), "text/plain");
        assert!(materialized.url().starts_with("file://"));
        assert_ne!(
            materialized.url(),
            format!("file://{}", source_path.display())
        );
        let worker_path = materialized.url().trim_start_matches("file://");
        let worker_bytes = std::fs::read(worker_path).expect("worker attachment should exist");
        assert_eq!(worker_bytes, b"remote attachment body");

        let _ = std::fs::remove_file(&source_path);
        let _ = shutdown_home_tx.send(true);
        let _ = shutdown_worker_tx.send(true);
        connector_home.await.expect("home connector should join");
        connector_worker
            .await
            .expect("worker connector should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remote_machine_agents_cancel_prompts_through_the_home_session() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config_home = DaemonConfig::for_tests();
        config_home.daemon_id = "daemon-home".to_string();
        config_home.daemon_alias = Some("home".to_string());
        config_home.host_machine_id = "machine-home".to_string();
        config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_home.relay_token = Some("secret".to_string());
        config_home.relay_heartbeat_ms = 50;
        let mut config_worker = DaemonConfig::for_tests();
        config_worker.daemon_id = "daemon-worker".to_string();
        config_worker.daemon_alias = Some("worker".to_string());
        config_worker.host_machine_id = "machine-worker".to_string();
        config_worker.host_machine_alias = Some("builder-west".to_string());
        config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_worker.relay_token = Some("secret".to_string());
        config_worker.relay_heartbeat_ms = 50;
        config_worker.accept_remote_leases = true;
        let app_worker = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
        ));
        let state_worker = {
            let app = app_worker.lock().await;
            app.relay_client_state()
        };
        let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
        let connector_worker = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_worker),
            Arc::clone(&state_worker),
            shutdown_worker_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

        let provider = relay_discovery::list_live_kernels_for_machine(&config_home, "builder-west")
            .await
            .expect("worker kernels should be discoverable")
            .first()
            .and_then(|kernel| {
                kernel
                    .available_providers
                    .iter()
                    .find(|provider| provider.as_str() == "managed-dev-stub")
            })
            .cloned()
            .expect("worker should advertise managed-dev-stub");

        let app_home = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
        ));
        let state_home = {
            let app = app_home.lock().await;
            app.relay_client_state()
        };
        let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
        let connector_home = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_home),
            Arc::clone(&state_home),
            shutdown_home_rx,
        ));
        wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
        refresh_remote_inventory_projection_for_app_with_relay_state(&app_home)
            .await
            .expect("home remote inventory should refresh");
        let (session_id, attachment_id) = {
            let mut app_home = app_home.lock().await;
            let (session, _) = crate::app::KernelSessionService::new(&mut app_home)
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("home session should be created");
            let attachment = crate::app::KernelSessionService::new(&mut app_home)
                .attach(AttachRequest::new(
                    session.id(),
                    "home-client",
                    ClientCapabilityLevel::InteractiveStructured,
                ))
                .expect("home attachment should attach");
            (session.id().to_string(), attachment.id().to_string())
        };
        let remote_agent_id = {
            let mut app_home = app_home.lock().await;
            crate::app::KernelSessionService::new(&mut app_home)
                .spawn_agent(
                    CreateAgentRequest::new(&session_id, &provider)
                        .with_alias("remote-reviewer")
                        .with_model("default")
                        .with_kernel(&config_worker.daemon_id),
                )
                .expect("remote agent should spawn")
                .id()
                .to_string()
        };

        let outcome = app_home
            .lock()
            .await
            .submit_prompt(
                &session_id,
                &attachment_id,
                Some(&remote_agent_id),
                "cancel this remote prompt\n",
                Vec::new(),
            )
            .expect("remote prompt should submit");
        assert!(matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Started { .. }
        ));

        let cancellation = app_home
            .lock()
            .await
            .cancel_active_prompt(&session_id, &attachment_id)
            .expect("remote prompt should cancel");
        assert_eq!(cancellation.prompt.target_agent_id(), remote_agent_id);
        assert_eq!(
            cancellation.prompt.status(),
            crate::session::PromptStatus::Cancelling
        );

        let _ = shutdown_home_tx.send(true);
        let _ = shutdown_worker_tx.send(true);
        connector_home.await.expect("home connector should join");
        connector_worker
            .await
            .expect("worker connector should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn incoming_peer_events_project_runtime_to_the_home_session() {
        let _relay_test_guard = relay_client_test_guard().await;
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should bootstrap"),
        ));
        let (session_id, agent_id, attachment_id, daemon_public_key) = {
            let mut app = app.lock().await;
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("session should be created");
            let attachment = crate::app::KernelSessionService::new(&mut app)
                .attach(AttachRequest::new(
                    session.id(),
                    "home-client",
                    ClientCapabilityLevel::InteractiveStructured,
                ))
                .expect("attachment should attach");
            (
                session.id().to_string(),
                agent.id().to_string(),
                attachment.id().to_string(),
                relay_crypto::public_key_from_private_key_base64(&app.config().relay_private_key)
                    .expect("daemon public key should derive"),
            )
        };
        let sender_private_key = relay_crypto::generate_private_key_base64();
        let plaintext = serde_json::to_vec(&RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id: session_id.clone(),
            home_agent_id: agent_id.clone(),
            provider_run_id: "remote:worker:provider-run-1".to_string(),
            output_chunks: vec![crate::transport::relay_peer::RelayProjectedOutputChunk {
                kind: crate::terminal::TerminalOutputKind::ProviderOutput,
                merge_key: Some("assistant-1".to_string()),
                bytes: b"remote output".to_vec(),
            }],
            notices: vec!["remote notice".to_string()],
            completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
                message_id: "assistant-msg-1".to_string(),
                completed_at_ms: 1234,
            }],
        })
        .expect("peer event should serialize");
        let encrypted_event = relay_crypto::encrypt_payload_for_peer(
            &sender_private_key,
            &daemon_public_key,
            &plaintext,
        )
        .expect("peer event should encrypt");

        let provider_runtime_lanes = {
            let app = app.lock().await;
            app.provider_run_operation_lanes()
        };
        let router = Arc::new(CommandRouter::with_interactive_capacity_and_provider_lanes(
            Arc::clone(&app),
            INTERACTIVE_COMMAND_QUEUE_LIMIT,
            provider_runtime_lanes,
        ));
        handle_daemon_peer_event(&router, encrypted_event)
            .await
            .expect("peer event should project");

        let mut app = app.lock().await;
        let outputs = app
            .terminal_mut()
            .drain_output_records(&session_id, &attachment_id);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].agent_id.as_deref(), Some(agent_id.as_str()));

        let notices = app
            .terminal_mut()
            .drain_notice_records(&session_id, &attachment_id);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].agent_id.as_deref(), Some(agent_id.as_str()));

        let completions = app
            .terminal_mut()
            .drain_completion_records(&session_id, &attachment_id);
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].agent_id.as_deref(), Some(agent_id.as_str()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forwarded_native_interactions_resolve_back_to_worker_over_temporary_connection() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config_home = DaemonConfig::for_tests();
        config_home.daemon_id = "daemon-home".to_string();
        config_home.daemon_alias = Some("home".to_string());
        config_home.host_machine_id = "machine-home".to_string();
        config_home.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_home.relay_token = Some("secret".to_string());
        config_home.relay_heartbeat_ms = 50;
        let app_home = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap"),
        ));
        let state_home = {
            let app = app_home.lock().await;
            app.relay_client_state()
        };
        let (shutdown_home_tx, shutdown_home_rx) = watch::channel(false);
        let connector_home = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_home),
            Arc::clone(&state_home),
            shutdown_home_rx,
        ));

        let mut config_worker = DaemonConfig::for_tests();
        config_worker.daemon_id = "daemon-worker".to_string();
        config_worker.daemon_alias = Some("worker".to_string());
        config_worker.host_machine_id = "machine-worker".to_string();
        config_worker.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config_worker.relay_token = Some("secret".to_string());
        config_worker.relay_heartbeat_ms = 50;
        let app_worker = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config_worker.clone()).expect("worker daemon should bootstrap"),
        ));
        let state_worker = {
            let app = app_worker.lock().await;
            app.relay_client_state()
        };
        let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
        let connector_worker = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_worker),
            Arc::clone(&state_worker),
            shutdown_worker_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config_home.daemon_id).await;
        wait_for_daemon_registration(registry.clone(), &config_worker.daemon_id).await;

        let (home_session_id, home_agent_id) = {
            let mut app = app_home.lock().await;
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("home session should be created");
            (session.id().to_string(), agent.id().to_string())
        };
        let interaction = crate::session::RuntimeInteraction::new(
            "native-test-interaction",
            "worker-agent",
            crate::session::RuntimeInteractionKind::Permission,
            crate::session::RuntimeInteractionLevel::Warning,
            Some("Synthetic permission".to_string()),
            "Approve synthetic forwarded native interaction?",
            vec![
                crate::session::RuntimeInteractionChoice::new(
                    "allow_once",
                    "Allow",
                    "allowed once",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
                ),
                crate::session::RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "denied",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            None,
            None,
        );
        let context = crate::transport::relay_peer::RemoteNativeInteractionContext {
            home_session_id: home_session_id.clone(),
            home_agent_id: home_agent_id.clone(),
            leased_agent_id: "leased-agent-test".to_string(),
            worker_provider_run_id: "provider-run-test".to_string(),
        };

        let worker_request = {
            let config_worker = config_worker.clone();
            tokio::spawn(async move {
                send_peer_request_via_temporary_connection(
                    &config_worker,
                    ClientTarget {
                        daemon_id: Some("daemon-home".to_string()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::ForwardNativeInteraction {
                        context,
                        interaction,
                    },
                )
                .await
            })
        };

        let interaction_id =
            wait_for_active_interaction(Arc::clone(&app_home), &home_session_id, &home_agent_id)
                .await;
        let respond_request =
            crate::local::LocalDaemonRequest::RespondToInteraction(RespondToInteractionRequest {
                session_id: home_session_id.clone(),
                interaction_id,
                choice_id: "allow_once".to_string(),
                custom_reply: None,
            });
        let provider_runtime_lanes = {
            let app = app_home.lock().await;
            app.provider_run_operation_lanes()
        };
        let router = CommandRouter::with_interactive_capacity_and_provider_lanes(
            Arc::clone(&app_home),
            INTERACTIVE_COMMAND_QUEUE_LIMIT,
            provider_runtime_lanes,
        );
        router
            .dispatch(
                KernelCommand::from_local_request(
                    "respond-native-test",
                    None,
                    None,
                    &respond_request,
                ),
                respond_request,
            )
            .await
            .expect("home interaction response should be accepted");

        let response = worker_request
            .await
            .expect("worker peer request task should join")
            .expect("worker should receive native interaction response");
        match response {
            RelayPeerResponse::NativeInteractionResolved { resolution } => {
                assert_eq!(resolution.status, "answered");
                assert_eq!(resolution.choice_id.as_deref(), Some("allow_once"));
                assert_eq!(resolution.reply.as_deref(), Some("allowed once"));
            }
            other => panic!("unexpected peer response: {other:?}"),
        }

        let _ = shutdown_home_tx.send(true);
        let _ = shutdown_worker_tx.send(true);
        connector_home.await.expect("home connector should join");
        connector_worker
            .await
            .expect("worker connector should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn proxied_session_requests_are_handled_through_relay() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config.relay_token = Some("secret".to_string());
        config.relay_heartbeat_ms = 50;
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
        ));
        let created_session_id = {
            let mut app = app.lock().await;
            create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test")
        };
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector_task = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app),
            Arc::clone(&state),
            shutdown_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut client_socket, _) = connect_async(&url)
            .await
            .expect("client should connect to relay");
        send_client_envelope(
            &mut client_socket,
            &RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some(config.daemon_id.clone()),
                    daemon_alias: None,
                },
            },
        )
        .await;
        let daemon_public_key = expect_client_connected(&mut client_socket).await;

        let list_request_private_key = send_client_request(
            &mut client_socket,
            "list-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::ListSessions(ListSessionsRequest),
        )
        .await;
        let list_response =
            expect_client_response(&mut client_socket, "list-1", &list_request_private_key).await;
        assert!(matches!(
            list_response,
            LocalDaemonResponse::SessionsListed { sessions } if sessions.iter().any(|session| session.id() == created_session_id)
        ));
        assert!(
            app.lock()
                .await
                .session_state_projection_store()
                .has_warmed_list(),
            "relay daemon requests should enter through the command router and warm projections"
        );

        let state_request_private_key = send_client_request(
            &mut client_socket,
            "state-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                session_id: created_session_id.clone(),
            }),
        )
        .await;
        let state_response =
            expect_client_response(&mut client_socket, "state-1", &state_request_private_key).await;
        assert!(matches!(
            state_response,
            LocalDaemonResponse::SessionState { session, .. } if session.id() == created_session_id
        ));

        let attach_request_private_key = send_client_request(
            &mut client_socket,
            "attach-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
                session_id: created_session_id.clone(),
                client_id: "relay-client".to_string(),
                capability_level: ClientCapabilityLevel::MessageTransport,
            }),
        )
        .await;
        let attach_response =
            expect_client_response(&mut client_socket, "attach-1", &attach_request_private_key)
                .await;
        assert!(matches!(
            attach_response,
            LocalDaemonResponse::SessionAttached { attachment } if attachment.session_id() == created_session_id
        ));

        let schema_path = std::env::temp_dir().join(format!(
            "arroba-relay-validate-schema-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &schema_path,
            r#"{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}}"#,
        )
        .expect("schema should write");
        let validate_request_private_key = send_client_request(
            &mut client_socket,
            "validate-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::ValidateWorkflowOutput(ValidateWorkflowOutputRequest {
                session_id: created_session_id.clone(),
                output_schema_ref: schema_path.display().to_string(),
                output_json: r#"{"ok":true}"#.to_string(),
                validation_policy: None,
            }),
        )
        .await;
        let validate_response = expect_client_response(
            &mut client_socket,
            "validate-1",
            &validate_request_private_key,
        )
        .await;
        assert!(matches!(
            validate_response,
            LocalDaemonResponse::WorkflowOutputValidated {
                valid: true,
                warning: None
            }
        ));

        let _ = shutdown_tx.send(true);
        connector_task.await.expect("connector task should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn proxied_session_subscriptions_are_forwarded_through_relay() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config.relay_token = Some("secret".to_string());
        config.relay_heartbeat_ms = 50;
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
        ));
        let created_session_id = {
            let mut app = app.lock().await;
            create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test")
        };
        let attachment_id = {
            let mut app = app.lock().await;
            attach_test_client(
                &mut app,
                &created_session_id,
                "relay-client",
                ClientCapabilityLevel::MessageTransport,
            )
        };
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector_task = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app),
            Arc::clone(&state),
            shutdown_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut client_socket, _) = connect_async(&url)
            .await
            .expect("client should connect to relay");
        send_client_envelope(
            &mut client_socket,
            &RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some(config.daemon_id.clone()),
                    daemon_alias: None,
                },
            },
        )
        .await;
        let _daemon_public_key = expect_client_connected(&mut client_socket).await;

        let subscription_private_key = relay_crypto::generate_private_key_base64();
        let subscription_public_key =
            relay_crypto::public_key_from_private_key_base64(&subscription_private_key)
                .expect("subscription public key should derive");
        send_client_envelope(
            &mut client_socket,
            &RelayEnvelope::ClientSubscribe {
                request_id: "sub-1".to_string(),
                subscription_id: "subscription-1".to_string(),
                target: ClientTarget {
                    daemon_id: Some(config.daemon_id.clone()),
                    daemon_alias: None,
                },
                session_id: created_session_id.clone(),
                attachment_id: attachment_id.clone(),
                client_public_key: subscription_public_key.clone(),
                subscription_scope: None,
                resume_from_event_id: None,
            },
        )
        .await;
        let subscribe_response =
            expect_json_client_response(&mut client_socket, "sub-1", &subscription_private_key)
                .await;
        assert_eq!(subscribe_response["ok"], serde_json::json!(true));

        let event = expect_client_event(&mut client_socket, &subscription_private_key).await;
        assert_eq!(event["event"], serde_json::json!("session_snapshot"));
        assert_eq!(
            event["session"]["id"],
            serde_json::json!(created_session_id)
        );

        let _ = shutdown_tx.send(true);
        connector_task.await.expect("connector task should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn relay_subscription_replays_recent_events_after_resume_cursor() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config.relay_token = Some("secret".to_string());
        config.relay_heartbeat_ms = 50;
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
        ));
        let created_session_id = {
            let mut app = app.lock().await;
            create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test")
        };
        let attachment_id = {
            let mut app = app.lock().await;
            attach_test_client(
                &mut app,
                &created_session_id,
                "relay-client",
                ClientCapabilityLevel::MessageTransport,
            )
        };
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector_task = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app),
            Arc::clone(&state),
            shutdown_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut client_socket, _) = connect_async(&url)
            .await
            .expect("client should connect to relay");
        send_client_envelope(
            &mut client_socket,
            &RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some(config.daemon_id.clone()),
                    daemon_alias: None,
                },
            },
        )
        .await;
        let _daemon_public_key = expect_client_connected(&mut client_socket).await;

        let subscription_private_key = relay_crypto::generate_private_key_base64();
        let subscription_public_key =
            relay_crypto::public_key_from_private_key_base64(&subscription_private_key)
                .expect("subscription public key should derive");
        send_client_envelope(
            &mut client_socket,
            &RelayEnvelope::ClientSubscribe {
                request_id: "sub-1".to_string(),
                subscription_id: "subscription-1".to_string(),
                target: ClientTarget {
                    daemon_id: Some(config.daemon_id.clone()),
                    daemon_alias: None,
                },
                session_id: created_session_id.clone(),
                attachment_id: attachment_id.clone(),
                client_public_key: subscription_public_key.clone(),
                subscription_scope: None,
                resume_from_event_id: None,
            },
        )
        .await;
        let _ = expect_json_client_response(&mut client_socket, "sub-1", &subscription_private_key)
            .await;
        let first_event =
            expect_client_event_envelope(&mut client_socket, &subscription_private_key).await;
        assert_eq!(
            first_event.1["event"],
            serde_json::json!("session_snapshot")
        );

        send_client_envelope(
            &mut client_socket,
            &RelayEnvelope::ClientUnsubscribe {
                request_id: "unsub-1".to_string(),
                subscription_id: "subscription-1".to_string(),
                client_public_key: subscription_public_key.clone(),
            },
        )
        .await;
        let _ =
            expect_json_client_response(&mut client_socket, "unsub-1", &subscription_private_key)
                .await;

        send_client_envelope(
            &mut client_socket,
            &RelayEnvelope::ClientSubscribe {
                request_id: "sub-2".to_string(),
                subscription_id: "subscription-1".to_string(),
                target: ClientTarget {
                    daemon_id: Some(config.daemon_id.clone()),
                    daemon_alias: None,
                },
                session_id: created_session_id.clone(),
                attachment_id: attachment_id.clone(),
                client_public_key: subscription_public_key,
                subscription_scope: None,
                resume_from_event_id: Some(first_event.0),
            },
        )
        .await;
        let resume_response =
            expect_json_client_response(&mut client_socket, "sub-2", &subscription_private_key)
                .await;
        assert_eq!(
            resume_response["resumed_from_event_id"],
            serde_json::json!(first_event.0)
        );
        let resumed_event = expect_named_client_event(
            &mut client_socket,
            &subscription_private_key,
            "transport_resumed",
        )
        .await;
        assert_eq!(
            resumed_event.1["resumed_from_event_id"],
            serde_json::json!(first_event.0)
        );

        let _ = shutdown_tx.send(true);
        connector_task.await.expect("connector task should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn relay_subscription_emits_replay_gap_and_snapshot_for_stale_cursor() {
        let _relay_test_guard = relay_client_test_guard().await;
        let config = DaemonConfig::for_tests();
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
        ));
        let created_session_id = {
            let mut app = app.lock().await;
            create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test")
        };
        let attachment_id = {
            let mut app = app.lock().await;
            attach_test_client(
                &mut app,
                &created_session_id,
                "relay-client",
                ClientCapabilityLevel::MessageTransport,
            )
        };
        let provider_runtime_lanes = {
            let app = app.lock().await;
            app.provider_run_operation_lanes()
        };
        let router = Arc::new(CommandRouter::with_interactive_capacity_and_provider_lanes(
            Arc::clone(&app),
            INTERACTIVE_COMMAND_QUEUE_LIMIT,
            provider_runtime_lanes,
        ));
        let event_runtime = Arc::new(RelayEventRuntime::for_tests(1));
        let event_stream_id = subscription_event_stream_id(&created_session_id, &attachment_id);
        let first = event_runtime
            .event_log
            .append(
                event_stream_id.clone(),
                KernelEvent::Heartbeat {
                    session_id: created_session_id.clone(),
                },
            )
            .await
            .expect("first event should append");
        let second = event_runtime
            .event_log
            .append(
                event_stream_id,
                KernelEvent::Heartbeat {
                    session_id: created_session_id.clone(),
                },
            )
            .await
            .expect("second event should append");

        let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel();
        let subscription_private_key = relay_crypto::generate_private_key_base64();
        let subscription_public_key =
            relay_crypto::public_key_from_private_key_base64(&subscription_private_key)
                .expect("subscription public key should derive");

        replay_recent_relay_events(
            &event_runtime,
            &router,
            &app,
            &outgoing_tx,
            "subscription-1",
            &subscription_public_key,
            &created_session_id,
            &attachment_id,
            Some(first.event_id),
        )
        .await
        .expect("stale replay should emit recovery events");

        let gap =
            decrypt_relay_event_from_channel(&mut outgoing_rx, &subscription_private_key).await;
        assert_eq!(gap.0, second.event_id + 1);
        assert_eq!(gap.1["event"], serde_json::json!("replay_gap"));
        assert_eq!(
            gap.1["requested_from_event_id"],
            serde_json::json!(first.event_id)
        );
        assert_eq!(
            gap.1["first_retained_event_id"],
            serde_json::json!(second.event_id)
        );
        assert_eq!(gap.1["latest_event_id"], serde_json::json!(second.event_id));

        let snapshot =
            decrypt_relay_event_from_channel(&mut outgoing_rx, &subscription_private_key).await;
        assert_eq!(snapshot.0, second.event_id + 2);
        assert_eq!(snapshot.1["event"], serde_json::json!("session_snapshot"));
        assert_eq!(
            snapshot.1["session"]["id"],
            serde_json::json!(created_session_id)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn interactive_session_requests_are_handled_through_relay() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config.relay_token = Some("secret".to_string());
        config.relay_heartbeat_ms = 50;
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
        ));
        let (created_session_id, default_agent_id) = {
            let mut app = app.lock().await;
            create_test_session_with_alias(
                &mut app,
                "workspace-relay-test",
                "worktree-relay-test",
                "main",
            )
        };
        let attachment_id = {
            let mut app = app.lock().await;
            attach_test_client(
                &mut app,
                &created_session_id,
                "relay-client",
                ClientCapabilityLevel::MessageTransport,
            )
        };
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector_task = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app),
            Arc::clone(&state),
            shutdown_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut client_socket, _) = connect_async(&url)
            .await
            .expect("client should connect to relay");
        send_client_envelope(
            &mut client_socket,
            &RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some(config.daemon_id.clone()),
                    daemon_alias: None,
                },
            },
        )
        .await;
        let daemon_public_key = expect_client_connected(&mut client_socket).await;

        let resolve_private_key = send_client_request(
            &mut client_socket,
            "resolve-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
                session_ref: "main".to_string(),
                workspace_id: Some("workspace-relay-test".to_string()),
            }),
        )
        .await;
        let resolve_response =
            expect_client_response(&mut client_socket, "resolve-1", &resolve_private_key).await;
        assert!(matches!(
            resolve_response,
            LocalDaemonResponse::SessionResolved { session } if session.id() == created_session_id
        ));

        let focus_private_key = send_client_request(
            &mut client_socket,
            "focus-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: created_session_id.clone(),
                agent_id: default_agent_id.clone(),
            }),
        )
        .await;
        let focus_response =
            expect_client_response(&mut client_socket, "focus-1", &focus_private_key).await;
        assert!(matches!(
            focus_response,
            LocalDaemonResponse::AgentFocused { agent } if agent.id() == default_agent_id
        ));

        let config_private_key = send_client_request(
            &mut client_socket,
            "config-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
                session_id: created_session_id.clone(),
                attachment_id: attachment_id.clone(),
                values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                requires_idle: false,
            }),
        )
        .await;
        let config_response =
            expect_client_response(&mut client_socket, "config-1", &config_private_key).await;
        assert!(matches!(
            config_response,
            LocalDaemonResponse::SessionConfigUpdated { config, .. }
                if config.values().get("theme").map(String::as_str) == Some("compact")
        ));

        let detach_private_key = send_client_request(
            &mut client_socket,
            "detach-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::DetachFromSession(DetachFromSessionRequest {
                attachment_id: attachment_id.clone(),
            }),
        )
        .await;
        let detach_response =
            expect_client_response(&mut client_socket, "detach-1", &detach_private_key).await;
        assert!(matches!(
            detach_response,
            LocalDaemonResponse::SessionDetached { attachment } if attachment.id() == attachment_id
        ));

        let _ = shutdown_tx.send(true);
        connector_task.await.expect("connector task should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn terminal_resize_errors_are_returned_through_relay() {
        let _relay_test_guard = relay_client_test_guard().await;
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        let registry = server.registry();
        let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
        let server_task = {
            let server = Arc::clone(&server);
            tokio::spawn(async move {
                server
                    .run_until(async {
                        let _ = server_shutdown_rx.await;
                    })
                    .await
                    .expect("relay server should run");
            })
        };

        let mut config = DaemonConfig::for_tests();
        config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
        config.relay_token = Some("secret".to_string());
        config.relay_heartbeat_ms = 50;
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
        ));
        let created_session_id = {
            let mut app = app.lock().await;
            create_test_session(&mut app, "workspace-relay-test", "worktree-relay-test")
        };
        let state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connector_task = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app),
            Arc::clone(&state),
            shutdown_rx,
        ));

        wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut client_socket, _) = connect_async(&url)
            .await
            .expect("client should connect to relay");
        send_client_envelope(
            &mut client_socket,
            &RelayEnvelope::ClientConnect {
                auth_token: "secret".to_string(),
                target: ClientTarget {
                    daemon_id: Some(config.daemon_id.clone()),
                    daemon_alias: None,
                },
            },
        )
        .await;
        let daemon_public_key = expect_client_connected(&mut client_socket).await;

        let resize_private_key = send_client_request(
            &mut client_socket,
            "resize-1",
            &config.daemon_id,
            &daemon_public_key,
            LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
                session_id: created_session_id,
                cols: 120,
                rows: 40,
            }),
        )
        .await;
        let resize_error =
            expect_client_error(&mut client_socket, "resize-1", &resize_private_key).await;
        assert_eq!(resize_error.code, "no_active_provider_run");

        let _ = shutdown_tx.send(true);
        connector_task.await.expect("connector task should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    async fn wait_for_daemon_registration(
        registry: Arc<RwLock<arroba_relay::server::RelayRegistry>>,
        daemon_id: &str,
    ) {
        for _ in 0..200 {
            if registry.read().await.daemon(daemon_id).is_some() {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
        panic!("daemon `{daemon_id}` did not register with relay");
    }

    async fn wait_for_active_interaction(
        app: Arc<Mutex<DaemonApp>>,
        session_id: &str,
        agent_id: &str,
    ) -> String {
        for _ in 0..80 {
            {
                let app = app.lock().await;
                if let Ok(session) = app.sessions().get_session(session_id) {
                    if let Some(interaction) = session.active_interaction_for_agent(agent_id) {
                        return interaction.id().to_string();
                    }
                }
            }
            sleep(Duration::from_millis(25)).await;
        }
        panic!("interaction for agent `{agent_id}` did not become active");
    }

    async fn send_client_envelope<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        envelope: &RelayEnvelope,
    ) where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        socket
            .send(Message::Text(
                serde_json::to_string(envelope)
                    .expect("relay envelope should serialize")
                    .into(),
            ))
            .await
            .expect("client envelope should send");
    }

    async fn send_client_request<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        request_id: &str,
        daemon_id: &str,
        daemon_public_key: &str,
        request: LocalDaemonRequest,
    ) -> String
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let client_private_key = relay_crypto::generate_private_key_base64();
        let plaintext = serde_json::to_vec(&request).expect("request should serialize");
        let encrypted_request = relay_crypto::encrypt_payload_for_peer(
            &client_private_key,
            daemon_public_key,
            &plaintext,
        )
        .expect("request should encrypt");
        send_client_envelope(
            socket,
            &RelayEnvelope::ClientRequest {
                request_id: request_id.to_string(),
                target: ClientTarget {
                    daemon_id: Some(daemon_id.to_string()),
                    daemon_alias: None,
                },
                encrypted_request,
            },
        )
        .await;
        client_private_key
    }

    async fn expect_client_connected<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
    ) -> String
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        match socket.next().await {
            Some(Ok(Message::Text(payload))) => {
                match serde_json::from_str::<RelayEnvelope>(&payload)
                    .expect("relay envelope should parse")
                {
                    RelayEnvelope::ClientConnected {
                        daemon_public_key, ..
                    } => daemon_public_key,
                    other => panic!("unexpected envelope: {other:?}"),
                }
            }
            other => panic!("unexpected relay message: {other:?}"),
        }
    }

    async fn expect_client_response<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        request_id: &str,
        client_private_key: &str,
    ) -> LocalDaemonResponse
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(payload))) => {
                    match serde_json::from_str::<RelayEnvelope>(&payload)
                        .expect("relay envelope should parse")
                    {
                        RelayEnvelope::ClientResponse {
                            request_id: response_request_id,
                            encrypted_response,
                            error,
                        } => {
                            assert_eq!(response_request_id, request_id);
                            assert!(error.is_none(), "unexpected relay error: {error:?}");
                            let encrypted_response =
                                encrypted_response.expect("response payload should exist");
                            let decrypted = relay_crypto::decrypt_payload_for_private_key(
                                client_private_key,
                                &encrypted_response,
                            )
                            .expect("response should decrypt");
                            return serde_json::from_slice(&decrypted.plaintext)
                                .expect("local response should deserialize");
                        }
                        RelayEnvelope::ClientEvent { .. } => {}
                        other => panic!("unexpected envelope: {other:?}"),
                    }
                }
                other => panic!("unexpected relay message: {other:?}"),
            }
        }
    }

    async fn expect_json_client_response<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        request_id: &str,
        client_private_key: &str,
    ) -> serde_json::Value
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(payload))) => {
                    match serde_json::from_str::<RelayEnvelope>(&payload)
                        .expect("relay envelope should parse")
                    {
                        RelayEnvelope::ClientResponse {
                            request_id: response_request_id,
                            encrypted_response,
                            error,
                        } => {
                            assert_eq!(response_request_id, request_id);
                            assert!(error.is_none(), "unexpected relay error: {error:?}");
                            let encrypted_response =
                                encrypted_response.expect("response payload should exist");
                            let decrypted = relay_crypto::decrypt_payload_for_private_key(
                                client_private_key,
                                &encrypted_response,
                            )
                            .expect("response should decrypt");
                            return serde_json::from_slice(&decrypted.plaintext)
                                .expect("json response should deserialize");
                        }
                        RelayEnvelope::ClientEvent { .. } => {}
                        other => panic!("unexpected envelope: {other:?}"),
                    }
                }
                other => panic!("unexpected relay message: {other:?}"),
            }
        }
    }

    async fn expect_client_event<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        client_private_key: &str,
    ) -> serde_json::Value
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(payload))) => {
                    match serde_json::from_str::<RelayEnvelope>(&payload)
                        .expect("relay envelope should parse")
                    {
                        RelayEnvelope::ClientEvent {
                            encrypted_event, ..
                        } => {
                            let decrypted = relay_crypto::decrypt_payload_for_private_key(
                                client_private_key,
                                &encrypted_event,
                            )
                            .expect("event should decrypt");
                            return serde_json::from_slice(&decrypted.plaintext)
                                .expect("event should deserialize");
                        }
                        RelayEnvelope::ClientResponse { .. } => continue,
                        other => panic!("unexpected envelope: {other:?}"),
                    }
                }
                other => panic!("unexpected relay message: {other:?}"),
            }
        }
    }

    async fn expect_client_event_envelope<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        client_private_key: &str,
    ) -> (u64, serde_json::Value)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(payload))) => {
                    match serde_json::from_str::<RelayEnvelope>(&payload)
                        .expect("relay envelope should parse")
                    {
                        RelayEnvelope::ClientEvent {
                            event_id,
                            encrypted_event,
                            ..
                        } => {
                            let decrypted = relay_crypto::decrypt_payload_for_private_key(
                                client_private_key,
                                &encrypted_event,
                            )
                            .expect("event should decrypt");
                            return (
                                event_id,
                                serde_json::from_slice(&decrypted.plaintext)
                                    .expect("event should deserialize"),
                            );
                        }
                        RelayEnvelope::ClientResponse { .. } => continue,
                        other => panic!("unexpected envelope: {other:?}"),
                    }
                }
                other => panic!("unexpected relay message: {other:?}"),
            }
        }
    }

    async fn expect_named_client_event<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        client_private_key: &str,
        expected_event: &str,
    ) -> (u64, serde_json::Value)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        loop {
            let envelope = expect_client_event_envelope(socket, client_private_key).await;
            if envelope.1["event"] == serde_json::json!(expected_event) {
                return envelope;
            }
        }
    }

    async fn decrypt_relay_event_from_channel(
        outgoing_rx: &mut mpsc::UnboundedReceiver<RelayEnvelope>,
        client_private_key: &str,
    ) -> (u64, serde_json::Value) {
        match outgoing_rx
            .recv()
            .await
            .expect("relay event should be emitted")
        {
            RelayEnvelope::DaemonEvent {
                event_id,
                encrypted_event,
                ..
            } => {
                let decrypted = relay_crypto::decrypt_payload_for_private_key(
                    client_private_key,
                    &encrypted_event,
                )
                .expect("event should decrypt");
                (
                    event_id,
                    serde_json::from_slice(&decrypted.plaintext).expect("event should deserialize"),
                )
            }
            other => panic!("unexpected relay envelope: {other:?}"),
        }
    }

    async fn expect_client_error<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        request_id: &str,
        _client_private_key: &str,
    ) -> RelayError
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        match socket.next().await {
            Some(Ok(Message::Text(payload))) => {
                match serde_json::from_str::<RelayEnvelope>(&payload)
                    .expect("relay envelope should parse")
                {
                    RelayEnvelope::ClientResponse {
                        request_id: response_request_id,
                        encrypted_response,
                        error,
                    } => {
                        assert_eq!(response_request_id, request_id);
                        assert!(encrypted_response.is_none());
                        error.expect("relay error should exist")
                    }
                    other => panic!("unexpected envelope: {other:?}"),
                }
            }
            other => panic!("unexpected relay message: {other:?}"),
        }
    }
}
