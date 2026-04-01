use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{accept_async, tungstenite::protocol::Message};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::LocalDaemonRequest;
use crate::provider::RuntimeProviderRun;
use crate::session::RuntimeSession;
use crate::terminal::{RuntimeNoticeRecord, TerminalOutputRecord};

const WATCH_INTERVAL_MS: u64 = 50;
const STATE_INTERVAL_TICKS: u64 = 4;

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
    },
    Unsubscribe {
        request_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KernelOutgoingFrame {
    Response {
        request_id: String,
        response: Box<Option<Value>>,
        error: Option<String>,
    },
    Event {
        event_id: u64,
        event: Box<KernelEvent>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum KernelEvent {
    TerminalOutput {
        records: Vec<TerminalOutputRecord>,
    },
    RuntimeNotices {
        notices: Vec<RuntimeNoticeRecord>,
    },
    SessionSnapshot {
        session: Box<RuntimeSession>,
        provider_run: Box<Option<RuntimeProviderRun>>,
    },
    SessionUnavailable {
        message: String,
    },
}

#[derive(Debug, Clone)]
struct KernelSubscription {
    session_id: String,
    attachment_id: String,
}

#[derive(Debug)]
struct ConnectionState {
    subscription: Option<KernelSubscription>,
    watch_task: Option<JoinHandle<()>>,
}

pub async fn run_kernel_websocket_server<F>(app: DaemonApp, shutdown: F) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    let bind_host = app.config().kernel_websocket_host.clone();
    let bind_port = app.config().kernel_websocket_port;
    let listener = TcpListener::bind((bind_host.as_str(), bind_port))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "bind kernel websocket",
            message: error.to_string(),
        })?;
    let app = Arc::new(Mutex::new(app));

    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => return Ok(()),
            accept_result = listener.accept() => {
                let (stream, _) = accept_result.map_err(|error| DaemonError::LocalTransport {
                    operation: "accept kernel websocket",
                    message: error.to_string(),
                })?;
                let app = Arc::clone(&app);
                tokio::spawn(async move {
                    let _ = handle_kernel_connection(app, stream).await;
                });
            }
        }
    }
}

async fn handle_kernel_connection(
    app: Arc<Mutex<DaemonApp>>,
    stream: tokio::net::TcpStream,
) -> Result<(), DaemonError> {
    let socket = accept_async(stream)
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "accept kernel websocket handshake",
            message: error.to_string(),
        })?;

    let (mut writer, mut reader) = socket.split();
    let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<KernelOutgoingFrame>();
    let event_counter = Arc::new(AtomicU64::new(0));
    let connection_state = Arc::new(Mutex::new(ConnectionState {
        subscription: None,
        watch_task: None,
    }));

    let writer_task = tokio::spawn(async move {
        while let Some(frame) = outgoing_rx.recv().await {
            let payload = match serialize_frame(&frame) {
                Ok(payload) => payload,
                Err(_) => break,
            };
            if writer.send(Message::Text(payload.into())).await.is_err() {
                break;
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
                    &connection_state,
                    &outgoing_tx,
                    &event_counter,
                    payload.as_bytes(),
                )
                .await;
            }
            Message::Binary(payload) => {
                handle_incoming_payload(
                    &app,
                    &connection_state,
                    &outgoing_tx,
                    &event_counter,
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
    connection_state: &Arc<Mutex<ConnectionState>>,
    outgoing_tx: &mpsc::UnboundedSender<KernelOutgoingFrame>,
    event_counter: &Arc<AtomicU64>,
    payload: &[u8],
) {
    let frame = match serde_json::from_slice::<KernelIncomingFrame>(payload) {
        Ok(frame) => frame,
        Err(error) => {
            let _ = outgoing_tx.send(KernelOutgoingFrame::Response {
                request_id: "unknown".to_string(),
                response: Box::new(None),
                error: Some(format!("invalid kernel transport payload: {error}")),
            });
            return;
        }
    };

    match frame {
        KernelIncomingFrame::Request { request_id, request } => {
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
                    error: Some(error.to_string()),
                },
            };
            let _ = outgoing_tx.send(outgoing);
        }
        KernelIncomingFrame::Subscribe {
            request_id,
            session_id,
            attachment_id,
        } => {
            crate::logging::info_with_fields(
                "daemon.kernel_transport",
                "kernel websocket subscribed",
                serde_json::json!({
                    "session_id": session_id,
                    "attachment_id": attachment_id,
                }),
            );
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
                    outgoing_tx.clone(),
                    Arc::clone(event_counter),
                    KernelSubscription {
                        session_id,
                        attachment_id,
                    },
                )));
            }
            let _ = outgoing_tx.send(KernelOutgoingFrame::Response {
                request_id,
                response: Box::new(Some(serde_json::json!({ "ok": true }))),
                error: None,
            });
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
            let _ = outgoing_tx.send(KernelOutgoingFrame::Response {
                request_id,
                response: Box::new(Some(serde_json::json!({ "ok": true }))),
                error: None,
            });
        }
    }
}

async fn run_subscription_loop(
    app: Arc<Mutex<DaemonApp>>,
    outgoing_tx: mpsc::UnboundedSender<KernelOutgoingFrame>,
    event_counter: Arc<AtomicU64>,
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
                snapshot,
            } => {
                if !records.is_empty() {
                    let _ = outgoing_tx.send(KernelOutgoingFrame::Event {
                        event_id: next_event_id(&event_counter),
                        event: Box::new(KernelEvent::TerminalOutput { records }),
                    });
                }
                if !notices.is_empty() {
                    let _ = outgoing_tx.send(KernelOutgoingFrame::Event {
                        event_id: next_event_id(&event_counter),
                        event: Box::new(KernelEvent::RuntimeNotices { notices }),
                    });
                }
                if let Some(snapshot) = *snapshot {
                    previous_snapshot = Some(snapshot.clone());
                    let _ = outgoing_tx.send(KernelOutgoingFrame::Event {
                        event_id: next_event_id(&event_counter),
                        event: Box::new(KernelEvent::SessionSnapshot {
                            session: Box::new(snapshot.0),
                            provider_run: Box::new(snapshot.1),
                        }),
                    });
                }
            }
            WatchResult::Unavailable(message) => {
                let _ = outgoing_tx.send(KernelOutgoingFrame::Event {
                    event_id: next_event_id(&event_counter),
                    event: Box::new(KernelEvent::SessionUnavailable { message }),
                });
                break;
            }
        }

        tick = tick.wrapping_add(1);
        sleep(Duration::from_millis(WATCH_INTERVAL_MS)).await;
    }
}

enum WatchResult {
    Ok {
        records: Vec<TerminalOutputRecord>,
        notices: Vec<RuntimeNoticeRecord>,
        snapshot: Box<Option<(RuntimeSession, Option<RuntimeProviderRun>)>>,
    },
    Unavailable(String),
}

fn watch_subscription_state(
    app: &mut DaemonApp,
    session_id: &str,
    attachment_id: &str,
    tick: u64,
    previous_snapshot: Option<(RuntimeSession, Option<RuntimeProviderRun>)>,
) -> WatchResult {
    if app.ensure_attachment_in_session(session_id, attachment_id).is_err() {
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

    let notices = app.terminal_mut().drain_notice_records(session_id, attachment_id);
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
                return WatchResult::Unavailable("Current session is no longer available.".to_string());
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
        snapshot,
    }
}

fn build_session_snapshot(
    app: &mut DaemonApp,
    session_id: &str,
) -> Result<(RuntimeSession, Option<RuntimeProviderRun>), DaemonError> {
    let mut session = app.sessions().get_session(session_id)?;
    let agents = app.agents().get_session_agents(session_id);
    session.set_agents(agents);
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
