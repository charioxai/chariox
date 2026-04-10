use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use arroba_relay::protocol::{EncryptedRelayPayload, RelayEnvelope, RelayError};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel_transport::{
    watch_subscription_state, KernelEvent, WatchResult, WATCH_INTERVAL_MS,
};
use crate::local::LocalDaemonRequest;
use crate::transport::relay_crypto;

#[derive(Debug, Clone, Default)]
pub struct RelayClientState {
    connected: bool,
}

const RELAY_HEARTBEAT_INTERVAL_TICKS: u64 = 20;

type RelaySubscriptionTasks = Arc<Mutex<BTreeMap<String, JoinHandle<()>>>>;

pub async fn run_daemon_relay_connector(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let (relay_url, heartbeat) = {
        let app = app.lock().await;
        let config = app.config();
        let Some(relay_url) = config.relay_url.clone() else {
            return;
        };
        if config.relay_token.is_none() {
            return;
        }
        (relay_url, Duration::from_millis(config.relay_heartbeat_ms))
    };

    loop {
        if *shutdown.borrow() {
            set_connected(&state, false).await;
            return;
        }

        match connect_async(&relay_url).await {
            Ok((socket, _)) => {
                let (mut writer, mut reader) = socket.split();
                let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<RelayEnvelope>();
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
                });
                let subscription_tasks: RelaySubscriptionTasks =
                    Arc::new(Mutex::new(BTreeMap::new()));
                let event_counter = Arc::new(AtomicU64::new(0));
                let daemon_id = {
                    let app = app.lock().await;
                    app.config().daemon_id.clone()
                };
                let register = {
                    let app = app.lock().await;
                    RelayEnvelope::DaemonRegister {
                        registration: app.relay_registration(),
                    }
                };
                if outgoing_tx.send(register).is_err() {
                    writer_task.abort();
                    set_connected(&state, false).await;
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                set_connected(&state, true).await;

                loop {
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_ok() && *shutdown.borrow() {
                                let _ = outgoing_tx.send(RelayEnvelope::Close {
                                    reason: "daemon shutting down".to_string(),
                                });
                                sleep(Duration::from_millis(25)).await;
                                abort_subscription_tasks(&subscription_tasks).await;
                                writer_task.abort();
                                set_connected(&state, false).await;
                                return;
                            }
                        }
                        incoming = reader.next() => {
                            match incoming {
                                Some(Ok(Message::Text(payload))) => {
                                    if handle_incoming_envelope(
                                        &app,
                                        &outgoing_tx,
                                        &subscription_tasks,
                                        &event_counter,
                                        &payload,
                                    )
                                    .await
                                    .is_err()
                                    {
                                        abort_subscription_tasks(&subscription_tasks).await;
                                        writer_task.abort();
                                        set_connected(&state, false).await;
                                        break;
                                    }
                                }
                                Some(Ok(Message::Close(_))) => {
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    set_connected(&state, false).await;
                                    break;
                                }
                                Some(Ok(_)) => {}
                                Some(Err(_)) | None => {
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    set_connected(&state, false).await;
                                    break;
                                }
                            }
                        }
                        _ = sleep(heartbeat) => {
                            let heartbeat_frame = RelayEnvelope::DaemonHeartbeat {
                                daemon_id: daemon_id.clone(),
                            };
                            if outgoing_tx.send(heartbeat_frame).is_err() {
                                abort_subscription_tasks(&subscription_tasks).await;
                                writer_task.abort();
                                set_connected(&state, false).await;
                                break;
                            }
                        }
                    }
                }
            }
            Err(_) => {
                set_connected(&state, false).await;
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
    app: &Arc<Mutex<DaemonApp>>,
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    subscription_tasks: &RelaySubscriptionTasks,
    event_counter: &Arc<AtomicU64>,
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
            encrypted_request,
        } => {
            let relay_response = handle_daemon_request(app, encrypted_request).await;
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: relay_response.encrypted_response,
                    error: relay_response.error,
                },
            )?;
        }
        RelayEnvelope::DaemonSubscribe {
            relay_request_id,
            relay_subscription_id,
            session_id,
            attachment_id,
            client_public_key,
            resume_from_event_id,
        } => {
            {
                let app = app.lock().await;
                app.ensure_attachment_in_session(&session_id, &attachment_id)?;
            }
            if let Some(existing) = subscription_tasks
                .lock()
                .await
                .remove(&relay_subscription_id)
            {
                existing.abort();
            }
            let ack = encrypt_json_response(
                app,
                &client_public_key,
                serde_json::json!({
                    "ok": true,
                    "resumed_from_event_id": resume_from_event_id,
                }),
            )
            .await?;
            let task = tokio::spawn(run_relay_subscription_loop(
                Arc::clone(app),
                outgoing_tx.clone(),
                relay_subscription_id.clone(),
                client_public_key,
                session_id,
                attachment_id,
                Arc::clone(event_counter),
            ));
            subscription_tasks
                .lock()
                .await
                .insert(relay_subscription_id, task);
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: Some(ack),
                    error: None,
                },
            )?;
        }
        RelayEnvelope::DaemonUnsubscribe {
            relay_request_id,
            relay_subscription_id,
            client_public_key,
        } => {
            let existing = subscription_tasks
                .lock()
                .await
                .remove(&relay_subscription_id);
            if let Some(task) = existing {
                task.abort();
            }
            let ack =
                encrypt_json_response(app, &client_public_key, serde_json::json!({ "ok": true }))
                    .await?;
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: Some(ack),
                    error: None,
                },
            )?;
        }
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

#[derive(Debug, Clone)]
struct RelayRequestOutcome {
    encrypted_response: Option<EncryptedRelayPayload>,
    error: Option<RelayError>,
}

async fn handle_daemon_request(
    app: &Arc<Mutex<DaemonApp>>,
    encrypted_request: EncryptedRelayPayload,
) -> RelayRequestOutcome {
    let (request, client_public_key, daemon_private_key) = {
        let app = app.lock().await;
        let daemon_private_key = app.config().relay_private_key.clone();
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

    if !is_supported_relay_request(&request) {
        return RelayRequestOutcome {
            encrypted_response: None,
            error: Some(relay_error(
                "unsupported_request",
                "relay transport does not yet support this request type",
                false,
            )),
        };
    }

    let result = {
        let mut app = app.lock().await;
        app.handle_local_request(request)
    };
    match result {
        Ok(response) => {
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
            match relay_crypto::encrypt_payload_for_peer(
                &daemon_private_key,
                &client_public_key,
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

fn is_supported_relay_request(request: &LocalDaemonRequest) -> bool {
    matches!(
        request,
        LocalDaemonRequest::ListSessions(_)
            | LocalDaemonRequest::GetSessionState(_)
            | LocalDaemonRequest::AttachToSession(_)
            | LocalDaemonRequest::ResolveSession(_)
            | LocalDaemonRequest::DetachFromSession(_)
            | LocalDaemonRequest::GetProviderRun(_)
            | LocalDaemonRequest::GetProviderCatalog(_)
            | LocalDaemonRequest::GetProviderCommandCatalogs(_)
            | LocalDaemonRequest::GetProviderAuthStatus(_)
            | LocalDaemonRequest::StartProviderLogin(_)
            | LocalDaemonRequest::LogoutProvider(_)
            | LocalDaemonRequest::SubmitPrompt(_)
            | LocalDaemonRequest::CompletePrompt(_)
            | LocalDaemonRequest::CancelActivePrompt(_)
            | LocalDaemonRequest::UpdateSessionConfig(_)
            | LocalDaemonRequest::ResizeTerminal(_)
            | LocalDaemonRequest::PumpTerminalOutput(_)
            | LocalDaemonRequest::LaunchProviderRun(_)
            | LocalDaemonRequest::FocusAgent(_)
            | LocalDaemonRequest::CycleAgentFocus(_)
            | LocalDaemonRequest::ListAgents(_)
            | LocalDaemonRequest::EndSession(_)
    )
}

fn map_relay_error(error: &DaemonError) -> RelayError {
    match error {
        DaemonError::SessionNotFound { .. } => {
            relay_error("session_not_found", &error.to_string(), false)
        }
        DaemonError::AttachmentNotFound { .. } => {
            relay_error("attachment_not_found", &error.to_string(), false)
        }
        DaemonError::AttachmentNotInSession { .. } => {
            relay_error("attachment_not_in_session", &error.to_string(), false)
        }
        DaemonError::NoActiveProviderRun { .. } => {
            relay_error("no_active_provider_run", &error.to_string(), false)
        }
        DaemonError::LocalTransport { .. } => {
            relay_error("transport_error", &error.to_string(), true)
        }
        _ => relay_error("relay_request_failed", &error.to_string(), false),
    }
}

fn relay_error(code: &str, message: &str, retryable: bool) -> RelayError {
    RelayError {
        code: code.to_string(),
        message: message.to_string(),
        retryable,
    }
}

async fn encrypt_json_response(
    app: &Arc<Mutex<DaemonApp>>,
    client_public_key: &str,
    value: serde_json::Value,
) -> Result<EncryptedRelayPayload, DaemonError> {
    let daemon_private_key = {
        let app = app.lock().await;
        app.config().relay_private_key.clone()
    };
    let plaintext = serde_json::to_vec(&value).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay response",
        message: error.to_string(),
    })?;
    relay_crypto::encrypt_payload_for_peer(&daemon_private_key, client_public_key, &plaintext)
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

async fn abort_subscription_tasks(subscription_tasks: &RelaySubscriptionTasks) {
    let mut guard = subscription_tasks.lock().await;
    for (_, task) in guard.iter() {
        task.abort();
    }
    guard.clear();
}

async fn run_relay_subscription_loop(
    app: Arc<Mutex<DaemonApp>>,
    outgoing_tx: mpsc::UnboundedSender<RelayEnvelope>,
    subscription_id: String,
    client_public_key: String,
    session_id: String,
    attachment_id: String,
    event_counter: Arc<AtomicU64>,
) {
    let mut previous_snapshot = None;
    let mut tick: u64 = 0;

    loop {
        let watch_result = {
            let mut app = app.lock().await;
            watch_subscription_state(
                &mut app,
                &session_id,
                &attachment_id,
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
                    && emit_relay_event(
                        &app,
                        &outgoing_tx,
                        &subscription_id,
                        &client_public_key,
                        &event_counter,
                        KernelEvent::TerminalOutput { records },
                    )
                    .await
                    .is_err()
                {
                    break;
                }
                if !notices.is_empty()
                    && emit_relay_event(
                        &app,
                        &outgoing_tx,
                        &subscription_id,
                        &client_public_key,
                        &event_counter,
                        KernelEvent::RuntimeNotices { notices },
                    )
                    .await
                    .is_err()
                {
                    break;
                }
                for completion in completions {
                    if emit_relay_event(
                        &app,
                        &outgoing_tx,
                        &subscription_id,
                        &client_public_key,
                        &event_counter,
                        KernelEvent::AssistantMessageCompleted {
                            session_id: completion.session_id,
                            provider_run_id: completion.provider_run_id,
                            agent_id: completion.agent_id,
                            recipient_attachment_ids: completion.recipient_attachment_ids,
                            message_id: completion.message_id,
                            completed_at_ms: completion.completed_at_ms,
                        },
                    )
                    .await
                    .is_err()
                    {
                        return;
                    }
                }
                if let Some(snapshot) = *snapshot {
                    previous_snapshot = Some(snapshot.clone());
                    if emit_relay_event(
                        &app,
                        &outgoing_tx,
                        &subscription_id,
                        &client_public_key,
                        &event_counter,
                        KernelEvent::SessionSnapshot {
                            session: Box::new(snapshot.0),
                            provider_run: Box::new(snapshot.1),
                        },
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                }
                if tick.is_multiple_of(RELAY_HEARTBEAT_INTERVAL_TICKS)
                    && emit_relay_event(
                        &app,
                        &outgoing_tx,
                        &subscription_id,
                        &client_public_key,
                        &event_counter,
                        KernelEvent::Heartbeat {
                            session_id: session_id.clone(),
                        },
                    )
                    .await
                    .is_err()
                {
                    break;
                }
            }
            WatchResult::Unavailable(message) => {
                let _ = emit_relay_event(
                    &app,
                    &outgoing_tx,
                    &subscription_id,
                    &client_public_key,
                    &event_counter,
                    KernelEvent::SessionUnavailable {
                        session_id: session_id.clone(),
                        message,
                    },
                )
                .await;
                break;
            }
        }

        tick = tick.wrapping_add(1);
        sleep(Duration::from_millis(WATCH_INTERVAL_MS)).await;
    }
}

async fn emit_relay_event(
    app: &Arc<Mutex<DaemonApp>>,
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    subscription_id: &str,
    client_public_key: &str,
    event_counter: &Arc<AtomicU64>,
    event: KernelEvent,
) -> Result<(), DaemonError> {
    let daemon_private_key = {
        let app = app.lock().await;
        app.config().relay_private_key.clone()
    };
    let plaintext = serde_json::to_vec(&event).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay event",
        message: error.to_string(),
    })?;
    let encrypted_event =
        relay_crypto::encrypt_payload_for_peer(&daemon_private_key, client_public_key, &plaintext)?;
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonEvent {
            subscription_id: subscription_id.to_string(),
            event_id: event_counter.fetch_add(1, Ordering::Relaxed) + 1,
            encrypted_event,
        },
    )
}

async fn set_connected(state: &Arc<RwLock<RelayClientState>>, connected: bool) {
    state.write().await.connected = connected;
}

#[cfg(test)]
mod tests {
    use super::*;

    use arroba_relay::{protocol::ClientTarget, RelayConfig, RelayServer};
    use tokio::sync::oneshot;
    use tokio::time::{sleep, Duration};

    use crate::attachment::ClientCapabilityLevel;
    use crate::config::DaemonConfig;
    use crate::local::{
        AttachToSessionRequest, DetachFromSessionRequest, FocusAgentRequest,
        GetSessionStateRequest, ListSessionsRequest, LocalDaemonResponse, ResizeTerminalRequest,
        ResolveSessionRequest, UpdateSessionConfigRequest,
    };
    use crate::session::CreateSessionRequest;
    use crate::transport::relay_crypto;
    use std::collections::BTreeMap;

    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_connector_registers_with_relay() {
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
    async fn proxied_session_requests_are_handled_through_relay() {
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
            let response = app
                .handle_local_request(LocalDaemonRequest::CreateSession(
                    CreateSessionRequest::new("workspace-relay-test", "worktree-relay-test"),
                ))
                .expect("session should be created");
            match response {
                LocalDaemonResponse::SessionCreated { session, .. } => session.id().to_string(),
                other => panic!("unexpected response: {other:?}"),
            }
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
            LocalDaemonResponse::SessionState { session } if session.id() == created_session_id
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

        let _ = shutdown_tx.send(true);
        connector_task.await.expect("connector task should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn proxied_session_subscriptions_are_forwarded_through_relay() {
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
            let response = app
                .handle_local_request(LocalDaemonRequest::CreateSession(
                    CreateSessionRequest::new("workspace-relay-test", "worktree-relay-test"),
                ))
                .expect("session should be created");
            match response {
                LocalDaemonResponse::SessionCreated { session, .. } => session.id().to_string(),
                other => panic!("unexpected response: {other:?}"),
            }
        };
        let attachment_id = {
            let mut app = app.lock().await;
            let response = app
                .handle_local_request(LocalDaemonRequest::AttachToSession(
                    AttachToSessionRequest {
                        session_id: created_session_id.clone(),
                        client_id: "relay-client".to_string(),
                        capability_level: ClientCapabilityLevel::MessageTransport,
                    },
                ))
                .expect("session should attach");
            match response {
                LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
                other => panic!("unexpected response: {other:?}"),
            }
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
    async fn interactive_session_requests_are_handled_through_relay() {
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
            let response = app
                .handle_local_request(LocalDaemonRequest::CreateSession(
                    CreateSessionRequest::new("workspace-relay-test", "worktree-relay-test")
                        .with_alias("main"),
                ))
                .expect("session should be created");
            match response {
                LocalDaemonResponse::SessionCreated { session, agent } => {
                    (session.id().to_string(), agent.id().to_string())
                }
                other => panic!("unexpected response: {other:?}"),
            }
        };
        let attachment_id = {
            let mut app = app.lock().await;
            let response = app
                .handle_local_request(LocalDaemonRequest::AttachToSession(
                    AttachToSessionRequest {
                        session_id: created_session_id.clone(),
                        client_id: "relay-client".to_string(),
                        capability_level: ClientCapabilityLevel::MessageTransport,
                    },
                ))
                .expect("session should attach");
            match response {
                LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
                other => panic!("unexpected response: {other:?}"),
            }
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
            let response = app
                .handle_local_request(LocalDaemonRequest::CreateSession(
                    CreateSessionRequest::new("workspace-relay-test", "worktree-relay-test"),
                ))
                .expect("session should be created");
            match response {
                LocalDaemonResponse::SessionCreated { session, .. } => session.id().to_string(),
                other => panic!("unexpected response: {other:?}"),
            }
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
        for _ in 0..40 {
            if registry.read().await.daemon(daemon_id).is_some() {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
        panic!("daemon `{daemon_id}` did not register with relay");
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
                        serde_json::from_slice(&decrypted.plaintext)
                            .expect("local response should deserialize")
                    }
                    other => panic!("unexpected envelope: {other:?}"),
                }
            }
            other => panic!("unexpected relay message: {other:?}"),
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
                        serde_json::from_slice(&decrypted.plaintext)
                            .expect("json response should deserialize")
                    }
                    other => panic!("unexpected envelope: {other:?}"),
                }
            }
            other => panic!("unexpected relay message: {other:?}"),
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
