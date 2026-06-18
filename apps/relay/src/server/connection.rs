use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use crate::auth::{
    RelayAction, RelayAuthError, RelayAuthRequest, RelayAuthVerifier, VerifiedRelayIdentity,
};
use crate::protocol::{
    ClientTarget, RelayCallerIdentity, RelayConnectionRole, RelayEnvelope, RelayError,
    RelayMetadataQuery,
};
use crate::registry::{
    ActiveSubscription, DaemonKey, DisplayStreamEvent, DisplayStreamSender, PeerHandle,
    PendingClientRequest, PendingDaemonPeerRequest, PendingRequestKind, RelayRegistry, RelaySender,
};

const RELAY_OUTGOING_QUEUE_CAPACITY: usize = 1024;
const RELAY_WEBSOCKET_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const RELAY_FIRST_MESSAGE_TIMEOUT: Duration = Duration::from_secs(10);
const RELAY_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const RELAY_CONNECTION_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const RELAY_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const RELAY_PONG_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) async fn handle_connection(
    stream: TcpStream,
    peer_addr: SocketAddr,
    registry: Arc<RwLock<RelayRegistry>>,
    auth_verifier: RelayAuthVerifier,
    relay_request_counter: Arc<AtomicU64>,
) -> Result<(), std::io::Error> {
    let socket =
        match tokio::time::timeout(RELAY_WEBSOCKET_HANDSHAKE_TIMEOUT, accept_async(stream)).await {
            Ok(Ok(socket)) => socket,
            Ok(Err(error)) => return Err(std::io::Error::other(error.to_string())),
            Err(_) => return Ok(()),
        };
    let (mut writer, mut reader) = socket.split();
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Message>(RELAY_OUTGOING_QUEUE_CAPACITY);
    let mut writer_task = tokio::spawn(async move {
        while let Some(message) = outgoing_rx.recv().await {
            if writer.send(message).await.is_err() {
                break;
            }
        }
    });
    let mut registered_daemon_key: Option<DaemonKey> = None;
    let mut first_message_received = false;
    let mut last_read_at = Instant::now();
    let mut last_ping_at = Instant::now() - RELAY_HEARTBEAT_INTERVAL;
    let mut connection_check = tokio::time::interval(RELAY_CONNECTION_CHECK_INTERVAL);
    connection_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    connection_check.tick().await;

    let connection_result: Result<(), std::io::Error> = async {
        loop {
            let message = tokio::select! {
                message = reader.next() => message,
                _ = connection_check.tick() => {
                    let elapsed = last_read_at.elapsed();
                    if (!first_message_received && elapsed >= RELAY_FIRST_MESSAGE_TIMEOUT)
                        || elapsed >= RELAY_IDLE_TIMEOUT
                    {
                        relay_log(
                            "warn",
                            "relay_connection_timeout",
                            json!({
                                "peer_addr": peer_addr.to_string(),
                                "reason": if !first_message_received { "first_message" } else { "idle" },
                                "elapsed_ms": elapsed.as_millis(),
                            }),
                        );
                        send_close(&outgoing_tx, "relay connection idle timeout".to_string());
                        break;
                    }
                    if last_ping_at > last_read_at && last_ping_at.elapsed() >= RELAY_PONG_TIMEOUT {
                        relay_log(
                            "warn",
                            "relay_connection_timeout",
                            json!({
                                "peer_addr": peer_addr.to_string(),
                                "reason": "heartbeat",
                                "elapsed_ms": last_ping_at.elapsed().as_millis(),
                            }),
                        );
                        send_close(&outgoing_tx, "relay heartbeat timeout".to_string());
                        break;
                    }
                    if last_ping_at.elapsed() >= RELAY_HEARTBEAT_INTERVAL {
                        if outgoing_tx.try_send(Message::Ping(Vec::new().into())).is_err() {
                            break;
                        }
                        last_ping_at = Instant::now();
                    }
                    continue;
                }
                _ = &mut writer_task => break,
            };
            let Some(message) = message else {
                break;
            };
            let message = message.map_err(|error| std::io::Error::other(error.to_string()))?;
            last_read_at = Instant::now();
            match message {
                Message::Text(text) => {
                    first_message_received = true;
                    let envelope: RelayEnvelope = serde_json::from_str(&text).map_err(|error| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
                    })?;
                    match envelope {
                        RelayEnvelope::DaemonRegister { registration } => {
                            let identity = verify_relay_token(
                                &auth_verifier,
                                &registration.auth_token,
                                RelayAction::DaemonRegister,
                                None,
                            )?;
                            let daemon_key = DaemonKey::new(
                                identity.realm_id.clone(),
                                registration.daemon_id.clone(),
                            );
                            if registered_daemon_key
                                .as_ref()
                                .is_some_and(|current_key| current_key != &daemon_key)
                            {
                                send_close(
                                    &outgoing_tx,
                                    "daemon connection already registered".to_string(),
                                );
                                break;
                            }
                            registered_daemon_key = Some(daemon_key.clone());
                            let mut replaced_senders = Vec::new();
                            let mut guard = registry.write().await;
                            guard.peers.retain(|addr, peer| {
                                let replace = *addr != peer_addr
                                    && peer.role == RelayConnectionRole::Daemon
                                    && peer.realm_id.as_deref() == Some(identity.realm_id.as_str())
                                    && peer
                                        .daemon_registration
                                        .as_ref()
                                        .map(|candidate| candidate.daemon_id.as_str())
                                        == Some(registration.daemon_id.as_str());
                                if replace {
                                    replaced_senders.push(peer.sender.clone());
                                }
                                !replace
                            });
                            guard.peers.insert(
                                peer_addr,
                                PeerHandle {
                                    sender: outgoing_tx.clone(),
                                    role: RelayConnectionRole::Daemon,
                                    realm_id: Some(identity.realm_id.clone()),
                                    identity: Some(identity.into()),
                                    daemon_registration: Some(registration.clone()),
                                },
                            );
                            guard.daemons.insert(daemon_key.clone(), registration);
                            guard.daemon_peers.insert(daemon_key, peer_addr);
                            drop(guard);
                            for sender in replaced_senders {
                                send_close(&sender, "daemon reconnected".to_string());
                            }
                        }
                        RelayEnvelope::DaemonHeartbeat {
                            daemon_id,
                            registration,
                        } => {
                            let Some(current_daemon_key) = registered_daemon_key.clone() else {
                                break;
                            };
                            if current_daemon_key.daemon_id != daemon_id {
                                break;
                            }
                            if let Some(registration) = registration {
                                let identity = verify_relay_token(
                                    &auth_verifier,
                                    &registration.auth_token,
                                    RelayAction::DaemonHeartbeat,
                                    Some(daemon_id.as_str()),
                                )?;
                                if identity.realm_id != current_daemon_key.realm_id {
                                    break;
                                }
                                if registration.daemon_id != daemon_id {
                                    break;
                                }
                                let mut guard = registry.write().await;
                                if let Some(peer) = guard.peers.get_mut(&peer_addr) {
                                    peer.realm_id = Some(identity.realm_id.clone());
                                    peer.identity = Some(identity.into());
                                    peer.daemon_registration = Some(registration.clone());
                                }
                                guard.daemons.insert(current_daemon_key, registration);
                            }
                        }
                        RelayEnvelope::ClientConnect { auth_token, target } => {
                            let identity = verify_relay_token(
                                &auth_verifier,
                                &auth_token,
                                RelayAction::ClientConnect,
                                target
                                    .daemon_id
                                    .as_deref()
                                    .or(target.daemon_alias.as_deref()),
                            )?;
                            let Some(daemon_key) =
                                resolve_target_daemon_key(&registry, &identity.realm_id, &target)
                                    .await
                            else {
                                log_target_not_connected(
                                    "client_connect",
                                    &registry,
                                    peer_addr,
                                    &identity.realm_id,
                                    &target,
                                )
                                .await;
                                send_close(
                                    &outgoing_tx,
                                    "target daemon is not connected to relay".to_string(),
                                );
                                break;
                            };
                            let daemon_public_key = {
                                let guard = registry.read().await;
                                guard
                                    .daemons
                                    .get(&daemon_key)
                                    .map(|registration| registration.public_key.clone())
                            };
                            let mut guard = registry.write().await;
                            guard.peers.insert(
                                peer_addr,
                                PeerHandle {
                                    sender: outgoing_tx.clone(),
                                    role: RelayConnectionRole::Client,
                                    realm_id: Some(identity.realm_id.clone()),
                                    identity: Some(identity.into()),
                                    daemon_registration: None,
                                },
                            );
                            send_envelope(
                                &outgoing_tx,
                                &RelayEnvelope::ClientConnected {
                                    target,
                                    daemon_public_key: daemon_public_key.unwrap_or_default(),
                                },
                            )?;
                        }
                        RelayEnvelope::ClientMetadataRequest {
                            request_id,
                            auth_token,
                            query,
                        } => {
                            let identity = verify_relay_token(
                                &auth_verifier,
                                &auth_token,
                                RelayAction::ClientMetadataRead,
                                None,
                            )?;
                            let guard = registry.read().await;
                            let (machines, kernels, kernel) = match query {
                                RelayMetadataQuery::ListLiveMachines => (
                                    Some(guard.live_machines_in_realm(&identity.realm_id)),
                                    None,
                                    None,
                                ),
                                RelayMetadataQuery::ListLiveKernelsForMachine { machine_ref } => (
                                    None,
                                    Some(guard.live_kernels_for_machine_in_realm(
                                        &identity.realm_id,
                                        &machine_ref,
                                    )),
                                    None,
                                ),
                                RelayMetadataQuery::GetLiveKernel { kernel_ref } => (
                                    None,
                                    None,
                                    guard.live_kernel_in_realm(&identity.realm_id, &kernel_ref),
                                ),
                            };
                            send_envelope(
                                &outgoing_tx,
                                &RelayEnvelope::ClientMetadataResponse {
                                    request_id,
                                    machines,
                                    kernels,
                                    kernel,
                                    error: None,
                                },
                            )?;
                        }
                        RelayEnvelope::DaemonPeerRequest {
                            request_id,
                            target,
                            encrypted_request,
                        } => {
                            let Some(requester_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before sending peer requests".to_string(),
                                );
                                break;
                            };
                            let Some(target_daemon_key) = resolve_target_daemon_key(
                                &registry,
                                &requester_daemon_key.realm_id,
                                &target,
                            )
                            .await
                            else {
                                log_target_not_connected(
                                    "daemon_peer_request",
                                    &registry,
                                    peer_addr,
                                    &requester_daemon_key.realm_id,
                                    &target,
                                )
                                .await;
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::DaemonPeerResponse {
                                        request_id,
                                        from_daemon_id: String::new(),
                                        encrypted_response: None,
                                        error: Some(relay_error(
                                            "target_not_connected",
                                            "target daemon is not connected to relay",
                                            true,
                                        )),
                                    },
                                )?;
                                continue;
                            };
                            let relay_request_id = format!(
                                "relay-peer-request-{}",
                                relay_request_counter.fetch_add(1, Ordering::Relaxed) + 1
                            );
                            let daemon_sender = {
                                let mut guard = registry.write().await;
                                guard.pending_daemon_peer_requests.insert(
                                    relay_request_id.clone(),
                                    PendingDaemonPeerRequest {
                                        requester_daemon_key: requester_daemon_key.clone(),
                                        requester_request_id: request_id.clone(),
                                        target_daemon_key: target_daemon_key.clone(),
                                    },
                                );
                                resolve_daemon_sender_locked(&guard, &target_daemon_key)
                            };
                            let Some(daemon_sender) = daemon_sender else {
                                registry
                                    .write()
                                    .await
                                    .pending_daemon_peer_requests
                                    .remove(&relay_request_id);
                                log_daemon_sender_missing(
                                    "daemon_peer_request",
                                    &registry,
                                    peer_addr,
                                    &target_daemon_key,
                                    &relay_request_id,
                                )
                                .await;
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::DaemonPeerResponse {
                                        request_id,
                                        from_daemon_id: target_daemon_key.daemon_id,
                                        encrypted_response: None,
                                        error: Some(relay_error(
                                            "target_not_connected",
                                            "target daemon is not connected to relay",
                                            true,
                                        )),
                                    },
                                )?;
                                continue;
                            };
                            send_envelope(
                                &daemon_sender,
                                &RelayEnvelope::DaemonIncomingPeerRequest {
                                    relay_request_id,
                                    from_daemon_id: requester_daemon_key.daemon_id,
                                    caller_identity: peer_identity(&registry, peer_addr).await,
                                    encrypted_request,
                                },
                            )?;
                        }
                        RelayEnvelope::DaemonPeerEvent {
                            target,
                            encrypted_event,
                        } => {
                            let Some(requester_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before sending peer events".to_string(),
                                );
                                break;
                            };
                            let Some(target_daemon_key) = resolve_target_daemon_key(
                                &registry,
                                &requester_daemon_key.realm_id,
                                &target,
                            )
                            .await
                            else {
                                log_target_not_connected(
                                    "daemon_peer_event",
                                    &registry,
                                    peer_addr,
                                    &requester_daemon_key.realm_id,
                                    &target,
                                )
                                .await;
                                continue;
                            };
                            let daemon_sender = {
                                let guard = registry.read().await;
                                resolve_daemon_sender_locked(&guard, &target_daemon_key)
                            };
                            if let Some(daemon_sender) = daemon_sender {
                                send_envelope(
                                    &daemon_sender,
                                    &RelayEnvelope::DaemonIncomingPeerEvent {
                                        from_daemon_id: requester_daemon_key.daemon_id,
                                        caller_identity: peer_identity(&registry, peer_addr).await,
                                        encrypted_event,
                                    },
                                )?;
                            }
                        }
                        RelayEnvelope::ClientRequest {
                            request_id,
                            target,
                            encrypted_request,
                        } => {
                            let Some(realm_id) =
                                connected_client_realm_id(&registry, peer_addr).await
                            else {
                                send_close(
                                    &outgoing_tx,
                                    "client must connect before sending requests".to_string(),
                                );
                                break;
                            };
                            let Some(daemon_key) =
                                resolve_target_daemon_key(&registry, &realm_id, &target).await
                            else {
                                log_target_not_connected(
                                    "client_request",
                                    &registry,
                                    peer_addr,
                                    &realm_id,
                                    &target,
                                )
                                .await;
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::ClientResponse {
                                        request_id,
                                        encrypted_response: None,
                                        error: Some(relay_error(
                                            "target_not_connected",
                                            "target daemon is not connected to relay",
                                            true,
                                        )),
                                    },
                                )?;
                                continue;
                            };
                            let relay_request_id = format!(
                                "relay-request-{}",
                                relay_request_counter.fetch_add(1, Ordering::Relaxed) + 1
                            );
                            let daemon_sender = {
                                let mut guard = registry.write().await;
                                guard.pending_requests.insert(
                                    relay_request_id.clone(),
                                    PendingClientRequest {
                                        client_addr: peer_addr,
                                        client_request_id: request_id.clone(),
                                        daemon_key: daemon_key.clone(),
                                        kind: PendingRequestKind::Request,
                                    },
                                );
                                resolve_daemon_sender_locked(&guard, &daemon_key)
                            };
                            let Some(daemon_sender) = daemon_sender else {
                                registry
                                    .write()
                                    .await
                                    .pending_requests
                                    .remove(&relay_request_id);
                                log_daemon_sender_missing(
                                    "client_request",
                                    &registry,
                                    peer_addr,
                                    &daemon_key,
                                    &relay_request_id,
                                )
                                .await;
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::ClientResponse {
                                        request_id,
                                        encrypted_response: None,
                                        error: Some(relay_error(
                                            "target_not_connected",
                                            "target daemon is not connected to relay",
                                            true,
                                        )),
                                    },
                                )?;
                                continue;
                            };
                            send_envelope(
                                &daemon_sender,
                                &RelayEnvelope::DaemonRequest {
                                    relay_request_id,
                                    caller_identity: peer_identity(&registry, peer_addr).await,
                                    encrypted_request,
                                },
                            )?;
                        }
                        RelayEnvelope::ClientSubscribe {
                            request_id,
                            subscription_id,
                            target,
                            session_id,
                            attachment_id,
                            client_public_key,
                            subscription_scope,
                            resume_from_event_id,
                        } => {
                            let Some(realm_id) =
                                connected_client_realm_id(&registry, peer_addr).await
                            else {
                                send_close(
                                    &outgoing_tx,
                                    "client must connect before subscribing".to_string(),
                                );
                                break;
                            };
                            relay_log(
                                "info",
                                "client_subscribe_received",
                                json!({
                                    "peer_addr": peer_addr.to_string(),
                                    "realm_id": realm_id,
                                    "request_id": request_id,
                                    "subscription_id": subscription_id,
                                    "target": target_log_value(&target),
                                    "session_id": session_id,
                                    "attachment_id": attachment_id,
                                    "subscription_scope": subscription_scope,
                                    "resume_from_event_id": resume_from_event_id,
                                }),
                            );
                            let Some(daemon_key) =
                                resolve_target_daemon_key(&registry, &realm_id, &target).await
                            else {
                                log_target_not_connected(
                                    "client_subscribe",
                                    &registry,
                                    peer_addr,
                                    &realm_id,
                                    &target,
                                )
                                .await;
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::ClientResponse {
                                        request_id,
                                        encrypted_response: None,
                                        error: Some(relay_error(
                                            "target_not_connected",
                                            "target daemon is not connected to relay",
                                            true,
                                        )),
                                    },
                                )?;
                                continue;
                            };
                            relay_log(
                                "info",
                                "client_subscribe_target_resolved",
                                json!({
                                    "peer_addr": peer_addr.to_string(),
                                    "realm_id": realm_id,
                                    "request_id": request_id,
                                    "subscription_id": subscription_id,
                                    "target": target_log_value(&target),
                                    "daemon_key": daemon_key_log_value(&daemon_key),
                                    "session_id": session_id,
                                    "attachment_id": attachment_id,
                                    "subscription_scope": subscription_scope,
                                }),
                            );
                            let relay_request_id = format!(
                                "relay-request-{}",
                                relay_request_counter.fetch_add(1, Ordering::Relaxed) + 1
                            );
                            let (subscription_conflict, daemon_sender) = {
                                let mut guard = registry.write().await;
                                if subscription_owned_by_other_client(
                                    &guard,
                                    &subscription_id,
                                    peer_addr,
                                ) {
                                    (true, None)
                                } else {
                                    guard.pending_requests.insert(
                                        relay_request_id.clone(),
                                        PendingClientRequest {
                                            client_addr: peer_addr,
                                            client_request_id: request_id.clone(),
                                            daemon_key: daemon_key.clone(),
                                            kind: PendingRequestKind::Subscribe {
                                                subscription_id: subscription_id.clone(),
                                            },
                                        },
                                    );
                                    (false, resolve_daemon_sender_locked(&guard, &daemon_key))
                                }
                            };
                            if subscription_conflict {
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::ClientResponse {
                                        request_id,
                                        encrypted_response: None,
                                        error: Some(relay_error(
                                            "subscription_conflict",
                                            "relay subscription id is already active for another client",
                                            true,
                                        )),
                                    },
                                )?;
                                continue;
                            }
                            let Some(daemon_sender) = daemon_sender else {
                                registry
                                    .write()
                                    .await
                                    .pending_requests
                                    .remove(&relay_request_id);
                                log_daemon_sender_missing(
                                    "client_subscribe",
                                    &registry,
                                    peer_addr,
                                    &daemon_key,
                                    &relay_request_id,
                                )
                                .await;
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::ClientResponse {
                                        request_id,
                                        encrypted_response: None,
                                        error: Some(relay_error(
                                            "target_not_connected",
                                            "target daemon is not connected to relay",
                                            true,
                                        )),
                                    },
                                )?;
                                continue;
                            };
                            relay_log(
                                "info",
                                "daemon_subscribe_forwarded",
                                json!({
                                    "relay_request_id": relay_request_id,
                                    "client_request_id": request_id,
                                    "subscription_id": subscription_id,
                                    "daemon_key": daemon_key_log_value(&daemon_key),
                                    "session_id": session_id,
                                    "attachment_id": attachment_id,
                                    "subscription_scope": subscription_scope,
                                    "resume_from_event_id": resume_from_event_id,
                                }),
                            );
                            send_envelope(
                                &daemon_sender,
                                &RelayEnvelope::DaemonSubscribe {
                                    relay_request_id,
                                    relay_subscription_id: subscription_id,
                                    caller_identity: peer_identity(&registry, peer_addr).await,
                                    session_id,
                                    attachment_id,
                                    client_public_key,
                                    subscription_scope,
                                    resume_from_event_id,
                                },
                            )?;
                        }
                        RelayEnvelope::ClientUnsubscribe {
                            request_id,
                            subscription_id,
                            client_public_key,
                        } => {
                            if connected_client_realm_id(&registry, peer_addr)
                                .await
                                .is_none()
                            {
                                send_close(
                                    &outgoing_tx,
                                    "client must connect before unsubscribing".to_string(),
                                );
                                break;
                            }
                            let relay_request_id = format!(
                                "relay-request-{}",
                                relay_request_counter.fetch_add(1, Ordering::Relaxed) + 1
                            );
                            let active = {
                                let guard = registry.read().await;
                                guard.subscriptions.get(&subscription_id).cloned()
                            };
                            let Some(active) = active.filter(|active| active.client_addr == peer_addr) else {
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::ClientResponse {
                                        request_id,
                                        encrypted_response: None,
                                        error: Some(relay_error(
                                            "subscription_not_found",
                                            "relay subscription is not active",
                                            false,
                                        )),
                                    },
                                )?;
                                continue;
                            };
                            let daemon_key = active.daemon_key.clone();
                            let daemon_sender = {
                                let mut guard = registry.write().await;
                                guard.pending_requests.insert(
                                    relay_request_id.clone(),
                                    PendingClientRequest {
                                        client_addr: peer_addr,
                                        client_request_id: request_id.clone(),
                                        daemon_key: daemon_key.clone(),
                                        kind: PendingRequestKind::Unsubscribe {
                                            subscription_id: subscription_id.clone(),
                                        },
                                    },
                                );
                                resolve_daemon_sender_locked(&guard, &daemon_key)
                            };
                            let Some(daemon_sender) = daemon_sender else {
                                registry
                                    .write()
                                    .await
                                    .pending_requests
                                    .remove(&relay_request_id);
                                log_daemon_sender_missing(
                                    "client_unsubscribe",
                                    &registry,
                                    peer_addr,
                                    &daemon_key,
                                    &relay_request_id,
                                )
                                .await;
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::ClientResponse {
                                        request_id,
                                        encrypted_response: None,
                                        error: Some(relay_error(
                                            "target_not_connected",
                                            "target daemon is not connected to relay",
                                            true,
                                        )),
                                    },
                                )?;
                                continue;
                            };
                            send_envelope(
                                &daemon_sender,
                                &RelayEnvelope::DaemonUnsubscribe {
                                    relay_request_id,
                                    relay_subscription_id: subscription_id,
                                    caller_identity: peer_identity(&registry, peer_addr).await,
                                    client_public_key,
                                },
                            )?;
                        }
                        RelayEnvelope::DaemonResponse {
                            relay_request_id,
                            encrypted_response,
                            error,
                        } => {
                            let Some(current_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before sending responses".to_string(),
                                );
                                break;
                            };
                            let client_target = {
                                let mut guard = registry.write().await;
                                let pending = guard
                                    .pending_requests
                                    .get(&relay_request_id)
                                    .filter(|pending| pending.daemon_key == current_daemon_key)
                                    .cloned();
                                if pending.is_some() {
                                    guard.pending_requests.remove(&relay_request_id);
                                }
                                pending.and_then(|pending| {
                                    if error.is_none() {
                                        match &pending.kind {
                                            PendingRequestKind::Subscribe { subscription_id } => {
                                                guard.subscriptions.insert(
                                                    subscription_id.clone(),
                                                    ActiveSubscription {
                                                        client_addr: pending.client_addr,
                                                        daemon_key: pending.daemon_key.clone(),
                                                    },
                                                );
                                            }
                                            PendingRequestKind::Unsubscribe { subscription_id } => {
                                                guard.subscriptions.remove(subscription_id);
                                            }
                                            PendingRequestKind::Request => {}
                                        }
                                    }
                                    guard.peers.get(&pending.client_addr).map(|peer| {
                                        (peer.sender.clone(), pending.client_request_id)
                                    })
                                })
                            };
                            if let Some((client_sender, client_request_id)) = client_target {
                                send_envelope(
                                    &client_sender,
                                    &RelayEnvelope::ClientResponse {
                                        request_id: client_request_id,
                                        encrypted_response,
                                        error,
                                    },
                                )?;
                            }
                        }
                        RelayEnvelope::DaemonIncomingPeerResponse {
                            relay_request_id,
                            encrypted_response,
                            error,
                        } => {
                            let Some(current_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before sending peer responses"
                                        .to_string(),
                                );
                                break;
                            };
                            let daemon_target = {
                                let mut guard = registry.write().await;
                                let pending = guard
                                    .pending_daemon_peer_requests
                                    .get(&relay_request_id)
                                    .filter(|pending| {
                                        pending.target_daemon_key == current_daemon_key
                                    })
                                    .cloned();
                                if pending.is_some() {
                                    guard.pending_daemon_peer_requests.remove(&relay_request_id);
                                }
                                pending.and_then(|pending| {
                                    resolve_daemon_sender_locked(
                                        &guard,
                                        &pending.requester_daemon_key,
                                    )
                                    .map(|sender| {
                                        (
                                            sender,
                                            pending.requester_request_id,
                                            pending.target_daemon_key.daemon_id,
                                        )
                                    })
                                })
                            };
                            if let Some((daemon_sender, requester_request_id, target_daemon_id)) =
                                daemon_target
                            {
                                send_envelope(
                                    &daemon_sender,
                                    &RelayEnvelope::DaemonPeerResponse {
                                        request_id: requester_request_id,
                                        from_daemon_id: target_daemon_id,
                                        encrypted_response,
                                        error,
                                    },
                                )?;
                            }
                        }
                        RelayEnvelope::DaemonDisplayTunnelRegister { registration } => {
                            let Some(current_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before display tunnel registration"
                                        .to_string(),
                                );
                                break;
                            };
                            let tunnel_id = registration.tunnel_id.clone();
                            let expires_at_ms = registration.expires_at_ms;
                            let error = if tunnel_id.trim().is_empty() {
                                Some(relay_error(
                                    "invalid_display_tunnel",
                                    "display tunnel id must not be empty",
                                    false,
                                ))
                            } else if expires_at_ms <= current_unix_ms() {
                                Some(relay_error(
                                    "display_tunnel_expired",
                                    "display tunnel expiry must be in the future",
                                    false,
                                ))
                            } else {
                                let mut guard = registry.write().await;
                                guard.prune_expired_display_tunnels(current_unix_ms());
                                guard.register_display_tunnel(
                                    current_daemon_key,
                                    tunnel_id.clone(),
                                    expires_at_ms,
                                    registration.capabilities,
                                );
                                None
                            };
                            send_envelope(
                                &outgoing_tx,
                                &RelayEnvelope::DaemonDisplayTunnelRegistered {
                                    tunnel_id,
                                    expires_at_ms,
                                    error,
                                },
                            )?;
                        }
                        RelayEnvelope::DaemonDisplayTunnelRevoke { tunnel_id } => {
                            let Some(current_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before display tunnel revocation"
                                        .to_string(),
                                );
                                break;
                            };
                            let mut guard = registry.write().await;
                            guard.prune_expired_display_tunnels(current_unix_ms());
                            if guard
                                .display_tunnel(&tunnel_id, current_unix_ms())
                                .is_some_and(|tunnel| tunnel.daemon_key == current_daemon_key)
                            {
                                guard.revoke_display_tunnel(&tunnel_id);
                            }
                        }
                        RelayEnvelope::DaemonDisplayTunnelResponseStart { response } => {
                            let Some(current_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before display tunnel responses"
                                        .to_string(),
                                );
                                break;
                            };
                            let sender = {
                                let guard = registry.read().await;
                                guard.display_stream_sender_for_daemon(
                                    &response.stream_id,
                                    &current_daemon_key,
                                )
                            };
                            if let Some(sender) = sender {
                                let _ = sender
                                    .send(DisplayStreamEvent::ResponseStart {
                                        status: response.status,
                                        headers: response.headers,
                                    })
                                    .await;
                            }
                        }
                        RelayEnvelope::DaemonDisplayTunnelChunk { chunk } => {
                            let Some(current_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before display tunnel chunks".to_string(),
                                );
                                break;
                            };
                            let sender = {
                                let guard = registry.read().await;
                                guard.display_stream_sender_for_daemon(
                                    &chunk.stream_id,
                                    &current_daemon_key,
                                )
                            };
                            if let Some(sender) = sender {
                                let _ = sender
                                    .send(DisplayStreamEvent::Chunk {
                                        data: chunk.data,
                                        message_kind: chunk.message_kind,
                                    })
                                    .await;
                            }
                        }
                        RelayEnvelope::DaemonDisplayTunnelClose { stream_id, error } => {
                            let Some(current_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before display tunnel close".to_string(),
                                );
                                break;
                            };
                            let sender = {
                                let mut guard = registry.write().await;
                                let sender = guard
                                    .display_stream_sender_for_daemon(
                                        &stream_id,
                                        &current_daemon_key,
                                    );
                                guard.remove_pending_display_stream(&stream_id);
                                sender
                            };
                            if let Some(sender) = sender {
                                let _ = sender.send(DisplayStreamEvent::Close { error }).await;
                            }
                        }
                        RelayEnvelope::DaemonEvent {
                            subscription_id,
                            event_id,
                            encrypted_event,
                        } => {
                            let Some(current_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before sending events".to_string(),
                                );
                                break;
                            };
                            let client_sender = {
                                let guard = registry.read().await;
                                guard
                                    .subscriptions
                                    .get(&subscription_id)
                                    .filter(|active| active.daemon_key == current_daemon_key)
                                    .and_then(|active| guard.peers.get(&active.client_addr))
                                    .map(|peer| peer.sender.clone())
                            };
                            if let Some(client_sender) = client_sender {
                                send_envelope(
                                    &client_sender,
                                    &RelayEnvelope::ClientEvent {
                                        subscription_id,
                                        event_id,
                                        encrypted_event,
                                    },
                                )?;
                            }
                        }
                        RelayEnvelope::Close { .. } => {
                            let _ = outgoing_tx.try_send(Message::Close(None));
                            break;
                        }
                        RelayEnvelope::ClientConnected { .. }
                        | RelayEnvelope::ClientMetadataResponse { .. }
                        | RelayEnvelope::DaemonPeerResponse { .. }
                        | RelayEnvelope::DaemonIncomingPeerRequest { .. }
                        | RelayEnvelope::DaemonIncomingPeerEvent { .. }
                        | RelayEnvelope::DaemonDisplayTunnelRegistered { .. }
                        | RelayEnvelope::DaemonDisplayTunnelOpen { .. }
                        | RelayEnvelope::DaemonDisplayTunnelClientChunk { .. }
                        | RelayEnvelope::DaemonDisplayTunnelClientClose { .. }
                        | RelayEnvelope::ClientResponse { .. }
                        | RelayEnvelope::DaemonRequest { .. }
                        | RelayEnvelope::DaemonSubscribe { .. }
                        | RelayEnvelope::DaemonUnsubscribe { .. }
                        | RelayEnvelope::ClientEvent { .. } => {}
                    }
                }
                Message::Ping(payload) => {
                    if outgoing_tx.try_send(Message::Pong(payload)).is_err() {
                        break;
                    }
                }
                Message::Pong(_) => {}
                Message::Close(_) => break,
                _ => {}
            }
        }
        Ok(())
    }
    .await;

    let (
        disconnect_errors,
        disconnect_peer_errors,
        disconnect_subscription_senders,
        disconnect_display_stream_senders,
        dropped_client_pending_requests,
    ) = remove_peer(&registry, peer_addr, registered_daemon_key.as_ref()).await;
    if connection_result.is_err()
        || registered_daemon_key.is_some()
        || !disconnect_errors.is_empty()
        || !disconnect_peer_errors.is_empty()
        || !disconnect_subscription_senders.is_empty()
        || !disconnect_display_stream_senders.is_empty()
        || dropped_client_pending_requests > 0
    {
        relay_log(
            if connection_result.is_err() {
                "warn"
            } else {
                "info"
            },
            "relay_peer_removed",
            json!({
                "peer_addr": peer_addr.to_string(),
                "daemon_key": registered_daemon_key.as_ref().map(daemon_key_log_value),
                "client_request_errors": disconnect_errors.len(),
                "daemon_peer_request_errors": disconnect_peer_errors.len(),
                "subscription_closes": disconnect_subscription_senders.len(),
                "display_stream_closes": disconnect_display_stream_senders.len(),
                "client_pending_request_drops": dropped_client_pending_requests,
                "error": connection_result.as_ref().err().map(|error| error.to_string()),
            }),
        );
    }
    for (sender, request_id) in disconnect_errors {
        let _ = send_envelope(
            &sender,
            &RelayEnvelope::ClientResponse {
                request_id,
                encrypted_response: None,
                error: Some(relay_error(
                    "target_disconnected",
                    "target daemon disconnected from relay",
                    true,
                )),
            },
        );
    }
    for (sender, request_id, target_daemon_id) in disconnect_peer_errors {
        let _ = send_envelope(
            &sender,
            &RelayEnvelope::DaemonPeerResponse {
                request_id,
                from_daemon_id: target_daemon_id,
                encrypted_response: None,
                error: Some(relay_error(
                    "target_disconnected",
                    "target daemon disconnected from relay",
                    true,
                )),
            },
        );
    }
    for sender in disconnect_subscription_senders {
        send_close(&sender, "target daemon disconnected from relay".to_string());
        let _ = sender.try_send(Message::Close(None));
    }
    for sender in disconnect_display_stream_senders {
        let _ = sender
            .send(DisplayStreamEvent::Close {
                error: Some(relay_error(
                    "target_disconnected",
                    "target daemon disconnected from relay",
                    true,
                )),
            })
            .await;
    }
    drop(outgoing_tx);
    writer_task.abort();
    let _ = writer_task.await;
    connection_result
}

async fn resolve_target_daemon_key(
    registry: &Arc<RwLock<RelayRegistry>>,
    realm_id: &str,
    target: &ClientTarget,
) -> Option<DaemonKey> {
    let guard = registry.read().await;
    if let Some(daemon_id) = target.daemon_id.as_ref() {
        let key = DaemonKey::new(realm_id.to_string(), daemon_id.clone());
        return guard.daemons.get(&key).map(|_| key);
    }
    let alias = target.daemon_alias.as_ref()?;
    guard
        .daemons
        .iter()
        .find(|(key, registration)| {
            key.realm_id == realm_id && registration.daemon_alias.as_ref() == Some(alias)
        })
        .map(|(key, _)| key.clone())
}

async fn log_target_not_connected(
    operation: &str,
    registry: &Arc<RwLock<RelayRegistry>>,
    peer_addr: SocketAddr,
    realm_id: &str,
    target: &ClientTarget,
) {
    let guard = registry.read().await;
    relay_log(
        "warn",
        "relay_target_not_connected",
        json!({
            "operation": operation,
            "peer_addr": peer_addr.to_string(),
            "realm_id": realm_id,
            "target": target_log_value(target),
            "peer_count": guard.peer_count(),
            "daemon_count": guard.daemon_count(),
            "pending_request_count": guard.pending_request_count(),
            "subscription_count": guard.subscription_count(),
        }),
    );
}

async fn log_daemon_sender_missing(
    operation: &str,
    registry: &Arc<RwLock<RelayRegistry>>,
    peer_addr: SocketAddr,
    daemon_key: &DaemonKey,
    relay_request_id: &str,
) {
    let guard = registry.read().await;
    relay_log(
        "warn",
        "relay_daemon_sender_missing",
        json!({
            "operation": operation,
            "peer_addr": peer_addr.to_string(),
            "daemon_key": daemon_key_log_value(daemon_key),
            "relay_request_id": relay_request_id,
            "daemon_registered": guard.daemons.contains_key(daemon_key),
            "peer_count": guard.peer_count(),
            "daemon_count": guard.daemon_count(),
            "pending_request_count": guard.pending_request_count(),
            "subscription_count": guard.subscription_count(),
        }),
    );
}

async fn connected_client_realm_id(
    registry: &Arc<RwLock<RelayRegistry>>,
    peer_addr: SocketAddr,
) -> Option<String> {
    registry
        .read()
        .await
        .peers
        .get(&peer_addr)
        .filter(|peer| peer.role == RelayConnectionRole::Client)
        .and_then(|peer| peer.realm_id.clone())
}

fn subscription_owned_by_other_client(
    registry: &RelayRegistry,
    subscription_id: &str,
    peer_addr: SocketAddr,
) -> bool {
    registry
        .subscriptions
        .get(subscription_id)
        .is_some_and(|active| active.client_addr != peer_addr)
        || registry.pending_requests.values().any(|pending| {
            pending.client_addr != peer_addr
                && matches!(
                    &pending.kind,
                    PendingRequestKind::Subscribe {
                        subscription_id: pending_subscription_id,
                    } if pending_subscription_id == subscription_id
                )
        })
}

async fn peer_identity(
    registry: &Arc<RwLock<RelayRegistry>>,
    peer_addr: SocketAddr,
) -> Option<RelayCallerIdentity> {
    registry
        .read()
        .await
        .peers
        .get(&peer_addr)
        .and_then(|peer| peer.identity.clone())
}

fn resolve_daemon_sender_locked(
    registry: &RelayRegistry,
    daemon_key: &DaemonKey,
) -> Option<RelaySender> {
    let registration = registry.daemons.get(daemon_key)?;
    let peer_addr = registry.daemon_peers.get(daemon_key)?;
    let peer = registry.peers.get(peer_addr)?;
    if peer.role == RelayConnectionRole::Daemon
        && peer.realm_id.as_deref() == Some(daemon_key.realm_id.as_str())
        && peer
            .daemon_registration
            .as_ref()
            .map(|candidate| candidate.daemon_id.as_str())
            == Some(registration.daemon_id.as_str())
    {
        Some(peer.sender.clone())
    } else {
        None
    }
}

async fn remove_peer(
    registry: &Arc<RwLock<RelayRegistry>>,
    peer_addr: SocketAddr,
    daemon_key: Option<&DaemonKey>,
) -> (
    Vec<(RelaySender, String)>,
    Vec<(RelaySender, String, String)>,
    Vec<RelaySender>,
    Vec<DisplayStreamSender>,
    usize,
) {
    let mut guard = registry.write().await;
    let removed_peer = guard.peers.remove(&peer_addr);
    let client_subscription_ids = guard
        .subscriptions
        .iter()
        .filter(|(_, active)| active.client_addr == peer_addr)
        .map(|(subscription_id, _)| subscription_id.clone())
        .collect::<Vec<_>>();
    for subscription_id in client_subscription_ids {
        guard.subscriptions.remove(&subscription_id);
    }
    let dropped_client_pending_requests = if removed_peer
        .as_ref()
        .is_some_and(|peer| peer.role == RelayConnectionRole::Client)
    {
        let before = guard.pending_requests.len();
        guard
            .pending_requests
            .retain(|_, pending| pending.client_addr != peer_addr);
        before.saturating_sub(guard.pending_requests.len())
    } else {
        0
    };
    if let Some(daemon_key) = daemon_key {
        let removed_current_daemon = removed_peer.as_ref().is_some_and(|peer| {
            peer.role == RelayConnectionRole::Daemon
                && peer.realm_id.as_deref() == Some(daemon_key.realm_id.as_str())
                && peer
                    .daemon_registration
                    .as_ref()
                    .map(|registration| registration.daemon_id.as_str())
                    == Some(daemon_key.daemon_id.as_str())
        });
        if !removed_current_daemon {
            return (
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                dropped_client_pending_requests,
            );
        }
        guard.daemons.remove(daemon_key);
        guard.daemon_peers.remove(daemon_key);
        guard.remove_display_tunnels_for_daemon(daemon_key);
        let display_stream_senders = guard.remove_display_streams_for_daemon(daemon_key);
        let daemon_subscriptions = guard
            .subscriptions
            .iter()
            .filter(|(_, active)| &active.daemon_key == daemon_key)
            .map(|(subscription_id, active)| (subscription_id.clone(), active.client_addr))
            .collect::<Vec<_>>();
        let mut subscription_client_addrs = daemon_subscriptions
            .iter()
            .map(|(_, client_addr)| *client_addr)
            .collect::<Vec<_>>();
        subscription_client_addrs.sort();
        subscription_client_addrs.dedup();
        for (subscription_id, _) in daemon_subscriptions {
            guard.subscriptions.remove(&subscription_id);
        }
        let subscription_client_senders = subscription_client_addrs
            .into_iter()
            .filter_map(|client_addr| {
                guard
                    .peers
                    .get(&client_addr)
                    .map(|peer| peer.sender.clone())
            })
            .collect::<Vec<_>>();
        let doomed_request_ids = guard
            .pending_requests
            .iter()
            .filter(|(_, pending)| &pending.daemon_key == daemon_key)
            .map(|(relay_request_id, _)| relay_request_id.clone())
            .collect::<Vec<_>>();
        let mut client_errors = Vec::new();
        for relay_request_id in doomed_request_ids {
            if let Some(pending) = guard.pending_requests.remove(&relay_request_id) {
                if let Some(peer) = guard.peers.get(&pending.client_addr) {
                    client_errors.push((peer.sender.clone(), pending.client_request_id));
                }
            }
        }
        let doomed_peer_request_ids = guard
            .pending_daemon_peer_requests
            .iter()
            .filter(|(_, pending)| {
                &pending.target_daemon_key == daemon_key
                    || &pending.requester_daemon_key == daemon_key
            })
            .map(|(relay_request_id, _)| relay_request_id.clone())
            .collect::<Vec<_>>();
        let mut daemon_errors = Vec::new();
        for relay_request_id in doomed_peer_request_ids {
            if let Some(pending) = guard.pending_daemon_peer_requests.remove(&relay_request_id) {
                if &pending.requester_daemon_key == daemon_key {
                    continue;
                }
                if let Some(sender) =
                    resolve_daemon_sender_locked(&guard, &pending.requester_daemon_key)
                {
                    daemon_errors.push((
                        sender,
                        pending.requester_request_id,
                        pending.target_daemon_key.daemon_id,
                    ));
                }
            }
        }
        return (
            client_errors,
            daemon_errors,
            subscription_client_senders,
            display_stream_senders,
            dropped_client_pending_requests,
        );
    }
    (
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        dropped_client_pending_requests,
    )
}

fn verify_relay_token(
    verifier: &RelayAuthVerifier,
    token: &str,
    action: RelayAction,
    target: Option<&str>,
) -> Result<VerifiedRelayIdentity, std::io::Error> {
    verifier
        .verify(RelayAuthRequest {
            token,
            action,
            target,
        })
        .map_err(relay_auth_error)
}

fn relay_auth_error(error: RelayAuthError) -> std::io::Error {
    let kind = match error {
        RelayAuthError::InvalidToken
        | RelayAuthError::ActionNotAllowed
        | RelayAuthError::TargetNotAllowed
        | RelayAuthError::TokenExpired
        | RelayAuthError::ScopedTokensUnavailable => std::io::ErrorKind::PermissionDenied,
    };
    std::io::Error::new(kind, error.to_string())
}

fn send_envelope(sender: &RelaySender, envelope: &RelayEnvelope) -> Result<(), std::io::Error> {
    let payload = serde_json::to_string(envelope)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    sender
        .try_send(Message::Text(payload.into()))
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::BrokenPipe, error.to_string()))
}

fn send_close(sender: &RelaySender, reason: String) {
    let _ = send_envelope(sender, &RelayEnvelope::Close { reason });
}

fn relay_error(code: &str, message: &str, retryable: bool) -> RelayError {
    RelayError {
        code: code.to_string(),
        message: message.to_string(),
        retryable,
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn relay_log(level: &str, event: &str, fields: Value) {
    eprintln!(
        "{}",
        json!({
            "component": "arroba-relay",
            "level": level,
            "event": event,
            "fields": fields,
        })
    );
}

fn target_log_value(target: &ClientTarget) -> Value {
    json!({
        "daemon_id": target.daemon_id,
        "daemon_alias": target.daemon_alias,
    })
}

fn daemon_key_log_value(daemon_key: &DaemonKey) -> Value {
    json!({
        "realm_id": daemon_key.realm_id,
        "daemon_id": daemon_key.daemon_id,
    })
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use tokio::sync::{mpsc, RwLock};

    use super::*;
    use crate::auth::DEFAULT_RELAY_REALM_ID;
    use crate::protocol::DaemonRegistration;

    fn peer_addr(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    fn daemon_registration(daemon_id: &str) -> DaemonRegistration {
        DaemonRegistration {
            auth_token: "token".to_string(),
            daemon_id: daemon_id.to_string(),
            machine_id: "machine-1".to_string(),
            machine_alias: None,
            os_name: None,
            kernel_started_at_ms: 0,
            daemon_alias: None,
            kernel_alias: None,
            public_key: "public-key".to_string(),
            capabilities: Vec::new(),
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            accepting_remote_leases: false,
            leased_agent_count: 0,
            local_session_count: 0,
        }
    }

    fn daemon_peer(sender: RelaySender, registration: DaemonRegistration) -> PeerHandle {
        PeerHandle {
            sender,
            role: RelayConnectionRole::Daemon,
            realm_id: Some(DEFAULT_RELAY_REALM_ID.to_string()),
            identity: None,
            daemon_registration: Some(registration),
        }
    }

    #[test]
    fn resolve_daemon_sender_uses_daemon_peer_index() {
        let daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-1");
        let registration = daemon_registration("daemon-1");
        let daemon_addr = peer_addr(10_001);
        let (sender, _receiver) = mpsc::channel::<Message>(1);
        let mut registry = RelayRegistry::default();
        registry
            .daemons
            .insert(daemon_key.clone(), registration.clone());
        registry
            .peers
            .insert(daemon_addr, daemon_peer(sender, registration));
        registry
            .daemon_peers
            .insert(daemon_key.clone(), daemon_addr);

        assert!(resolve_daemon_sender_locked(&registry, &daemon_key).is_some());

        registry
            .daemon_peers
            .insert(daemon_key.clone(), peer_addr(10_002));

        assert!(
            resolve_daemon_sender_locked(&registry, &daemon_key).is_none(),
            "stale route index entries must not fall back to scanning all peers"
        );
    }

    #[test]
    fn performance_drill_relay_fanout_resolves_daemon_routes_from_index() {
        let mut registry = RelayRegistry::default();
        for index in 0..2_000 {
            let daemon_id = format!("daemon-{index}");
            let daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, daemon_id.clone());
            let registration = daemon_registration(&daemon_id);
            let addr = peer_addr(20_000 + index as u16);
            let (sender, _receiver) = mpsc::channel::<Message>(1);
            registry
                .daemons
                .insert(daemon_key.clone(), registration.clone());
            registry
                .peers
                .insert(addr, daemon_peer(sender, registration));
            registry.daemon_peers.insert(daemon_key, addr);
        }

        let target_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-1999");
        assert!(resolve_daemon_sender_locked(&registry, &target_key).is_some());
        assert_eq!(registry.daemon_peers.len(), registry.daemons.len());

        registry
            .daemon_peers
            .insert(target_key.clone(), peer_addr(65_000));
        assert!(
            resolve_daemon_sender_locked(&registry, &target_key).is_none(),
            "indexed routing must not scan all relay peers when an index entry is stale"
        );
    }

    #[tokio::test]
    async fn remove_daemon_peer_clears_daemon_route_index() {
        let daemon_key = DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-1");
        let registration = daemon_registration("daemon-1");
        let peer_addr = peer_addr(10_003);
        let (sender, _receiver) = mpsc::channel::<Message>(1);
        let mut registry = RelayRegistry::default();
        registry
            .daemons
            .insert(daemon_key.clone(), registration.clone());
        registry
            .peers
            .insert(peer_addr, daemon_peer(sender, registration));
        registry.daemon_peers.insert(daemon_key.clone(), peer_addr);
        let registry = Arc::new(RwLock::new(registry));

        let _ = remove_peer(&registry, peer_addr, Some(&daemon_key)).await;

        let guard = registry.read().await;
        assert!(!guard.daemons.contains_key(&daemon_key));
        assert!(!guard.daemon_peers.contains_key(&daemon_key));
        assert!(!guard.peers.contains_key(&peer_addr));
    }
}
