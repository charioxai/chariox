use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use arroba_relay::protocol::{ClientTarget, EncryptedRelayPayload, RelayEnvelope, RelayError};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::kernel_transport::{
    event_is_relevant_to_attachment, event_session_id, watch_subscription_state, KernelEvent,
    WatchResult, RECENT_EVENT_LIMIT, WATCH_INTERVAL_MS,
};
use crate::local::LocalDaemonRequest;
use crate::transport::relay_crypto;
use crate::transport::relay_discovery;
use crate::transport::relay_peer::{RelayPeerEvent, RelayPeerRequest, RelayPeerResponse};

#[allow(dead_code)]
#[derive(Debug)]
pub struct RelayClientState {
    connected: bool,
    outgoing_tx: Option<mpsc::UnboundedSender<RelayEnvelope>>,
    pending_peer_requests: BTreeMap<String, oneshot::Sender<RelayPeerResponseEnvelope>>,
    next_peer_request_id: u64,
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

type RelaySubscriptionTasks = Arc<Mutex<BTreeMap<String, JoinHandle<()>>>>;

#[derive(Debug, Clone)]
struct PersistedRelayEvent {
    event_id: u64,
    event: KernelEvent,
}

#[derive(Debug, Default)]
struct RelayEventRuntime {
    event_counter: AtomicU64,
    recent_events: Mutex<BTreeMap<String, VecDeque<PersistedRelayEvent>>>,
}

pub async fn run_daemon_relay_connector(
    app: Arc<Mutex<DaemonApp>>,
    state: Arc<RwLock<RelayClientState>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let event_runtime = Arc::new(RelayEventRuntime::default());
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
            set_disconnected(&state).await;
            return;
        }

        match connect_async(&relay_url).await {
            Ok((socket, _)) => {
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
                let daemon_id = {
                    let app = app.lock().await;
                    app.config().daemon_id.clone()
                };
                let register = {
                    let mut app = app.lock().await;
                    RelayEnvelope::DaemonRegister {
                        registration: app.relay_registration(),
                    }
                };
                if outgoing_tx.send(register).is_err() {
                    writer_task.abort();
                    set_disconnected(&state).await;
                    sleep(Duration::from_secs(1)).await;
                    continue;
                }
                set_connected(&state, outgoing_tx.clone()).await;

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
                                set_disconnected(&state).await;
                                return;
                            }
                        }
                        incoming = reader.next() => {
                            match incoming {
                                Some(Ok(Message::Text(payload))) => {
                                    if handle_incoming_envelope(
                                        &app,
                                        &state,
                                        &outgoing_tx,
                                        &subscription_tasks,
                                        &event_runtime,
                                        &payload,
                                    )
                                    .await
                                    .is_err()
                                    {
                                        abort_subscription_tasks(&subscription_tasks).await;
                                        writer_task.abort();
                                        set_disconnected(&state).await;
                                        break;
                                    }
                                }
                                Some(Ok(Message::Close(_))) => {
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    set_disconnected(&state).await;
                                    break;
                                }
                                Some(Ok(_)) => {}
                                Some(Err(_)) | None => {
                                    abort_subscription_tasks(&subscription_tasks).await;
                                    writer_task.abort();
                                    set_disconnected(&state).await;
                                    break;
                                }
                            }
                        }
                        writer_done = &mut writer_done_rx => {
                            let _ = writer_done;
                            abort_subscription_tasks(&subscription_tasks).await;
                            writer_task.abort();
                            set_disconnected(&state).await;
                            break;
                        }
                        _ = sleep(heartbeat) => {
                            let heartbeat_frame = RelayEnvelope::DaemonHeartbeat {
                                daemon_id: daemon_id.clone(),
                            };
                            if outgoing_tx.send(heartbeat_frame).is_err() {
                                abort_subscription_tasks(&subscription_tasks).await;
                                writer_task.abort();
                                set_disconnected(&state).await;
                                break;
                            }
                        }
                    }
                }
            }
            Err(_) => {
                set_disconnected(&state).await;
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
        RelayEnvelope::DaemonIncomingPeerRequest {
            relay_request_id,
            from_daemon_id: _,
            encrypted_request,
        } => {
            let relay_response =
                handle_daemon_peer_request(app, outgoing_tx, encrypted_request).await;
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonIncomingPeerResponse {
                    relay_request_id,
                    encrypted_response: relay_response.encrypted_response,
                    error: relay_response.error,
                },
            )?;
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
            encrypted_event,
        } => {
            handle_daemon_peer_event(app, encrypted_event).await?;
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
            send_outgoing_envelope(
                outgoing_tx,
                RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: Some(ack),
                    error: None,
                },
            )?;
            replay_recent_relay_events(
                event_runtime,
                app,
                outgoing_tx,
                &relay_subscription_id,
                &client_public_key,
                &session_id,
                &attachment_id,
                resume_from_event_id,
            )
            .await?;
            let task = tokio::spawn(run_relay_subscription_loop(
                Arc::clone(app),
                outgoing_tx.clone(),
                relay_subscription_id.clone(),
                client_public_key.clone(),
                session_id.clone(),
                attachment_id.clone(),
                Arc::clone(event_runtime),
            ));
            subscription_tasks
                .lock()
                .await
                .insert(relay_subscription_id.clone(), task);
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

#[allow(dead_code)]
#[derive(Debug)]
struct RelayPeerResponseEnvelope {
    from_daemon_id: String,
    encrypted_response: Option<EncryptedRelayPayload>,
    error: Option<RelayError>,
}

async fn handle_daemon_peer_request(
    app: &Arc<Mutex<DaemonApp>>,
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    encrypted_request: EncryptedRelayPayload,
) -> RelayRequestOutcome {
    let (request, requester_public_key, daemon_private_key, daemon_id) = {
        let app = app.lock().await;
        let daemon_private_key = app.config().relay_private_key.clone();
        let daemon_id = app.config().daemon_id.clone();
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
        } => {
            let lease = {
                let mut app = app.lock().await;
                app.create_execution_lease(&home_kernel_id, &home_session_id, &home_agent_id)
            };
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
            let destroyed = {
                let mut app = app.lock().await;
                app.destroy_execution_lease(&lease_id)
            };
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
        } => {
            let leased_agent = {
                let mut app = app.lock().await;
                app.create_leased_agent(&lease_id, &provider, model, effort)
            };
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
            let destroyed = {
                let mut app = app.lock().await;
                app.destroy_leased_agent(&leased_agent_id)
            };
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
        RelayPeerRequest::SubmitLeasedPrompt {
            leased_agent_id,
            prompt,
            attachments,
        } => {
            let submitted = {
                let mut app = app.lock().await;
                app.submit_leased_prompt(&leased_agent_id, &prompt, attachments)
            };
            match submitted {
                Ok((provider_run_id, outcome)) => {
                    if let Err(error) = emit_leased_projection_event(
                        app,
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
            let completion = {
                let mut app = app.lock().await;
                app.complete_leased_prompt(&leased_agent_id)
            };
            match completion {
                Ok(completion) => {
                    let provider_run_id = app
                        .lock()
                        .await
                        .leased_agent_provider_run_id(&leased_agent_id)
                        .ok()
                        .flatten();
                    RelayPeerResponse::LeasedPromptCompleted {
                        provider_run_id,
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
            let cancellation = {
                let mut app = app.lock().await;
                app.cancel_leased_prompt(&leased_agent_id)
            };
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
    app: &Arc<Mutex<DaemonApp>>,
    encrypted_event: EncryptedRelayPayload,
) -> Result<(), DaemonError> {
    let daemon_private_key = {
        let app = app.lock().await;
        app.config().relay_private_key.clone()
    };
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
            let mut app = app.lock().await;
            app.project_remote_runtime_projection(
                &home_session_id,
                &home_agent_id,
                &provider_run_id,
                output_chunks,
                notices,
                completions,
            )?;
        }
    }
    Ok(())
}

async fn emit_leased_projection_event(
    app: &Arc<Mutex<DaemonApp>>,
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    leased_agent_id: &str,
    provider_run_id: &str,
    pump_output: bool,
) -> Result<(), DaemonError> {
    let (config, target_daemon_id, event) = {
        let mut app = app.lock().await;
        let config = app.config().clone();
        let Some((target_daemon_id, event)) =
            app.drain_leased_runtime_projection(leased_agent_id, provider_run_id, pump_output)?
        else {
            return Ok(());
        };
        (config, target_daemon_id, event)
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

#[allow(dead_code)]
pub async fn send_peer_request_via_relay(
    app: &Arc<Mutex<DaemonApp>>,
    state: &Arc<RwLock<RelayClientState>>,
    target: ClientTarget,
    request: RelayPeerRequest,
) -> Result<RelayPeerResponse, DaemonError> {
    let target_ref = target
        .daemon_id
        .as_deref()
        .or(target.daemon_alias.as_deref())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "peer target must include daemon id or alias".to_string(),
        })?;
    let config = {
        let app = app.lock().await;
        app.config().clone()
    };
    let kernel = relay_discovery::get_live_kernel(&config, target_ref).await?;
    let plaintext = serde_json::to_vec(&request).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay peer request",
        message: error.to_string(),
    })?;
    let encrypted_request = relay_crypto::encrypt_payload_for_peer(
        &config.relay_private_key,
        &kernel.public_key,
        &plaintext,
    )?;
    let (request_id, response_rx, outgoing_tx) = {
        let mut guard = state.write().await;
        let Some(outgoing_tx) = guard.outgoing_tx.clone() else {
            return Err(DaemonError::LocalTransport {
                operation: "send relay peer request",
                message: "relay is not connected".to_string(),
            });
        };
        guard.next_peer_request_id += 1;
        let request_id = format!("daemon-peer-{}", guard.next_peer_request_id);
        let (response_tx, response_rx) = oneshot::channel();
        guard
            .pending_peer_requests
            .insert(request_id.clone(), response_tx);
        (request_id, response_rx, outgoing_tx)
    };
    if outgoing_tx
        .send(RelayEnvelope::DaemonPeerRequest {
            request_id: request_id.clone(),
            target,
            encrypted_request,
        })
        .is_err()
    {
        let mut guard = state.write().await;
        guard.pending_peer_requests.remove(&request_id);
        return Err(DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "relay is not connected".to_string(),
        });
    }
    let envelope = response_rx.await.map_err(|_| DaemonError::LocalTransport {
        operation: "read relay peer response",
        message: "relay peer request was cancelled".to_string(),
    })?;
    if let Some(error) = envelope.error {
        return Err(DaemonError::LocalTransport {
            operation: "read relay peer response",
            message: error.message,
        });
    }
    let encrypted_response =
        envelope
            .encrypted_response
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "read relay peer response",
                message: format!(
                    "peer `{}` returned no response payload",
                    envelope.from_daemon_id
                ),
            })?;
    let decrypted = relay_crypto::decrypt_payload_for_private_key(
        &config.relay_private_key,
        &encrypted_response,
    )?;
    serde_json::from_slice::<RelayPeerResponse>(&decrypted.plaintext).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "decode relay peer response",
            message: error.to_string(),
        }
    })
}

pub async fn send_peer_request_via_temporary_connection(
    config: &crate::config::DaemonConfig,
    target: ClientTarget,
    request: RelayPeerRequest,
) -> Result<RelayPeerResponse, DaemonError> {
    let target_ref = target
        .daemon_id
        .as_deref()
        .or(target.daemon_alias.as_deref())
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "peer target must include daemon id or alias".to_string(),
        })?;
    let relay_url = config
        .relay_url
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "relay_url is not configured".to_string(),
        })?;
    let relay_token = config
        .relay_token
        .clone()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: "relay_token is not configured".to_string(),
        })?;
    let kernel = relay_discovery::get_live_kernel(config, target_ref).await?;
    let plaintext = serde_json::to_vec(&request).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize relay peer request",
        message: error.to_string(),
    })?;
    let encrypted_request = relay_crypto::encrypt_payload_for_peer(
        &config.relay_private_key,
        &kernel.public_key,
        &plaintext,
    )?;
    let (mut socket, _) =
        connect_async(&relay_url)
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "connect temporary relay peer socket",
                message: error.to_string(),
            })?;
    let request_id = format!(
        "daemon-peer-tmp-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    );
    let register = RelayEnvelope::DaemonRegister {
        registration: arroba_relay::protocol::DaemonRegistration {
            auth_token: relay_token,
            daemon_id: format!("{}:peer-tmp:{}", config.daemon_id, request_id),
            machine_id: config.host_machine_id.clone(),
            machine_alias: config.host_machine_alias.clone(),
            daemon_alias: config.daemon_alias.clone(),
            kernel_alias: config.daemon_alias.clone(),
            public_key: config.relay_public_key.clone(),
            capabilities: vec!["relay_peer_transport".to_string()],
            available_providers: Vec::new(),
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 0,
        },
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&register)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "serialize temporary relay register",
                    message: error.to_string(),
                })?
                .into(),
        ))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "write temporary relay register",
            message: error.to_string(),
        })?;
    socket
        .send(Message::Text(
            serde_json::to_string(&RelayEnvelope::DaemonPeerRequest {
                request_id: request_id.clone(),
                target,
                encrypted_request,
            })
            .map_err(|error| DaemonError::LocalTransport {
                operation: "serialize temporary relay peer request",
                message: error.to_string(),
            })?
            .into(),
        ))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "write temporary relay peer request",
            message: error.to_string(),
        })?;
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                let envelope = serde_json::from_str::<RelayEnvelope>(&text).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "decode temporary relay peer response",
                        message: error.to_string(),
                    }
                })?;
                if let RelayEnvelope::DaemonPeerResponse {
                    request_id: response_request_id,
                    from_daemon_id: _,
                    encrypted_response,
                    error,
                } = envelope
                {
                    if response_request_id != request_id {
                        continue;
                    }
                    if let Some(error) = error {
                        return Err(DaemonError::LocalTransport {
                            operation: "read temporary relay peer response",
                            message: error.message,
                        });
                    }
                    let encrypted_response =
                        encrypted_response.ok_or_else(|| DaemonError::LocalTransport {
                            operation: "read temporary relay peer response",
                            message: "peer returned no response payload".to_string(),
                        })?;
                    let decrypted = relay_crypto::decrypt_payload_for_private_key(
                        &config.relay_private_key,
                        &encrypted_response,
                    )?;
                    return serde_json::from_slice::<RelayPeerResponse>(&decrypted.plaintext)
                        .map_err(|error| DaemonError::LocalTransport {
                            operation: "decode temporary relay peer response",
                            message: error.to_string(),
                        });
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                return Err(DaemonError::LocalTransport {
                    operation: "read temporary relay peer response",
                    message: "relay closed temporary peer connection".to_string(),
                });
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                return Err(DaemonError::LocalTransport {
                    operation: "read temporary relay peer response",
                    message: error.to_string(),
                });
            }
        }
    }
}

async fn resolve_pending_peer_response(
    state: &Arc<RwLock<RelayClientState>>,
    request_id: String,
    response: RelayPeerResponseEnvelope,
) {
    let sender = state
        .write()
        .await
        .pending_peer_requests
        .remove(&request_id);
    if let Some(sender) = sender {
        let _ = sender.send(response);
    }
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
        DaemonError::RemoteLeasesDisabled { .. } => {
            relay_error("remote_leases_disabled", &error.to_string(), false)
        }
        DaemonError::ExecutionLeaseNotFound { .. } => {
            relay_error("execution_lease_not_found", &error.to_string(), false)
        }
        DaemonError::LeasedAgentNotFound { .. } => {
            relay_error("leased_agent_not_found", &error.to_string(), false)
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
    event_runtime: Arc<RelayEventRuntime>,
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
                        &event_runtime,
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
                        &event_runtime,
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
                        &event_runtime,
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
                        &event_runtime,
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
                        &event_runtime,
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
                    &event_runtime,
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
    event_runtime: &Arc<RelayEventRuntime>,
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
    let event_id = event_runtime.event_counter.fetch_add(1, Ordering::Relaxed) + 1;
    if let Some(session_id) = event_session_id(&event) {
        let mut recent_events = event_runtime.recent_events.lock().await;
        let entry = recent_events.entry(session_id.to_string()).or_default();
        entry.push_back(PersistedRelayEvent {
            event_id,
            event: event.clone(),
        });
        while entry.len() > RECENT_EVENT_LIMIT {
            entry.pop_front();
        }
    }
    send_relay_event_frame(outgoing_tx, subscription_id, event_id, encrypted_event)
}

async fn replay_recent_relay_events(
    event_runtime: &Arc<RelayEventRuntime>,
    app: &Arc<Mutex<DaemonApp>>,
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    subscription_id: &str,
    client_public_key: &str,
    session_id: &str,
    attachment_id: &str,
    resume_from_event_id: Option<u64>,
) -> Result<(), DaemonError> {
    let Some(cursor) = resume_from_event_id else {
        return Ok(());
    };
    let recent_events = event_runtime.recent_events.lock().await;
    let events = recent_events
        .get(session_id)
        .map(|events| events.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    drop(recent_events);
    for persisted in events {
        if persisted.event_id <= cursor {
            continue;
        }
        if !event_is_relevant_to_attachment(&persisted.event, attachment_id) {
            continue;
        }
        let daemon_private_key = {
            let app = app.lock().await;
            app.config().relay_private_key.clone()
        };
        let plaintext =
            serde_json::to_vec(&persisted.event).map_err(|error| DaemonError::LocalTransport {
                operation: "serialize relay event",
                message: error.to_string(),
            })?;
        let encrypted_event = relay_crypto::encrypt_payload_for_peer(
            &daemon_private_key,
            client_public_key,
            &plaintext,
        )?;
        send_relay_event_frame(
            outgoing_tx,
            subscription_id,
            persisted.event_id,
            encrypted_event,
        )?;
    }
    emit_relay_event(
        app,
        outgoing_tx,
        subscription_id,
        client_public_key,
        event_runtime,
        KernelEvent::TransportResumed {
            session_id: session_id.to_string(),
            resumed_from_event_id: Some(cursor),
        },
    )
    .await
}

fn send_relay_event_frame(
    outgoing_tx: &mpsc::UnboundedSender<RelayEnvelope>,
    subscription_id: &str,
    event_id: u64,
    encrypted_event: EncryptedRelayPayload,
) -> Result<(), DaemonError> {
    send_outgoing_envelope(
        outgoing_tx,
        RelayEnvelope::DaemonEvent {
            subscription_id: subscription_id.to_string(),
            event_id,
            encrypted_event,
        },
    )
}

async fn set_connected(
    state: &Arc<RwLock<RelayClientState>>,
    outgoing_tx: mpsc::UnboundedSender<RelayEnvelope>,
) {
    let mut guard = state.write().await;
    guard.connected = true;
    guard.outgoing_tx = Some(outgoing_tx);
}

async fn set_disconnected(state: &Arc<RwLock<RelayClientState>>) {
    let pending = {
        let mut guard = state.write().await;
        guard.connected = false;
        guard.outgoing_tx = None;
        std::mem::take(&mut guard.pending_peer_requests)
    };
    for (_, sender) in pending {
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
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::config::DaemonConfig;
    use crate::local::{
        AttachToSessionRequest, DetachFromSessionRequest, FocusAgentRequest,
        GetSessionStateRequest, ListSessionsRequest, LocalDaemonResponse, ResizeTerminalRequest,
        ResolveSessionRequest, UpdateSessionConfigRequest,
    };
    use crate::session::CreateSessionRequest;
    use crate::transport::relay_crypto;
    use crate::transport::relay_discovery;
    use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
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
    async fn proxied_peer_requests_are_handled_through_relay() {
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
        assert_eq!(app_b.lock().await.execution_lease_count(), 1);

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
        assert_eq!(app_b.lock().await.execution_lease_count(), 0);

        let _ = shutdown_a_tx.send(true);
        let _ = shutdown_b_tx.send(true);
        connector_a.await.expect("connector A should join");
        connector_b.await.expect("connector B should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn leased_agents_are_spawned_and_destroyed_through_peer_transport() {
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
            let (session, agent) = app
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
        assert_eq!(app_b.lock().await.leased_agent_count(), 1);

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
        assert_eq!(app_b.lock().await.leased_agent_count(), 0);

        let _ = shutdown_a_tx.send(true);
        let _ = shutdown_b_tx.send(true);
        connector_a.await.expect("connector A should join");
        connector_b.await.expect("connector B should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agents_can_be_spawned_on_a_remote_machine_and_cleaned_up() {
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
        let state_home = Arc::new(RwLock::new(RelayClientState::default()));
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
        let state_worker = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_worker_tx, shutdown_worker_rx) = watch::channel(false);
        let connector_worker = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app_worker),
            Arc::clone(&state_worker),
            shutdown_worker_rx,
        ));

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
                    .find(|provider| provider.as_str() == "dev-stub")
            })
            .cloned()
            .expect("worker should advertise dev-stub");

        let session_id = {
            let mut app = app_home.lock().await;
            let (session, _) = app
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("home session should be created");
            session.id().to_string()
        };

        let remote_agent = {
            let mut app = app_home.lock().await;
            app.spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("remote-reviewer")
                    .with_model("default")
                    .with_effort("medium")
                    .with_machine("builder-west"),
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
            let app = app_worker.lock().await;
            assert_eq!(app.execution_lease_count(), 1);
            assert_eq!(app.leased_agent_count(), 1);
        }

        {
            let mut app = app_home.lock().await;
            let destroyed = app
                .destroy_agent(remote_agent.id())
                .expect("remote agent should destroy");
            assert_eq!(destroyed.id(), remote_agent.id());
        }

        {
            let app = app_worker.lock().await;
            assert_eq!(app.execution_lease_count(), 0);
            assert_eq!(app.leased_agent_count(), 0);
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
        let state_worker = Arc::new(RwLock::new(RelayClientState::default()));
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
                    .find(|provider| provider.as_str() == "dev-stub")
            })
            .cloned()
            .expect("worker should advertise dev-stub");

        let mut app_home =
            DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap");

        let (session_id, attachment_id) = {
            let (session, _) = app_home
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("home session should be created");
            let attachment = app_home
                .attach(AttachRequest::new(
                    session.id(),
                    "home-client",
                    ClientCapabilityLevel::InteractiveStructured,
                ))
                .expect("home attachment should attach");
            (session.id().to_string(), attachment.id().to_string())
        };

        let remote_agent_id = {
            app_home
                .spawn_agent(
                    CreateAgentRequest::new(&session_id, &provider)
                        .with_alias("remote-reviewer")
                        .with_model("default")
                        .with_effort("medium")
                        .with_machine("builder-west"),
                )
                .expect("remote agent should spawn")
                .id()
                .to_string()
        };

        let outcome = app_home
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
            .complete_active_prompt(&session_id, &remote_agent_id, None)
            .expect("remote prompt should complete");
        assert_eq!(completion.completed.target_agent_id(), remote_agent_id);
        assert_eq!(
            completion.completed.prompt(),
            "remote prompt over home session\n"
        );

        let _ = shutdown_worker_tx.send(true);
        connector_worker
            .await
            .expect("worker connector should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remote_machine_agents_materialize_file_attachments_on_the_worker() {
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
        let state_worker = Arc::new(RwLock::new(RelayClientState::default()));
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
                    .find(|provider| provider.as_str() == "dev-stub")
            })
            .cloned()
            .expect("worker should advertise dev-stub");

        let mut app_home =
            DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap");
        let (session_id, attachment_id, remote_agent_id, remote_leased_agent_id) = {
            let (session, _) = app_home
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("home session should be created");
            let attachment = app_home
                .attach(AttachRequest::new(
                    session.id(),
                    "home-client",
                    ClientCapabilityLevel::InteractiveStructured,
                ))
                .expect("home attachment should attach");
            let remote_agent = app_home
                .spawn_agent(
                    CreateAgentRequest::new(session.id(), &provider)
                        .with_alias("remote-reviewer")
                        .with_machine("builder-west"),
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

        let worker_attachments = app_worker
            .lock()
            .await
            .leased_agent_active_prompt_attachments(&remote_leased_agent_id)
            .expect("worker prompt attachments should be available");
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
        let _ = shutdown_worker_tx.send(true);
        connector_worker
            .await
            .expect("worker connector should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remote_machine_agents_cancel_prompts_through_the_home_session() {
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
        let state_worker = Arc::new(RwLock::new(RelayClientState::default()));
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
                    .find(|provider| provider.as_str() == "dev-stub")
            })
            .cloned()
            .expect("worker should advertise dev-stub");

        let mut app_home =
            DaemonApp::bootstrap(config_home.clone()).expect("home daemon should bootstrap");
        let (session_id, attachment_id) = {
            let (session, _) = app_home
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("home session should be created");
            let attachment = app_home
                .attach(AttachRequest::new(
                    session.id(),
                    "home-client",
                    ClientCapabilityLevel::InteractiveStructured,
                ))
                .expect("home attachment should attach");
            (session.id().to_string(), attachment.id().to_string())
        };
        let remote_agent_id = app_home
            .spawn_agent(
                CreateAgentRequest::new(&session_id, &provider)
                    .with_alias("remote-reviewer")
                    .with_model("default")
                    .with_machine("builder-west"),
            )
            .expect("remote agent should spawn")
            .id()
            .to_string();

        let outcome = app_home
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
            .cancel_active_prompt(&session_id, &attachment_id)
            .expect("remote prompt should cancel");
        assert_eq!(cancellation.prompt.target_agent_id(), remote_agent_id);
        assert_eq!(
            cancellation.prompt.status(),
            crate::session::PromptStatus::Cancelling
        );

        let _ = shutdown_worker_tx.send(true);
        connector_worker
            .await
            .expect("worker connector should join");
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn incoming_peer_events_project_runtime_to_the_home_session() {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should bootstrap"),
        ));
        let (session_id, agent_id, attachment_id, daemon_public_key) = {
            let mut app = app.lock().await;
            let (session, agent) = app
                .create_session(CreateSessionRequest::new("workspace-home", "worktree-home"))
                .expect("session should be created");
            let attachment = app
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

        handle_daemon_peer_event(&app, encrypted_event)
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
    async fn relay_subscription_replays_recent_events_after_resume_cursor() {
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
