use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{
    accept_async,
    tungstenite::protocol::{frame::coding::CloseCode, CloseFrame, Message},
};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::provider::RuntimeProviderRun;
use crate::session::RuntimeSession;
use crate::terminal::{
    AssistantMessageCompletionRecord, RuntimeNoticeRecord, TerminalOutputRecord,
};

pub(crate) const WATCH_INTERVAL_MS: u64 = 50;
const STATE_INTERVAL_TICKS: u64 = 4;
const HEARTBEAT_INTERVAL_TICKS: u64 = 20;
pub(crate) const RECENT_EVENT_LIMIT: usize = 256;
const BACKPRESSURE_CLOSE_REASON: &str = "kernel transport overloaded; reconnecting";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KernelIncomingFrame {
    Request {
        request_id: String,
        request: LocalDaemonRequest,
    },
    Subscribe {
        request_id: String,
        session_id: String,
        attachment_id: String,
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
    },
    SessionUnavailable {
        session_id: String,
        message: String,
    },
    Heartbeat {
        session_id: String,
    },
    TransportResumed {
        session_id: String,
        resumed_from_event_id: Option<u64>,
    },
}

#[derive(Debug, Clone)]
struct KernelSubscription {
    session_id: String,
    attachment_id: String,
}

#[derive(Debug, Clone)]
struct PersistedKernelEvent {
    event_id: u64,
    event: KernelEvent,
}

#[derive(Debug, Default)]
struct KernelTransportRuntime {
    event_counter: AtomicU64,
    recent_events: Mutex<BTreeMap<String, VecDeque<PersistedKernelEvent>>>,
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

pub async fn run_kernel_websocket_server<F>(
    app: Arc<Mutex<DaemonApp>>,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    let (bind_host, bind_port) = {
        let app = app.lock().await;
        (
            app.config().kernel_websocket_host.clone(),
            app.config().kernel_websocket_port,
        )
    };
    let listener = TcpListener::bind((bind_host.as_str(), bind_port))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "bind kernel websocket",
            message: error.to_string(),
        })?;
    let pump_app = Arc::clone(&app);
    let runtime = Arc::new(KernelTransportRuntime::default());

    tokio::pin!(shutdown);

    tokio::spawn(async move {
        loop {
            {
                let mut app = pump_app.lock().await;
                crate::transport::TransportService::pump_active_prompts(&mut app);
            }
            sleep(Duration::from_millis(WATCH_INTERVAL_MS)).await;
        }
    });

    let mcp_app = Arc::clone(&app);
    tokio::spawn(async move {
        let _ = crate::transport::mcp_server::run_mcp_http_server(mcp_app).await;
    });

    loop {
        tokio::select! {
            _ = &mut shutdown => {
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
                tokio::spawn(async move {
                    let _ = handle_kernel_connection(app, runtime, stream).await;
                });
            }
        }
    }
}

async fn handle_kernel_connection(
    app: Arc<Mutex<DaemonApp>>,
    runtime: Arc<KernelTransportRuntime>,
    stream: tokio::net::TcpStream,
) -> Result<(), DaemonError> {
    let socket = accept_async(stream)
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "accept kernel websocket handshake",
            message: error.to_string(),
        })?;
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
    let connection_state = Arc::new(Mutex::new(ConnectionState {
        subscription: None,
        watch_task: None,
    }));

    let writer_task = tokio::spawn(async move {
        loop {
            tokio::select! {
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

    while let Some(message_result) = reader.next().await {
        let message = match message_result {
            Ok(message) => message,
            Err(error) => {
                return Err(DaemonError::LocalTransport {
                    operation: "read kernel websocket frame",
                    message: error.to_string(),
                });
            }
        };

        match message {
            Message::Text(payload) => {
                handle_incoming_payload(
                    &app,
                    &runtime,
                    &connection_state,
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
                    &connection_state,
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
    }
    writer_task.abort();

    Ok(())
}

async fn handle_incoming_payload(
    app: &Arc<Mutex<DaemonApp>>,
    runtime: &Arc<KernelTransportRuntime>,
    connection_state: &Arc<Mutex<ConnectionState>>,
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
            request,
        } => {
            let response = {
                let mut app = app.lock().await;
                app.handle_local_request(request)
            };
            let outgoing = match response {
                Ok(response) => KernelOutgoingFrame::Response {
                    request_id,
                    response: Box::new(Some(serde_json::to_value(response).unwrap_or(Value::Null))),
                    error: None,
                },
                Err(error) => KernelOutgoingFrame::Response {
                    request_id,
                    response: Box::new(None),
                    error: Some(map_kernel_error(&error)),
                },
            };
            let _ = try_send_outgoing_frame(
                outgoing_tx,
                close_tx,
                close_requested,
                outgoing,
                None,
                None,
            );
        }
        KernelIncomingFrame::Subscribe {
            request_id,
            session_id,
            attachment_id,
            resume_from_event_id,
        } => {
            crate::logging::info_with_fields(
                "daemon.kernel_transport",
                "kernel websocket subscribed",
                serde_json::json!({
                    "session_id": session_id,
                    "attachment_id": attachment_id,
                    "resume_from_event_id": resume_from_event_id,
                }),
            );
            replay_recent_events(
                runtime,
                outgoing_tx,
                close_tx,
                close_requested,
                &session_id,
                &attachment_id,
                resume_from_event_id,
            )
            .await;
            {
                let mut state = connection_state.lock().await;
                if let Some(task) = state.watch_task.take() {
                    task.abort();
                }
                state.subscription = Some(KernelSubscription {
                    session_id: session_id.clone(),
                    attachment_id: attachment_id.clone(),
                });
                state.watch_task = Some(tokio::spawn(run_subscription_loop(
                    Arc::clone(app),
                    Arc::clone(runtime),
                    outgoing_tx.clone(),
                    close_tx.clone(),
                    Arc::clone(close_requested),
                    KernelSubscription {
                        session_id: session_id.clone(),
                        attachment_id: attachment_id.clone(),
                    },
                )));
            }
            let _ = try_send_outgoing_frame(
                outgoing_tx,
                close_tx,
                close_requested,
                KernelOutgoingFrame::Response {
                    request_id,
                    response: Box::new(Some(serde_json::json!({
                        "ok": true,
                        "resumed_from_event_id": resume_from_event_id,
                    }))),
                    error: None,
                },
                Some(&session_id),
                Some(&attachment_id),
            );
        }
        KernelIncomingFrame::Unsubscribe { request_id } => {
            crate::logging::info_with_fields(
                "daemon.kernel_transport",
                "kernel websocket unsubscribed",
                serde_json::json!({}),
            );
            {
                let mut state = connection_state.lock().await;
                state.subscription = None;
                if let Some(task) = state.watch_task.take() {
                    task.abort();
                }
            }
            let _ = try_send_outgoing_frame(
                outgoing_tx,
                close_tx,
                close_requested,
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
    runtime: Arc<KernelTransportRuntime>,
    outgoing_tx: mpsc::Sender<KernelOutgoingFrame>,
    close_tx: mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: Arc<AtomicBool>,
    subscription: KernelSubscription,
) {
    let mut previous_snapshot: Option<(RuntimeSession, Option<RuntimeProviderRun>)> = None;
    let mut tick: u64 = 0;

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
                            session: Box::new(snapshot.0),
                            provider_run: Box::new(snapshot.1),
                        },
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
                        Some(&subscription.session_id),
                        Some(&subscription.attachment_id),
                    )
                    .await
                {
                    break;
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

pub(crate) enum WatchResult {
    Ok {
        records: Vec<TerminalOutputRecord>,
        notices: Vec<RuntimeNoticeRecord>,
        completions: Vec<AssistantMessageCompletionRecord>,
        snapshot: Box<Option<(RuntimeSession, Option<RuntimeProviderRun>)>>,
    },
    Unavailable(String),
}

pub(crate) fn watch_subscription_state(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    tick: u64,
    previous_snapshot: Option<(RuntimeSession, Option<RuntimeProviderRun>)>,
) -> WatchResult {
    if app
        .ensure_attachment_in_session(session_id, attachment_id)
        .is_err()
    {
        return WatchResult::Unavailable("Current session is no longer available.".to_string());
    }

    let records = match app.pump_terminal_output(session_id, attachment_id) {
        Ok(records) => records,
        Err(DaemonError::NoActiveProviderRun { .. }) => Vec::new(),
        Err(DaemonError::SessionNotFound { .. })
        | Err(DaemonError::AttachmentNotFound { .. })
        | Err(DaemonError::AttachmentNotInSession { .. }) => {
            return WatchResult::Unavailable("Current session is no longer available.".to_string());
        }
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.kernel_transport",
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
                    "daemon.kernel_transport",
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
    session_id: Option<&str>,
    attachment_id: Option<&str>,
) -> bool {
    let event_id = next_event_id(&runtime.event_counter);
    if let Some(session_id) = event_session_id(&event) {
        let mut recent_events = runtime.recent_events.lock().await;
        let entry = recent_events.entry(session_id.to_string()).or_default();
        entry.push_back(PersistedKernelEvent {
            event_id,
            event: event.clone(),
        });
        while entry.len() > RECENT_EVENT_LIMIT {
            entry.pop_front();
        }
    }
    try_send_outgoing_frame(
        outgoing_tx,
        close_tx,
        close_requested,
        KernelOutgoingFrame::Event {
            event_id,
            event: Box::new(event),
        },
        session_id,
        attachment_id,
    )
}

async fn replay_recent_events(
    runtime: &Arc<KernelTransportRuntime>,
    outgoing_tx: &mpsc::Sender<KernelOutgoingFrame>,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    session_id: &str,
    attachment_id: &str,
    resume_from_event_id: Option<u64>,
) {
    let Some(cursor) = resume_from_event_id else {
        return;
    };
    let recent_events = runtime.recent_events.lock().await;
    let Some(events) = recent_events.get(session_id) else {
        return;
    };

    for persisted in events {
        if persisted.event_id <= cursor {
            continue;
        }
        if !event_is_relevant_to_attachment(&persisted.event, attachment_id) {
            continue;
        }
        if !try_send_outgoing_frame(
            outgoing_tx,
            close_tx,
            close_requested,
            KernelOutgoingFrame::Event {
                event_id: persisted.event_id,
                event: Box::new(persisted.event.clone()),
            },
            Some(session_id),
            Some(attachment_id),
        ) {
            return;
        }
    }

    let _ = try_send_outgoing_frame(
        outgoing_tx,
        close_tx,
        close_requested,
        KernelOutgoingFrame::Event {
            event_id: next_event_id(&runtime.event_counter),
            event: Box::new(KernelEvent::TransportResumed {
                session_id: session_id.to_string(),
                resumed_from_event_id: Some(cursor),
            }),
        },
        Some(session_id),
        Some(attachment_id),
    );
}

fn try_send_outgoing_frame(
    outgoing_tx: &mpsc::Sender<KernelOutgoingFrame>,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    frame: KernelOutgoingFrame,
    session_id: Option<&str>,
    attachment_id: Option<&str>,
) -> bool {
    match outgoing_tx.try_send(frame) {
        Ok(()) => true,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            if !close_requested.swap(true, Ordering::SeqCst) {
                crate::logging::warn_with_fields(
                    "daemon.kernel_transport",
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
        KernelEvent::Heartbeat { session_id } => Some(session_id.as_str()),
        KernelEvent::TransportResumed { session_id, .. } => Some(session_id.as_str()),
    }
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
        | KernelEvent::Heartbeat { .. }
        | KernelEvent::TransportResumed { .. } => true,
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
) -> Result<(RuntimeSession, Option<RuntimeProviderRun>), DaemonError> {
    let mut session = app.sessions().get_session(session_id)?;
    let agents = app.agents().get_session_agents(session_id);
    session.set_agents(agents);
    app.project_session_runtime_view(&mut session);
    let provider_run = session
        .active_provider_run_id()
        .and_then(|provider_run_id| app.providers().get_run(provider_run_id).ok());
    Ok((session, provider_run))
}

fn serialize_frame(frame: &KernelOutgoingFrame) -> Result<String, DaemonError> {
    serde_json::to_string(frame).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize kernel websocket frame",
        message: error.to_string(),
    })
}

fn next_event_id(counter: &AtomicU64) -> u64 {
    counter.fetch_add(1, Ordering::Relaxed) + 1
}
