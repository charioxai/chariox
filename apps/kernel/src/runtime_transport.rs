use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{
    accept_async,
    tungstenite::protocol::{frame::coding::CloseCode, CloseFrame, Message},
};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, RelayStatus, RemoteMachineRecord};
use crate::provider::RuntimeProviderRun;
use crate::runtime::command::{KernelCommand, KernelCommandSource};
use crate::runtime::event_log::{EventLog, ReplayGap, ReplayOutcome};
use crate::runtime::projection::{
    AgentRuntimeActivity, SessionSnapshotProjection, TransportHealthStore,
};
use crate::runtime::router::CommandRouter;
use crate::session::RuntimeSession;
use crate::terminal::{
    AssistantMessageCompletionRecord, RuntimeNoticeRecord, TerminalOutputRecord,
};

pub(crate) const WATCH_INTERVAL_MS: u64 = 50;
const STATE_INTERVAL_TICKS: u64 = 4;
const HEARTBEAT_INTERVAL_TICKS: u64 = 20;
const RELAY_DISCOVERY_INTERVAL_TICKS: u64 = 100;
const WAITING_ROOM_INVENTORY_INTERVAL_TICKS: u64 = 50;
const WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE: &str = "waiting_room_inventory";
const WAITING_ROOM_INVENTORY_SENTINEL_ID: &str = "__waiting_room_inventory__";
const DURABLE_SNAPSHOT_POLL_INTERVAL_MS: u64 = 5_000;
const WEBSOCKET_PING_INTERVAL_MS: u64 = 5_000;
pub(crate) const RECENT_EVENT_LIMIT: usize = 256;
pub(crate) const COMMAND_RESULT_CACHE_LIMIT: usize = 512;
const BACKPRESSURE_CLOSE_REASON: &str = "kernel transport overloaded; reconnecting";
pub(crate) const INBOUND_REQUEST_LIMIT: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KernelIncomingFrame {
    Request {
        request_id: String,
        #[serde(default)]
        command_id: Option<String>,
        #[serde(default)]
        causation_id: Option<String>,
        #[serde(default)]
        correlation_id: Option<String>,
        request: LocalDaemonRequest,
    },
    Subscribe {
        request_id: String,
        session_id: String,
        attachment_id: String,
        #[serde(default)]
        subscription_scope: Option<String>,
        #[serde(default)]
        resume_from_event_id: Option<u64>,
    },
    Unsubscribe {
        request_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KernelTransportError {
    code: String,
    message: String,
    retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KernelOutgoingFrame {
    Response {
        request_id: String,
        response: Box<Option<Value>>,
        error: Option<KernelTransportError>,
    },
    Event {
        event_id: u64,
        event: Box<KernelEvent>,
    },
}

#[derive(Debug, Clone)]
struct CachedCommandResult {
    fingerprint: CommandFingerprint,
    response: Box<Option<Value>>,
    error: Option<KernelTransportError>,
}

#[derive(Debug)]
enum CommandResultEntry {
    Pending {
        fingerprint: CommandFingerprint,
        waiters: Vec<oneshot::Sender<CachedCommandResult>>,
    },
    Completed(CachedCommandResult),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandFingerprint {
    command_type: String,
    source: String,
    session_id: Option<String>,
    attachment_id: Option<String>,
    request_hash: u64,
}

impl CommandFingerprint {
    fn from_command_and_request(command: &KernelCommand, request: &LocalDaemonRequest) -> Self {
        let mut hasher = DefaultHasher::new();
        serde_json::to_vec(request)
            .unwrap_or_default()
            .hash(&mut hasher);
        Self {
            command_type: command.command_type.clone(),
            source: serde_json::to_string(&command.source)
                .unwrap_or_else(|_| "unknown".to_string()),
            session_id: command.session_id.clone(),
            attachment_id: command.attachment_id.clone(),
            request_hash: hasher.finish(),
        }
    }
}

enum CommandReservation {
    Dispatch,
    Wait(oneshot::Receiver<CachedCommandResult>),
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum KernelEvent {
    TerminalOutput {
        records: Vec<TerminalOutputRecord>,
    },
    RuntimeNotices {
        notices: Vec<RuntimeNoticeRecord>,
    },
    AssistantMessageCompleted {
        session_id: String,
        provider_run_id: String,
        agent_id: Option<String>,
        recipient_attachment_ids: Vec<String>,
        message_id: String,
        completed_at_ms: u64,
    },
    SessionSnapshot {
        session: Box<RuntimeSession>,
        provider_run: Box<Option<RuntimeProviderRun>>,
        agent_activity: Box<std::collections::BTreeMap<String, AgentRuntimeActivity>>,
    },
    SessionUnavailable {
        session_id: String,
        message: String,
    },
    RelayStatusChanged {
        status: RelayStatus,
    },
    RemoteMachinesChanged {
        machines: Vec<RemoteMachineRecord>,
    },
    WaitingRoomInventoryChanged {
        inventory_version: String,
    },
    Heartbeat {
        session_id: String,
    },
    TransportResumed {
        session_id: String,
        resumed_from_event_id: Option<u64>,
    },
    ReplayGap {
        session_id: String,
        requested_from_event_id: u64,
        first_retained_event_id: Option<u64>,
        latest_event_id: Option<u64>,
        message: String,
    },
}

#[derive(Debug, Clone)]
struct KernelSubscription {
    session_id: String,
    attachment_id: String,
    subscription_scope: KernelSubscriptionScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KernelSubscriptionScope {
    Session,
    WaitingRoomInventory,
}

#[derive(Debug)]
struct KernelTransportRuntime {
    event_log: EventLog<KernelEvent>,
    command_results: Mutex<BTreeMap<String, CommandResultEntry>>,
    command_result_order: Mutex<VecDeque<String>>,
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
            command_results: Mutex::new(BTreeMap::new()),
            command_result_order: Mutex::new(VecDeque::new()),
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
            command_results: Mutex::new(BTreeMap::new()),
            command_result_order: Mutex::new(VecDeque::new()),
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
            match reserve_command_result(runtime, &command.command_id, &fingerprint).await {
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
                    forget_pending_command_result(&runtime, &command.command_id).await;
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
                complete_command_result(&runtime, command_id, fingerprint, &outgoing).await;
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

async fn run_subscription_loop(
    app: Arc<Mutex<DaemonApp>>,
    router: Arc<CommandRouter>,
    runtime: Arc<KernelTransportRuntime>,
    outgoing_tx: mpsc::Sender<KernelOutgoingFrame>,
    close_tx: mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: Arc<AtomicBool>,
    subscription: KernelSubscription,
) {
    if subscription.subscription_scope == KernelSubscriptionScope::WaitingRoomInventory {
        run_waiting_room_inventory_subscription_loop(
            router,
            runtime,
            outgoing_tx,
            close_tx,
            close_requested,
        )
        .await;
        return;
    }
    let mut previous_snapshot: Option<SessionSnapshotProjection> = None;
    let mut previous_relay_status: Option<RelayStatus> = None;
    let mut previous_remote_machines: Option<Vec<RemoteMachineRecord>> = None;
    let mut previous_inventory_version: Option<String> = None;
    let mut tick: u64 = 0;
    let event_stream_id =
        subscription_event_stream_id(&subscription.session_id, &subscription.attachment_id);

    loop {
        let watch_result = {
            let mut app = app.lock().await;
            watch_subscription_state(
                &mut app,
                &subscription.session_id,
                &subscription.attachment_id,
                tick,
                previous_snapshot.clone(),
            )
        };

        match watch_result {
            WatchResult::Ok {
                records,
                notices,
                completions,
                snapshot,
            } => {
                if !records.is_empty()
                    && !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::TerminalOutput { records },
                        Some(&event_stream_id),
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                {
                    break;
                }
                if !notices.is_empty()
                    && !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::RuntimeNotices { notices },
                        Some(&event_stream_id),
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                {
                    break;
                }
                for completion in completions {
                    if !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::AssistantMessageCompleted {
                            session_id: completion.session_id,
                            provider_run_id: completion.provider_run_id,
                            agent_id: completion.agent_id,
                            recipient_attachment_ids: completion.recipient_attachment_ids,
                            message_id: completion.message_id,
                            completed_at_ms: completion.completed_at_ms,
                        },
                        Some(&event_stream_id),
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                    {
                        break;
                    }
                }
                if let Some(snapshot) = *snapshot {
                    previous_snapshot = Some(snapshot.clone());
                    if !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::SessionSnapshot {
                            session: Box::new(snapshot.session),
                            provider_run: Box::new(snapshot.provider_run),
                            agent_activity: Box::new(snapshot.agent_activity),
                        },
                        Some(&event_stream_id),
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                    {
                        break;
                    }
                }
                if tick.is_multiple_of(HEARTBEAT_INTERVAL_TICKS)
                    && !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::Heartbeat {
                            session_id: subscription.session_id.clone(),
                        },
                        Some(&event_stream_id),
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                {
                    break;
                }
                if tick.is_multiple_of(HEARTBEAT_INTERVAL_TICKS) {
                    match relay_status_snapshot_for_events(&app).await {
                        Ok(status) => {
                            if previous_relay_status.as_ref() != Some(&status) {
                                previous_relay_status = Some(status.clone());
                                if !emit_kernel_event(
                                    &runtime,
                                    &outgoing_tx,
                                    &close_tx,
                                    &close_requested,
                                    KernelEvent::RelayStatusChanged { status },
                                    Some(&event_stream_id),
                                    Some(&subscription.session_id),
                                    Some(&subscription.attachment_id),
                                )
                                .await
                                {
                                    break;
                                }
                            }
                        }
                        Err(error) => {
                            crate::logging::warn_with_fields(
                                "daemon.runtime_transport",
                                "kernel event loop failed to build relay status snapshot",
                                serde_json::json!({
                                    "session_id": subscription.session_id,
                                    "attachment_id": subscription.attachment_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                    }
                }
                if tick.is_multiple_of(RELAY_DISCOVERY_INTERVAL_TICKS) {
                    match remote_machines_snapshot_for_events(&app).await {
                        Ok(machines) => {
                            if previous_remote_machines.as_ref() != Some(&machines) {
                                previous_remote_machines = Some(machines.clone());
                                if !emit_kernel_event(
                                    &runtime,
                                    &outgoing_tx,
                                    &close_tx,
                                    &close_requested,
                                    KernelEvent::RemoteMachinesChanged { machines },
                                    Some(&event_stream_id),
                                    Some(&subscription.session_id),
                                    Some(&subscription.attachment_id),
                                )
                                .await
                                {
                                    break;
                                }
                            }
                        }
                        Err(error) => {
                            crate::logging::warn_with_fields(
                                "daemon.runtime_transport",
                                "kernel event loop failed to build remote machines snapshot",
                                serde_json::json!({
                                    "session_id": subscription.session_id,
                                    "attachment_id": subscription.attachment_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                    }
                }
                if tick.is_multiple_of(WAITING_ROOM_INVENTORY_INTERVAL_TICKS) {
                    match router.waiting_room_inventory_version().await {
                        Ok(inventory_version) => {
                            if previous_inventory_version.as_ref() != Some(&inventory_version) {
                                previous_inventory_version = Some(inventory_version.clone());
                                if !emit_kernel_event(
                                    &runtime,
                                    &outgoing_tx,
                                    &close_tx,
                                    &close_requested,
                                    KernelEvent::WaitingRoomInventoryChanged { inventory_version },
                                    Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE),
                                    Some(&subscription.session_id),
                                    Some(&subscription.attachment_id),
                                )
                                .await
                                {
                                    break;
                                }
                            }
                        }
                        Err(error) => {
                            crate::logging::warn_with_fields(
                                "daemon.runtime_transport",
                                "kernel event loop failed to build waiting-room inventory version",
                                serde_json::json!({
                                    "session_id": subscription.session_id,
                                    "attachment_id": subscription.attachment_id,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                    }
                }
            }
            WatchResult::Unavailable(message) => {
                let _ = emit_kernel_event(
                    &runtime,
                    &outgoing_tx,
                    &close_tx,
                    &close_requested,
                    KernelEvent::SessionUnavailable {
                        session_id: subscription.session_id.clone(),
                        message,
                    },
                    Some(&event_stream_id),
                    Some(&subscription.session_id),
                    Some(&subscription.attachment_id),
                )
                .await;
                break;
            }
        }

        tick = tick.wrapping_add(1);
        sleep(Duration::from_millis(WATCH_INTERVAL_MS)).await;
    }
}

async fn relay_status_snapshot_for_events(
    app: &Arc<Mutex<DaemonApp>>,
) -> Result<RelayStatus, DaemonError> {
    let (config, relay_state) = {
        let app = app.lock().await;
        (app.config().clone(), app.relay_client_state())
    };
    let connected = relay_state.read().await.connected();
    Ok(RelayStatus {
        configured: config.relay_url.is_some() && config.relay_token.is_some(),
        connected,
        relay_url: config.relay_url,
        relay_token_configured: config.relay_token.is_some(),
        daemon_id: config.daemon_id,
        machine_id: config.host_machine_id,
        machine_alias: config.host_machine_alias,
    })
}

async fn remote_machines_snapshot_for_events(
    app: &Arc<Mutex<DaemonApp>>,
) -> Result<Vec<RemoteMachineRecord>, DaemonError> {
    let remote_relay_inventory = {
        let app = app.lock().await;
        app.remote_relay_inventory_projection_store()
    };
    let (machines, _) = remote_relay_inventory.snapshot();
    Ok(machines)
}

pub(crate) enum WatchResult {
    Ok {
        records: Vec<TerminalOutputRecord>,
        notices: Vec<RuntimeNoticeRecord>,
        completions: Vec<AssistantMessageCompletionRecord>,
        snapshot: Box<Option<SessionSnapshotProjection>>,
    },
    Unavailable(String),
}

pub(crate) fn watch_subscription_state(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    tick: u64,
    previous_snapshot: Option<SessionSnapshotProjection>,
) -> WatchResult {
    if crate::app::KernelSessionReadService::new(app)
        .ensure_attachment_in_session(session_id, attachment_id)
        .is_err()
    {
        return WatchResult::Unavailable("Current session is no longer available.".to_string());
    }

    let records = match crate::app::provider_output::pump_terminal_output_for_attachment(
        app,
        session_id,
        attachment_id,
    ) {
        Ok(records) => records,
        Err(DaemonError::NoActiveProviderRun { .. }) => Vec::new(),
        Err(DaemonError::SessionNotFound { .. })
        | Err(DaemonError::AttachmentNotFound { .. })
        | Err(DaemonError::AttachmentNotInSession { .. }) => {
            return WatchResult::Unavailable("Current session is no longer available.".to_string());
        }
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.runtime_transport",
                "kernel event loop failed to pump terminal output",
                serde_json::json!({
                    "session_id": session_id,
                    "attachment_id": attachment_id,
                    "error": error.to_string(),
                }),
            );
            Vec::new()
        }
    };

    let notices = app
        .terminal_mut()
        .drain_notice_records(session_id, attachment_id);
    let completions = app
        .terminal_mut()
        .drain_completion_records(session_id, attachment_id);
    let snapshot = if tick.is_multiple_of(STATE_INTERVAL_TICKS) {
        match build_session_snapshot(app, session_id) {
            Ok(snapshot) => {
                if previous_snapshot.as_ref() != Some(&snapshot) {
                    Box::new(Some(snapshot))
                } else {
                    Box::new(None)
                }
            }
            Err(DaemonError::SessionNotFound { .. }) => {
                return WatchResult::Unavailable(
                    "Current session is no longer available.".to_string(),
                );
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "kernel event loop failed to build session snapshot",
                    serde_json::json!({
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                        "error": error.to_string(),
                    }),
                );
                Box::new(None)
            }
        }
    } else {
        Box::new(None)
    };

    WatchResult::Ok {
        records,
        notices,
        completions,
        snapshot,
    }
}

async fn emit_kernel_event(
    runtime: &Arc<KernelTransportRuntime>,
    outgoing_tx: &mpsc::Sender<KernelOutgoingFrame>,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    event: KernelEvent,
    event_stream_id: Option<&str>,
    session_id: Option<&str>,
    attachment_id: Option<&str>,
) -> bool {
    let stream_id = event_stream_id
        .map(str::to_string)
        .or_else(|| event_stream_id_for_event(&event, session_id));
    let event_id = if let Some(stream_id) = stream_id.as_deref() {
        match runtime
            .event_log
            .append(stream_id.to_string(), event.clone())
            .await
        {
            Ok(logged) => logged.event_id,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "failed to reserve kernel event id",
                    serde_json::json!({
                        "stream_id": stream_id,
                        "error": error.to_string(),
                    }),
                );
                return false;
            }
        }
    } else {
        match runtime.event_log.append("daemon", event.clone()).await {
            Ok(logged) => logged.event_id,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "failed to reserve kernel event id",
                    serde_json::json!({
                        "stream_id": "daemon",
                        "error": error.to_string(),
                    }),
                );
                return false;
            }
        }
    };
    runtime.transport_health.record_emitted_event();
    try_send_outgoing_frame(
        outgoing_tx,
        close_tx,
        close_requested,
        &runtime.transport_health,
        KernelOutgoingFrame::Event {
            event_id,
            event: Box::new(event),
        },
        session_id,
        attachment_id,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplaySubscriptionResult {
    NoCursor,
    Complete,
    Gap(ReplayGap),
    Overflow,
}

async fn replay_recent_events(
    runtime: &Arc<KernelTransportRuntime>,
    outgoing_tx: &mpsc::Sender<KernelOutgoingFrame>,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    session_id: &str,
    attachment_id: &str,
    resume_from_event_id: Option<u64>,
) -> ReplaySubscriptionResult {
    let Some(cursor) = resume_from_event_id else {
        return ReplaySubscriptionResult::NoCursor;
    };
    let stream_id = subscription_event_stream_id(session_id, attachment_id);
    let replay = runtime.event_log.replay_after(&stream_id, cursor).await;

    let events = match replay {
        ReplayOutcome::Replayed(events) => events,
        ReplayOutcome::Gap(gap) => {
            runtime.transport_health.record_replay_gap();
            let _ = emit_kernel_event(
                runtime,
                outgoing_tx,
                close_tx,
                close_requested,
                KernelEvent::ReplayGap {
                    session_id: session_id.to_string(),
                    requested_from_event_id: gap.requested_from_event_id,
                    first_retained_event_id: gap.first_retained_event_id,
                    latest_event_id: gap.latest_event_id,
                    message: "Replay cursor is outside the retained kernel event window; refresh the session projection.".to_string(),
                },
                Some(&stream_id),
                Some(session_id),
                Some(attachment_id),
            )
            .await;
            return ReplaySubscriptionResult::Gap(gap);
        }
    };

    for persisted in events {
        if !event_is_relevant_to_attachment(&persisted.event, attachment_id) {
            continue;
        }
        if !try_send_outgoing_frame(
            outgoing_tx,
            close_tx,
            close_requested,
            &runtime.transport_health,
            KernelOutgoingFrame::Event {
                event_id: persisted.event_id,
                event: Box::new(persisted.event.clone()),
            },
            Some(session_id),
            Some(attachment_id),
        ) {
            return ReplaySubscriptionResult::Overflow;
        }
    }

    if !try_send_outgoing_frame(
        outgoing_tx,
        close_tx,
        close_requested,
        &runtime.transport_health,
        KernelOutgoingFrame::Event {
            event_id: match runtime
                .event_log
                .append(
                    stream_id,
                    KernelEvent::TransportResumed {
                        session_id: session_id.to_string(),
                        resumed_from_event_id: Some(cursor),
                    },
                )
                .await
            {
                Ok(logged) => logged.event_id,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.runtime_transport",
                        "failed to reserve transport-resumed event id",
                        serde_json::json!({
                            "session_id": session_id,
                            "attachment_id": attachment_id,
                            "error": error.to_string(),
                        }),
                    );
                    return ReplaySubscriptionResult::Overflow;
                }
            },
            event: Box::new(KernelEvent::TransportResumed {
                session_id: session_id.to_string(),
                resumed_from_event_id: Some(cursor),
            }),
        },
        Some(session_id),
        Some(attachment_id),
    ) {
        return ReplaySubscriptionResult::Overflow;
    }
    ReplaySubscriptionResult::Complete
}

async fn emit_replay_gap_snapshot(
    app: &Arc<Mutex<DaemonApp>>,
    runtime: &Arc<KernelTransportRuntime>,
    outgoing_tx: &mpsc::Sender<KernelOutgoingFrame>,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    session_id: &str,
    attachment_id: &str,
) {
    let event_stream_id = subscription_event_stream_id(session_id, attachment_id);
    let snapshot = {
        let mut app = app.lock().await;
        build_session_snapshot(&mut app, session_id)
    };
    match snapshot {
        Ok(projection) => {
            let _ = emit_kernel_event(
                runtime,
                outgoing_tx,
                close_tx,
                close_requested,
                KernelEvent::SessionSnapshot {
                    session: Box::new(projection.session),
                    provider_run: Box::new(projection.provider_run),
                    agent_activity: Box::new(projection.agent_activity),
                },
                Some(&event_stream_id),
                Some(session_id),
                Some(attachment_id),
            )
            .await;
        }
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.runtime_transport",
                "kernel replay gap snapshot failed",
                serde_json::json!({
                    "session_id": session_id,
                    "attachment_id": attachment_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

async fn reserve_command_result(
    runtime: &Arc<KernelTransportRuntime>,
    command_id: &str,
    fingerprint: &CommandFingerprint,
) -> CommandReservation {
    let mut results = runtime.command_results.lock().await;
    match results.get_mut(command_id) {
        Some(CommandResultEntry::Completed(cached)) => {
            if cached.fingerprint == *fingerprint {
                let (tx, rx) = oneshot::channel();
                let _ = tx.send(cached.clone());
                CommandReservation::Wait(rx)
            } else {
                CommandReservation::Conflict
            }
        }
        Some(CommandResultEntry::Pending {
            fingerprint: existing,
            waiters,
        }) => {
            if existing == fingerprint {
                let (tx, rx) = oneshot::channel();
                waiters.push(tx);
                CommandReservation::Wait(rx)
            } else {
                CommandReservation::Conflict
            }
        }
        None => {
            results.insert(
                command_id.to_string(),
                CommandResultEntry::Pending {
                    fingerprint: fingerprint.clone(),
                    waiters: Vec::new(),
                },
            );
            CommandReservation::Dispatch
        }
    }
}

async fn complete_command_result(
    runtime: &Arc<KernelTransportRuntime>,
    command_id: String,
    fingerprint: CommandFingerprint,
    frame: &KernelOutgoingFrame,
) {
    let KernelOutgoingFrame::Response {
        response, error, ..
    } = frame
    else {
        return;
    };
    let cached = CachedCommandResult {
        fingerprint,
        response: response.clone(),
        error: error.clone(),
    };
    let waiters = {
        let mut results = runtime.command_results.lock().await;
        match results.insert(
            command_id.clone(),
            CommandResultEntry::Completed(cached.clone()),
        ) {
            Some(CommandResultEntry::Pending { waiters, .. }) => waiters,
            _ => Vec::new(),
        }
    };
    for waiter in waiters {
        let _ = waiter.send(cached.clone());
    }
    let mut order = runtime.command_result_order.lock().await;
    order.push_back(command_id);
    while order.len() > COMMAND_RESULT_CACHE_LIMIT {
        if let Some(expired) = order.pop_front() {
            runtime.command_results.lock().await.remove(&expired);
        }
    }
}

async fn forget_pending_command_result(runtime: &Arc<KernelTransportRuntime>, command_id: &str) {
    let mut results = runtime.command_results.lock().await;
    if matches!(
        results.get(command_id),
        Some(CommandResultEntry::Pending { .. })
    ) {
        results.remove(command_id);
    }
}

fn try_send_outgoing_frame(
    outgoing_tx: &mpsc::Sender<KernelOutgoingFrame>,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    transport_health: &TransportHealthStore,
    frame: KernelOutgoingFrame,
    session_id: Option<&str>,
    attachment_id: Option<&str>,
) -> bool {
    match outgoing_tx.try_send(frame) {
        Ok(()) => true,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            transport_health.record_outgoing_queue_overflow();
            if !close_requested.swap(true, Ordering::SeqCst) {
                transport_health.record_slow_consumer_close();
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "kernel websocket connection overflowed; closing slow consumer",
                    serde_json::json!({
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                    }),
                );
                let _ = close_tx.send(ConnectionCloseCommand {
                    reason: BACKPRESSURE_CLOSE_REASON.to_string(),
                });
            }
            false
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
    }
}

pub(crate) fn event_session_id(event: &KernelEvent) -> Option<&str> {
    match event {
        KernelEvent::TerminalOutput { records } => {
            records.first().map(|record| record.session_id.as_str())
        }
        KernelEvent::RuntimeNotices { notices } => {
            notices.first().map(|notice| notice.session_id.as_str())
        }
        KernelEvent::AssistantMessageCompleted { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::SessionSnapshot { session, .. } => Some(session.id()),
        KernelEvent::SessionUnavailable { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::RelayStatusChanged { .. } => None,
        KernelEvent::RemoteMachinesChanged { .. } => None,
        KernelEvent::WaitingRoomInventoryChanged { .. } => None,
        KernelEvent::Heartbeat { session_id } => Some(session_id.as_str()),
        KernelEvent::TransportResumed { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::ReplayGap { session_id, .. } => Some(session_id.as_str()),
    }
}

fn event_stream_id_for_event(
    event: &KernelEvent,
    fallback_session_id: Option<&str>,
) -> Option<String> {
    event_session_id(event)
        .or(fallback_session_id)
        .map(session_stream_id)
        .or_else(|| Some("daemon".to_string()))
}

fn session_stream_id(session_id: &str) -> String {
    format!("session:{session_id}")
}

fn subscription_event_stream_id(session_id: &str, attachment_id: &str) -> String {
    format!("session:{session_id}:attachment:{attachment_id}")
}

pub(crate) fn event_is_relevant_to_attachment(event: &KernelEvent, attachment_id: &str) -> bool {
    match event {
        KernelEvent::TerminalOutput { records } => records.iter().any(|record| {
            record
                .recipient_attachment_ids
                .iter()
                .any(|id| id == attachment_id)
        }),
        KernelEvent::RuntimeNotices { notices } => notices.iter().any(|notice| {
            notice.recipient_attachment_ids.is_empty()
                || notice
                    .recipient_attachment_ids
                    .iter()
                    .any(|id| id == attachment_id)
        }),
        KernelEvent::AssistantMessageCompleted {
            recipient_attachment_ids,
            ..
        } => recipient_attachment_ids
            .iter()
            .any(|id| id == attachment_id),
        KernelEvent::SessionSnapshot { .. }
        | KernelEvent::SessionUnavailable { .. }
        | KernelEvent::RelayStatusChanged { .. }
        | KernelEvent::RemoteMachinesChanged { .. }
        | KernelEvent::WaitingRoomInventoryChanged { .. }
        | KernelEvent::Heartbeat { .. }
        | KernelEvent::TransportResumed { .. }
        | KernelEvent::ReplayGap { .. } => true,
    }
}

async fn run_waiting_room_inventory_subscription_loop(
    router: Arc<CommandRouter>,
    runtime: Arc<KernelTransportRuntime>,
    outgoing_tx: mpsc::Sender<KernelOutgoingFrame>,
    close_tx: mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: Arc<AtomicBool>,
) {
    let mut previous_inventory_version: Option<String> = None;
    loop {
        match router.waiting_room_inventory_version().await {
            Ok(inventory_version) => {
                if previous_inventory_version.as_ref() != Some(&inventory_version) {
                    previous_inventory_version = Some(inventory_version.clone());
                    if !emit_kernel_event(
                        &runtime,
                        &outgoing_tx,
                        &close_tx,
                        &close_requested,
                        KernelEvent::WaitingRoomInventoryChanged { inventory_version },
                        Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE),
                        None,
                        None,
                    )
                    .await
                    {
                        break;
                    }
                }
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "kernel waiting-room inventory subscription failed to build version",
                    serde_json::json!({ "error": error.to_string() }),
                );
            }
        }
        sleep(Duration::from_millis(
            WATCH_INTERVAL_MS * WAITING_ROOM_INVENTORY_INTERVAL_TICKS,
        ))
        .await;
    }
}

fn kernel_subscription_scope(scope: Option<&str>) -> KernelSubscriptionScope {
    if scope == Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE) {
        KernelSubscriptionScope::WaitingRoomInventory
    } else {
        KernelSubscriptionScope::Session
    }
}

fn map_kernel_error(error: &DaemonError) -> KernelTransportError {
    match error {
        DaemonError::SessionNotFound { .. } => kernel_error("session_not_found", error, false),
        DaemonError::AttachmentNotFound { .. } => {
            kernel_error("attachment_not_found", error, false)
        }
        DaemonError::AttachmentNotInSession { .. } => {
            kernel_error("attachment_not_in_session", error, false)
        }
        DaemonError::NoActiveProviderRun { .. } => {
            kernel_error("no_active_provider_run", error, false)
        }
        DaemonError::ProviderRunNotFound { .. } => {
            kernel_error("provider_run_not_found", error, false)
        }
        DaemonError::WorkspaceClaimConflict { .. } => {
            kernel_error("workspace_claim_conflict", error, true)
        }
        DaemonError::ProviderAdapterNotFound { .. } => {
            kernel_error("provider_adapter_not_found", error, false)
        }
        DaemonError::ProviderProtocol { .. } => {
            kernel_error("provider_protocol_error", error, true)
        }
        DaemonError::LocalTransport { .. } => kernel_error("local_transport_error", error, true),
        DaemonError::PtySpawn { .. } => kernel_error("pty_spawn_failed", error, true),
        DaemonError::PtyCleanup { .. } => kernel_error("pty_cleanup_failed", error, true),
        DaemonError::PtyWrite { .. } => kernel_error("pty_write_failed", error, true),
        DaemonError::PtyResize { .. } => kernel_error("pty_resize_failed", error, true),
        _ => kernel_error("kernel_request_failed", error, false),
    }
}

fn kernel_error(code: &str, error: &DaemonError, retryable: bool) -> KernelTransportError {
    KernelTransportError {
        code: code.to_string(),
        message: error.to_string(),
        retryable,
    }
}

fn build_session_snapshot(
    app: &mut DaemonApp,
    session_id: &str,
) -> Result<SessionSnapshotProjection, DaemonError> {
    SessionSnapshotProjection::from_daemon_app(app, session_id, 0)
}

fn serialize_frame(frame: &KernelOutgoingFrame) -> Result<String, DaemonError> {
    serde_json::to_string(frame).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize kernel websocket frame",
        message: error.to_string(),
    })
}
