use std::future::Future;
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{
    accept_async,
    tungstenite::protocol::{frame::coding::CloseCode, CloseFrame, Message},
};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::runtime::command::{KernelCommand, KernelCommandSource};
use crate::runtime::event_log::EventLog;
use crate::runtime::projection::TransportHealthStore;
use crate::runtime::router::CommandRouter;
use crate::transport::kernel_protocol::{
    kernel_subscription_scope, map_kernel_error, serialize_frame, KernelEvent, KernelIncomingFrame,
    KernelOutgoingFrame, KernelSubscriptionScope, KernelTransportError,
    WAITING_ROOM_INVENTORY_SENTINEL_ID, WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
};

mod command_cache;
mod outgoing;
mod subscriptions;

pub(crate) use command_cache::COMMAND_RESULT_CACHE_LIMIT;
use command_cache::{CommandFingerprint, CommandReservation, CommandResultCache};
use outgoing::try_send_outgoing_frame;
use subscriptions::{
    emit_replay_gap_snapshot, replay_recent_events, run_subscription_loop, ReplaySubscriptionResult,
};
pub(crate) use subscriptions::{watch_subscription_state, WatchResult};

pub(crate) const WATCH_INTERVAL_MS: u64 = 50;
const STATE_INTERVAL_TICKS: u64 = 4;
const HEARTBEAT_INTERVAL_TICKS: u64 = 20;
const RELAY_DISCOVERY_INTERVAL_TICKS: u64 = 100;
const WAITING_ROOM_INVENTORY_INTERVAL_TICKS: u64 = 50;
const DURABLE_SNAPSHOT_POLL_INTERVAL_MS: u64 = 5_000;
const WEBSOCKET_PING_INTERVAL_MS: u64 = 5_000;
pub(crate) const RECENT_EVENT_LIMIT: usize = 256;
const BACKPRESSURE_CLOSE_REASON: &str = "kernel transport overloaded; reconnecting";
pub(crate) const INBOUND_REQUEST_LIMIT: usize = 8;

#[derive(Debug, Clone)]
struct KernelSubscription {
    session_id: String,
    attachment_id: String,
    subscription_scope: KernelSubscriptionScope,
}

#[derive(Debug)]
struct KernelTransportRuntime {
    event_log: EventLog<KernelEvent>,
    command_result_cache: CommandResultCache,
    transport_health: TransportHealthStore,
}

impl Default for KernelTransportRuntime {
    fn default() -> Self {
        Self::new(TransportHealthStore::default())
    }
}

impl KernelTransportRuntime {
    fn new(transport_health: TransportHealthStore) -> Self {
        Self {
            event_log: EventLog::new(RECENT_EVENT_LIMIT),
            command_result_cache: CommandResultCache::default(),
            transport_health,
        }
    }

    fn new_with_persistent_event_ids(
        transport_health: TransportHealthStore,
        event_counter_path: impl Into<std::path::PathBuf>,
    ) -> Result<Self, DaemonError> {
        Ok(Self {
            event_log: EventLog::new_with_persistent_event_ids(
                RECENT_EVENT_LIMIT,
                event_counter_path,
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "reserve kernel event ids",
                message: error.to_string(),
            })?,
            command_result_cache: CommandResultCache::default(),
            transport_health,
        })
    }
}

#[derive(Debug)]
struct ConnectionState {
    subscription: Option<KernelSubscription>,
    watch_task: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
struct ConnectionCloseCommand {
    reason: String,
}

struct TransportConnectionGuard {
    transport_health: TransportHealthStore,
}

impl Drop for TransportConnectionGuard {
    fn drop(&mut self) {
        self.transport_health.record_connection_closed();
    }
}

pub async fn run_kernel_websocket_server<F>(
    app: Arc<Mutex<DaemonApp>>,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    let (bind_host, bind_port, provider_runtime_lanes) = {
        let app = app.lock().await;
        (
            app.config().kernel_websocket_host.clone(),
            app.config().kernel_websocket_port,
            app.provider_run_operation_lanes(),
        )
    };
    let listener = TcpListener::bind((bind_host.as_str(), bind_port))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "bind kernel websocket",
            message: error.to_string(),
        })?;
    run_kernel_websocket_server_with_bound_listener(app, listener, provider_runtime_lanes, shutdown)
        .await
}

pub async fn run_kernel_websocket_server_on_listener<F>(
    app: Arc<Mutex<DaemonApp>>,
    listener: StdTcpListener,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    listener
        .set_nonblocking(true)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "configure kernel websocket listener",
            message: error.to_string(),
        })?;
    let listener =
        TcpListener::from_std(listener).map_err(|error| DaemonError::LocalTransport {
            operation: "adopt kernel websocket listener",
            message: error.to_string(),
        })?;
    let provider_runtime_lanes = {
        let app = app.lock().await;
        app.provider_run_operation_lanes()
    };
    run_kernel_websocket_server_with_bound_listener(app, listener, provider_runtime_lanes, shutdown)
        .await
}

async fn run_kernel_websocket_server_with_bound_listener<F>(
    app: Arc<Mutex<DaemonApp>>,
    listener: TcpListener,
    provider_runtime_lanes: crate::provider::ProviderRunOperationLanes,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    let pump_app = Arc::clone(&app);
    let (transport_health, durable_snapshot_scheduler, event_counter_path) = {
        let app = app.lock().await;
        (
            app.transport_health_store(),
            app.durable_snapshot_scheduler(),
            app.config().kernel_event_counter_path(),
        )
    };
    let runtime = Arc::new(KernelTransportRuntime::new_with_persistent_event_ids(
        transport_health.clone(),
        event_counter_path,
    )?);
    let router = Arc::new(
        CommandRouter::with_interactive_capacity_provider_lanes_and_transport_health(
            Arc::clone(&app),
            crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
            provider_runtime_lanes,
            transport_health,
        ),
    );

    tokio::pin!(shutdown);

    let pump_task = tokio::spawn(async move {
        loop {
            {
                let mut app = pump_app.lock().await;
                crate::app::provider_output::pump_active_prompt_outputs(&mut app);
                crate::app::workflow_runtime::pump_workflow_watchdogs(&mut app);
            }
            sleep(Duration::from_millis(WATCH_INTERVAL_MS)).await;
        }
    });
    let mut durable_snapshot_task = durable_snapshot_scheduler.map(|scheduler| {
        tokio::spawn(scheduler.run(Duration::from_millis(DURABLE_SNAPSHOT_POLL_INTERVAL_MS)))
    });

    let mcp_router = Arc::clone(&router);
    let mcp_task = tokio::spawn(async move {
        let _ = crate::transport::mcp_server::run_mcp_http_server(mcp_router).await;
    });

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                pump_task.abort();
                if let Some(task) = durable_snapshot_task.take() {
                    task.abort();
                }
                mcp_task.abort();
                let mut app = app.lock().await;
                let _ = app.shutdown_cleanup();
                return Ok(());
            },
            accept_result = listener.accept() => {
                let (stream, _) = accept_result.map_err(|error| DaemonError::LocalTransport {
                    operation: "accept kernel websocket",
                    message: error.to_string(),
                })?;
                let app = Arc::clone(&app);
                let runtime = Arc::clone(&runtime);
                let router = Arc::clone(&router);
                tokio::spawn(async move {
                    let _ = handle_kernel_connection(app, runtime, router, stream).await;
                });
            }
        }
    }
}

async fn handle_kernel_connection(
    app: Arc<Mutex<DaemonApp>>,
    runtime: Arc<KernelTransportRuntime>,
    router: Arc<CommandRouter>,
    stream: tokio::net::TcpStream,
) -> Result<(), DaemonError> {
    let socket = accept_async(stream)
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "accept kernel websocket handshake",
            message: error.to_string(),
        })?;
    runtime.transport_health.record_connection_opened();
    let _connection_guard = TransportConnectionGuard {
        transport_health: runtime.transport_health.clone(),
    };
    let (queue_capacity, write_delay_ms) = {
        let app = app.lock().await;
        (
            app.config().kernel_websocket_queue_capacity,
            app.config().kernel_websocket_write_delay_ms,
        )
    };

    let (mut writer, mut reader) = socket.split();
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<KernelOutgoingFrame>(queue_capacity);
    let (close_tx, mut close_rx) = mpsc::unbounded_channel::<ConnectionCloseCommand>();
    let close_requested = Arc::new(AtomicBool::new(false));
    let inbound_request_permits = Arc::new(Semaphore::new(INBOUND_REQUEST_LIMIT));
    let connection_state = Arc::new(Mutex::new(ConnectionState {
        subscription: None,
        watch_task: None,
    }));

    let writer_task = tokio::spawn(async move {
        let mut transport_ping =
            tokio::time::interval(Duration::from_millis(WEBSOCKET_PING_INTERVAL_MS));
        transport_ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = transport_ping.tick() => {
                    if writer.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
                Some(command) = close_rx.recv() => {
                    let _ = writer.send(Message::Close(Some(CloseFrame {
                        code: CloseCode::Policy,
                        reason: command.reason.into(),
                    }))).await;
                    break;
                }
                Some(frame) = outgoing_rx.recv() => {
                    let payload = match serialize_frame(&frame) {
                        Ok(payload) => payload,
                        Err(_) => break,
                    };
                    if write_delay_ms > 0 {
                        sleep(Duration::from_millis(write_delay_ms)).await;
                    }
                    if writer.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
                else => break,
            }
        }
    });

    let mut read_error = None;
    while let Some(message_result) = reader.next().await {
        let message = match message_result {
            Ok(message) => message,
            Err(error) => {
                read_error = Some(DaemonError::LocalTransport {
                    operation: "read kernel websocket frame",
                    message: error.to_string(),
                });
                break;
            }
        };

        match message {
            Message::Text(payload) => {
                handle_incoming_payload(
                    &app,
                    &runtime,
                    &router,
                    &connection_state,
                    &inbound_request_permits,
                    &outgoing_tx,
                    &close_tx,
                    &close_requested,
                    payload.as_bytes(),
                )
                .await;
            }
            Message::Binary(payload) => {
                handle_incoming_payload(
                    &app,
                    &runtime,
                    &router,
                    &connection_state,
                    &inbound_request_permits,
                    &outgoing_tx,
                    &close_tx,
                    &close_requested,
                    &payload,
                )
                .await;
            }
            Message::Ping(_) => {}
            Message::Close(_) => break,
            Message::Pong(_) | Message::Frame(_) => {}
        }
    }

    {
        let mut state = connection_state.lock().await;
        if let Some(task) = state.watch_task.take() {
            task.abort();
        }
        if state.subscription.take().is_some() {
            runtime.transport_health.record_subscription_closed();
        }
    }
    writer_task.abort();

    if let Some(error) = read_error {
        return Err(error);
    }

    Ok(())
}

async fn handle_incoming_payload(
    app: &Arc<Mutex<DaemonApp>>,
    runtime: &Arc<KernelTransportRuntime>,
    router: &Arc<CommandRouter>,
    connection_state: &Arc<Mutex<ConnectionState>>,
    inbound_request_permits: &Arc<Semaphore>,
    outgoing_tx: &mpsc::Sender<KernelOutgoingFrame>,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    payload: &[u8],
) {
    let frame = match serde_json::from_slice::<KernelIncomingFrame>(payload) {
        Ok(frame) => frame,
        Err(error) => {
            let _ = try_send_outgoing_frame(
                outgoing_tx,
                close_tx,
                close_requested,
                &runtime.transport_health,
                KernelOutgoingFrame::Response {
                    request_id: "unknown".to_string(),
                    response: Box::new(None),
                    error: Some(KernelTransportError {
                        code: "invalid_frame".to_string(),
                        message: format!("invalid kernel transport payload: {error}"),
                        retryable: false,
                    }),
                },
                None,
                None,
            );
            return;
        }
    };

    match frame {
        KernelIncomingFrame::Request {
            request_id,
            command_id,
            causation_id,
            correlation_id,
            request,
        } => {
            runtime.transport_health.record_incoming_request();
            let caller = router
                .local_command_caller(KernelCommandSource::LocalCli)
                .await;
            let command = KernelCommand::from_local_request_with_caller(
                command_id.unwrap_or_else(|| request_id.clone()),
                KernelCommandSource::LocalCli,
                caller,
                correlation_id.clone(),
                causation_id.clone(),
                &request,
            );
            let fingerprint = CommandFingerprint::from_command_and_request(&command, &request);
            match runtime
                .command_result_cache
                .reserve(&command.command_id, &fingerprint)
                .await
            {
                CommandReservation::Wait(wait_rx) => {
                    let outgoing_tx = outgoing_tx.clone();
                    let close_tx = close_tx.clone();
                    let close_requested = Arc::clone(close_requested);
                    let transport_health = runtime.transport_health.clone();
                    let session_id = command.session_id.clone();
                    let attachment_id = command.attachment_id.clone();
                    tokio::spawn(async move {
                        let Ok(cached) = wait_rx.await else {
                            let _ = try_send_outgoing_frame(
                                &outgoing_tx,
                                &close_tx,
                                &close_requested,
                                &transport_health,
                                KernelOutgoingFrame::Response {
                                    request_id,
                                    response: Box::new(None),
                                    error: Some(KernelTransportError {
                                        code: "duplicate_command_unavailable".to_string(),
                                        message:
                                            "original duplicate command result was unavailable"
                                                .to_string(),
                                        retryable: true,
                                    }),
                                },
                                session_id.as_deref(),
                                attachment_id.as_deref(),
                            );
                            return;
                        };
                        let _ = try_send_outgoing_frame(
                            &outgoing_tx,
                            &close_tx,
                            &close_requested,
                            &transport_health,
                            KernelOutgoingFrame::Response {
                                request_id,
                                response: cached.response,
                                error: cached.error,
                            },
                            session_id.as_deref(),
                            attachment_id.as_deref(),
                        );
                    });
                    return;
                }
                CommandReservation::Conflict => {
                    runtime.transport_health.record_duplicate_command_conflict();
                    let _ = try_send_outgoing_frame(
                        outgoing_tx,
                        close_tx,
                        close_requested,
                        &runtime.transport_health,
                        KernelOutgoingFrame::Response {
                            request_id,
                            response: Box::new(None),
                            error: Some(KernelTransportError {
                                code: "duplicate_command_conflict".to_string(),
                                message: format!(
                                    "command_id `{}` was already used for a different request",
                                    command.command_id
                                ),
                                retryable: false,
                            }),
                        },
                        command.session_id.as_deref(),
                        command.attachment_id.as_deref(),
                    );
                    return;
                }
                CommandReservation::Dispatch => {}
            };
            let permit = match Arc::clone(inbound_request_permits).try_acquire_owned() {
                Ok(permit) => permit,
                Err(error) => {
                    runtime
                        .command_result_cache
                        .forget_pending(&command.command_id)
                        .await;
                    runtime.transport_health.record_inbound_overload_rejection();
                    let _ = try_send_outgoing_frame(
                        outgoing_tx,
                        close_tx,
                        close_requested,
                        &runtime.transport_health,
                        KernelOutgoingFrame::Response {
                            request_id,
                            response: Box::new(None),
                            error: Some(KernelTransportError {
                                code: "kernel_request_overloaded".to_string(),
                                message: format!(
                                    "kernel request admission queue overloaded: {error}"
                                ),
                                retryable: true,
                            }),
                        },
                        command.session_id.as_deref(),
                        command.attachment_id.as_deref(),
                    );
                    return;
                }
            };
            crate::logging::info_with_fields(
                "daemon.runtime_transport",
                "kernel command accepted",
                serde_json::json!({
                    "request_id": request_id,
                    "command_id": command.command_id,
                    "command_type": command.command_type,
                    "correlation_id": command.correlation_id,
                    "priority": format!("{:?}", command.priority),
                    "session_id": command.session_id,
                    "attachment_id": command.attachment_id,
                    "agent_id": command.agent_id,
                }),
            );
            let runtime = Arc::clone(runtime);
            let router = Arc::clone(router);
            let outgoing_tx = outgoing_tx.clone();
            let close_tx = close_tx.clone();
            let close_requested = Arc::clone(close_requested);
            tokio::spawn(async move {
                let _permit = permit;
                let command_id = command.command_id.clone();
                let session_id = command.session_id.clone();
                let attachment_id = command.attachment_id.clone();
                let response = router.dispatch(command, request).await;
                let outgoing = match response {
                    Ok(response) => KernelOutgoingFrame::Response {
                        request_id,
                        response: Box::new(Some(
                            serde_json::to_value(response).unwrap_or(Value::Null),
                        )),
                        error: None,
                    },
                    Err(error) => KernelOutgoingFrame::Response {
                        request_id,
                        response: Box::new(None),
                        error: Some(map_kernel_error(&error)),
                    },
                };
                runtime
                    .command_result_cache
                    .complete(command_id, fingerprint, &outgoing)
                    .await;
                let _ = try_send_outgoing_frame(
                    &outgoing_tx,
                    &close_tx,
                    &close_requested,
                    &runtime.transport_health,
                    outgoing,
                    session_id.as_deref(),
                    attachment_id.as_deref(),
                );
            });
        }
        KernelIncomingFrame::Subscribe {
            request_id,
            session_id,
            attachment_id,
            subscription_scope,
            resume_from_event_id,
        } => {
            let scope = kernel_subscription_scope(subscription_scope.as_deref());
            crate::logging::info_with_fields(
                "daemon.runtime_transport",
                "kernel websocket subscribed",
                serde_json::json!({
                    "session_id": session_id,
                    "attachment_id": attachment_id,
                    "subscription_scope": subscription_scope,
                    "resume_from_event_id": resume_from_event_id,
                }),
            );
            if scope != KernelSubscriptionScope::WaitingRoomInventory
                && (session_id == WAITING_ROOM_INVENTORY_SENTINEL_ID
                    || attachment_id == WAITING_ROOM_INVENTORY_SENTINEL_ID)
            {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "waiting-room inventory sentinel arrived without subscription scope",
                    serde_json::json!({
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                        "subscription_scope": subscription_scope,
                        "diagnosis": "client likely dropped subscription_scope=waiting_room_inventory",
                    }),
                );
            }
            let replay_gap = if scope == KernelSubscriptionScope::WaitingRoomInventory {
                None
            } else {
                let replay_result = replay_recent_events(
                    runtime,
                    outgoing_tx,
                    close_tx,
                    close_requested,
                    &session_id,
                    &attachment_id,
                    resume_from_event_id,
                )
                .await;
                match replay_result {
                    ReplaySubscriptionResult::Gap(gap) => {
                        emit_replay_gap_snapshot(
                            app,
                            runtime,
                            outgoing_tx,
                            close_tx,
                            close_requested,
                            &session_id,
                            &attachment_id,
                        )
                        .await;
                        Some(gap)
                    }
                    ReplaySubscriptionResult::Overflow => return,
                    ReplaySubscriptionResult::Complete | ReplaySubscriptionResult::NoCursor => None,
                }
            };
            {
                let mut state = connection_state.lock().await;
                if let Some(task) = state.watch_task.take() {
                    task.abort();
                }
                if state.subscription.is_none() {
                    runtime.transport_health.record_subscription_opened();
                }
                state.subscription = Some(KernelSubscription {
                    session_id: session_id.clone(),
                    attachment_id: attachment_id.clone(),
                    subscription_scope: scope.clone(),
                });
                state.watch_task = Some(tokio::spawn(run_subscription_loop(
                    Arc::clone(app),
                    Arc::clone(router),
                    Arc::clone(runtime),
                    outgoing_tx.clone(),
                    close_tx.clone(),
                    Arc::clone(close_requested),
                    KernelSubscription {
                        session_id: session_id.clone(),
                        attachment_id: attachment_id.clone(),
                        subscription_scope: scope,
                    },
                )));
            }
            let _ = try_send_outgoing_frame(
                outgoing_tx,
                close_tx,
                close_requested,
                &runtime.transport_health,
                KernelOutgoingFrame::Response {
                    request_id,
                    response: Box::new(Some(serde_json::json!({
                        "ok": true,
                        "resumed_from_event_id": resume_from_event_id,
                        "replay_gap": replay_gap.as_ref().map(|gap| serde_json::json!({
                            "requested_from_event_id": gap.requested_from_event_id,
                            "first_retained_event_id": gap.first_retained_event_id,
                            "latest_event_id": gap.latest_event_id,
                        })),
                    }))),
                    error: None,
                },
                if subscription_scope.as_deref() == Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE)
                {
                    None
                } else {
                    Some(&session_id)
                },
                if subscription_scope.as_deref() == Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE)
                {
                    None
                } else {
                    Some(&attachment_id)
                },
            );
        }
        KernelIncomingFrame::Unsubscribe { request_id } => {
            crate::logging::info_with_fields(
                "daemon.runtime_transport",
                "kernel websocket unsubscribed",
                serde_json::json!({}),
            );
            {
                let mut state = connection_state.lock().await;
                if state.subscription.take().is_some() {
                    runtime.transport_health.record_subscription_closed();
                }
                if let Some(task) = state.watch_task.take() {
                    task.abort();
                }
            }
            let _ = try_send_outgoing_frame(
                outgoing_tx,
                close_tx,
                close_requested,
                &runtime.transport_health,
                KernelOutgoingFrame::Response {
                    request_id,
                    response: Box::new(Some(serde_json::json!({ "ok": true }))),
                    error: None,
                },
                None,
                None,
            );
        }
    }
}
