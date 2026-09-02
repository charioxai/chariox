use std::future::pending;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use crate::auth::{RelayAction, RelayAuthVerifier};
use crate::protocol::{RelayConnectionRole, RelayEnvelope, RelayError, RelayMetadataQuery};
use crate::registry::{
    ActiveEventRoute, ActiveSubscription, DaemonKey, DisplayStreamEvent, PeerHandle,
    PendingDaemonPeerRequest, PendingRequestKind, RelayRegistry, RelaySender,
};

mod support;
#[cfg(test)]
mod tests;

use support::*;

const DEFAULT_RELAY_OUTGOING_QUEUE_CAPACITY: usize = 1024;
const RELAY_WEBSOCKET_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const RELAY_FIRST_MESSAGE_TIMEOUT: Duration = Duration::from_secs(10);
const RELAY_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const RELAY_CONNECTION_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const RELAY_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const RELAY_PONG_TIMEOUT: Duration = Duration::from_secs(15);
const RELAY_WEBSOCKET_CLOSE_TIMEOUT: Duration = Duration::from_millis(250);

async fn try_forward_display_stream_event(
    registry: &Arc<RwLock<RelayRegistry>>,
    daemon_sender: &RelaySender,
    daemon_key: &DaemonKey,
    stream_id: &str,
    event: DisplayStreamEvent,
) {
    let sender = registry
        .read()
        .await
        .display_stream_sender_for_daemon(stream_id, daemon_key);
    let Some(sender) = sender else {
        return;
    };
    if sender.try_send(event).is_ok() {
        return;
    }
    registry
        .write()
        .await
        .remove_pending_display_stream(stream_id);
    let _ = send_envelope(
        daemon_sender,
        &RelayEnvelope::DaemonDisplayTunnelClientClose {
            stream_id: stream_id.to_string(),
            error: Some(relay_error(
                "display_stream_backpressure",
                "display viewer stopped accepting encrypted stream packets",
                true,
            )),
        },
    );
}

async fn close_display_stream_from_daemon(
    registry: &Arc<RwLock<RelayRegistry>>,
    daemon_key: &DaemonKey,
    stream_id: &str,
    error: Option<RelayError>,
) {
    let sender = {
        let mut guard = registry.write().await;
        let sender = guard.display_stream_sender_for_daemon(stream_id, daemon_key);
        guard.remove_pending_display_stream(stream_id);
        sender
    };
    let Some(sender) = sender else {
        return;
    };
    // A full queue cannot accept the terminal event. Removing the registry's
    // sender and dropping this last clone closes the channel after its queued
    // frames drain, so the viewer still observes termination without blocking
    // the daemon read lane.
    let _ = sender.try_send(DisplayStreamEvent::Close { error });
}

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
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Message>(relay_outgoing_queue_capacity());
    let routes = registry.read().await.route_index();
    let mut writer_task = Some(tokio::spawn(async move {
        while let Some(message) = outgoing_rx.recv().await {
            if writer.send(message).await.is_err() {
                let _ = writer.flush().await;
                break;
            }
        }
    }));
    let mut registered_daemon_key: Option<DaemonKey> = None;
    let mut auth_expiry_deadline: Option<Instant> = None;
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
                _ = async {
                    match auth_expiry_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                        None => pending().await,
                    }
                } => {
                    relay_log(
                        "warn",
                        "relay_connection_token_expired",
                        json!({
                            "peer_addr": peer_addr.to_string(),
                        }),
                    );
                    send_close(&outgoing_tx, "relay token expired".to_string());
                    break;
                }
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
                _ = async {
                    match writer_task.as_mut() {
                        Some(task) => {
                            let _ = task.await;
                        }
                        None => pending().await,
                    }
                }, if writer_task.is_some() => {
                    writer_task = None;
                    break;
                }
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
                            if let Err(reason) = validate_daemon_registration_identity(
                                &identity,
                                &registration,
                            ) {
                                send_close(&outgoing_tx, reason.to_string());
                                break;
                            }
                            auth_expiry_deadline = relay_auth_expiry_deadline(
                                &auth_verifier,
                                identity.expires_at_ms,
                            );
                            let daemon_key = DaemonKey::new(
                                identity.realm_id.clone(),
                                registration.daemon_id.clone(),
                            );
                            let allowed_actions = identity.allowed_actions.clone();
                            let allowed_targets = identity.allowed_targets.clone();
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
                                    allowed_actions,
                                    allowed_targets,
                                    daemon_registration: Some(registration.clone()),
                                    client_daemon_key: None,
                                },
                            );
                            guard.daemons.insert(daemon_key.clone(), registration);
                            guard.daemon_peers.insert(daemon_key.clone(), peer_addr);
                            routes.set_daemon_sender(daemon_key, outgoing_tx.clone());
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
                                    None,
                                )?;
                                if let Err(reason) = validate_daemon_registration_identity(
                                    &identity,
                                    &registration,
                                ) {
                                    send_close(&outgoing_tx, reason.to_string());
                                    break;
                                }
                                auth_expiry_deadline = relay_auth_expiry_deadline(
                                    &auth_verifier,
                                    identity.expires_at_ms,
                                );
                                if identity.realm_id != current_daemon_key.realm_id {
                                    break;
                                }
                                if registration.daemon_id != daemon_id {
                                    break;
                                }
                                let allowed_actions = identity.allowed_actions.clone();
                                let allowed_targets = identity.allowed_targets.clone();
                                let mut guard = registry.write().await;
                                if let Some(peer) = guard.peers.get_mut(&peer_addr) {
                                    peer.realm_id = Some(identity.realm_id.clone());
                                    peer.identity = Some(identity.into());
                                    peer.allowed_actions = allowed_actions;
                                    peer.allowed_targets = allowed_targets;
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
                            auth_expiry_deadline = relay_auth_expiry_deadline(
                                &auth_verifier,
                                identity.expires_at_ms,
                            );
                            let allowed_actions = identity.allowed_actions.clone();
                            let allowed_targets = identity.allowed_targets.clone();
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
                            let existing_client_key = {
                                let guard = registry.read().await;
                                guard
                                    .peers
                                    .get(&peer_addr)
                                    .filter(|peer| peer.role == RelayConnectionRole::Client)
                                    .and_then(|peer| peer.client_daemon_key.clone())
                            };
                            if existing_client_key
                                .as_ref()
                                .is_some_and(|existing| existing != &daemon_key)
                            {
                                send_close(
                                    &outgoing_tx,
                                    "client connection already bound to target".to_string(),
                                );
                                break;
                            }
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
                                    allowed_actions,
                                    allowed_targets,
                                    daemon_registration: None,
                                    client_daemon_key: Some(daemon_key.clone()),
                                },
                            );
                            routes.set_client_sender(peer_addr, outgoing_tx.clone());
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
                            if !peer_allows_action(&registry, peer_addr, RelayAction::PeerRequest)
                                .await
                            {
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::DaemonPeerResponse {
                                        request_id,
                                        from_daemon_id: String::new(),
                                        encrypted_response: None,
                                        error: Some(relay_error(
                                            "action_not_allowed",
                                            "daemon token does not allow peer requests",
                                            false,
                                        )),
                                    },
                                )?;
                                continue;
                            }
                            if let Some(error) =
                                invalid_runtime_identifier("request_id", &request_id)
                            {
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::DaemonPeerResponse {
                                        request_id,
                                        from_daemon_id: String::new(),
                                        encrypted_response: None,
                                        error: Some(error),
                                    },
                                )?;
                                continue;
                            }
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
                            if !peer_allows_target(
                                &registry,
                                peer_addr,
                                &target,
                                &target_daemon_key,
                            )
                            .await
                            {
                                send_envelope(
                                    &outgoing_tx,
                                    &RelayEnvelope::DaemonPeerResponse {
                                        request_id,
                                        from_daemon_id: target_daemon_key.daemon_id,
                                        encrypted_response: None,
                                        error: Some(relay_error(
                                            "target_not_allowed",
                                            "daemon token does not allow the requested target",
                                            false,
                                        )),
                                    },
                                )?;
                                continue;
                            }
                            let relay_request_id = format!(
                                "relay-peer-request-{}",
                                relay_request_counter.fetch_add(1, Ordering::Relaxed) + 1
                            );
                            routes.insert_pending_daemon(
                                relay_request_id.clone(),
                                PendingDaemonPeerRequest {
                                    requester_daemon_key: requester_daemon_key.clone(),
                                    requester_request_id: request_id.clone(),
                                    target_daemon_key: target_daemon_key.clone(),
                                },
                            );
                            let daemon_sender = routes.daemon_sender(&target_daemon_key);
                            let Some(daemon_sender) = daemon_sender else {
                                routes.remove_pending_daemon(&relay_request_id);
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
                            if send_envelope(
                                &daemon_sender,
                                &RelayEnvelope::DaemonIncomingPeerRequest {
                                    relay_request_id: relay_request_id.clone(),
                                    from_daemon_id: requester_daemon_key.daemon_id,
                                    caller_identity: peer_identity(&registry, peer_addr).await,
                                    encrypted_request,
                                },
                            )
                            .is_err()
                            {
                                reject_peer_pending_on_target_backpressure(
                                    &registry,
                                    &outgoing_tx,
                                    &relay_request_id,
                                    request_id,
                                    target_daemon_key.daemon_id,
                                )
                                .await?;
                            }
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
                            if !peer_allows_action(&registry, peer_addr, RelayAction::PeerEvent)
                                .await
                            {
                                continue;
                            }
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
                            if !peer_allows_target(
                                &registry,
                                peer_addr,
                                &target,
                                &target_daemon_key,
                            )
                            .await
                            {
                                continue;
                            }
                            let daemon_sender = routes.daemon_sender(&target_daemon_key);
                            if let Some(daemon_sender) = daemon_sender {
                                if send_envelope(
                                    &daemon_sender,
                                    &RelayEnvelope::DaemonIncomingPeerEvent {
                                        from_daemon_id: requester_daemon_key.daemon_id,
                                        caller_identity: peer_identity(&registry, peer_addr).await,
                                        encrypted_event,
                                    },
                                )
                                .is_err()
                                {
                                    log_daemon_sender_backpressure(
                                        "daemon_peer_event",
                                        peer_addr,
                                        &target_daemon_key,
                                    );
                                }
                            }
                        }
                        envelope @ RelayEnvelope::ClientRequest { .. }
                        | envelope @ RelayEnvelope::ClientSubscribe { .. }
                        | envelope @ RelayEnvelope::ClientUnsubscribe { .. } => {
                            if handle_client_packet_route_envelope(
                                envelope,
                                &registry,
                                &routes,
                                peer_addr,
                                &outgoing_tx,
                                &relay_request_counter,
                            )
                            .await?
                            .should_break()
                            {
                                break;
                            }
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
                            let pending = routes.take_pending_client_if(
                                &relay_request_id,
                                |pending| pending.daemon_key == current_daemon_key,
                            );
                            let mut orphaned_subscription = None;
                            let client_target = if let Some(pending) = pending {
                                    if error.is_none() {
                                        match &pending.kind {
                                            PendingRequestKind::Subscribe {
                                                subscription_id,
                                                client_public_key,
                                            } => {
                                                let mut guard = registry.write().await;
                                                if guard.peers.contains_key(&pending.client_addr) {
                                                    guard.subscriptions.insert(
                                                        subscription_id.clone(),
                                                        ActiveSubscription {
                                                            client_addr: pending.client_addr,
                                                            daemon_key: pending.daemon_key.clone(),
                                                            client_public_key: client_public_key.clone(),
                                                        },
                                                    );
                                                    if let Some(client_sender) =
                                                        routes.client_sender(&pending.client_addr)
                                                    {
                                                        routes.set_subscription(
                                                            subscription_id.clone(),
                                                            ActiveEventRoute {
                                                                daemon_key: pending.daemon_key.clone(),
                                                                client_sender,
                                                            },
                                                        );
                                                    }
                                                } else {
                                                    orphaned_subscription = Some((
                                                        subscription_id.clone(),
                                                        client_public_key.clone(),
                                                    ));
                                                }
                                            }
                                            PendingRequestKind::Unsubscribe { subscription_id } => {
                                                let mut guard = registry.write().await;
                                                guard.subscriptions.remove(subscription_id);
                                                routes.remove_subscription(subscription_id);
                                            }
                                            PendingRequestKind::Request => {}
                                        }
                                    }
                                    routes
                                        .client_sender(&pending.client_addr)
                                        .map(|sender| (sender, pending.client_request_id))
                                } else {
                                    None
                            };
                            if let Some((subscription_id, client_public_key)) = orphaned_subscription {
                                let _ = send_daemon_subscription_cleanup(
                                    &outgoing_tx,
                                    &relay_request_counter,
                                    subscription_id,
                                    client_public_key,
                                );
                            }
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
                            let daemon_target = routes
                                .take_pending_daemon_if(&relay_request_id, |pending| {
                                    pending.target_daemon_key == current_daemon_key
                                })
                                .and_then(|pending| {
                                    routes
                                    .daemon_sender(&pending.requester_daemon_key)
                                    .map(|sender| {
                                        (
                                            sender,
                                            pending.requester_request_id,
                                            pending.target_daemon_key.daemon_id,
                                        )
                                    })
                                });
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
                            try_forward_display_stream_event(
                                &registry,
                                &outgoing_tx,
                                &current_daemon_key,
                                &response.stream_id,
                                DisplayStreamEvent::ResponseStart {
                                    status: response.status,
                                    headers: response.headers,
                                },
                            )
                            .await;
                        }
                        RelayEnvelope::DaemonDisplayTunnelChunk { chunk } => {
                            let Some(current_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before display tunnel chunks".to_string(),
                                );
                                break;
                            };
                            try_forward_display_stream_event(
                                &registry,
                                &outgoing_tx,
                                &current_daemon_key,
                                &chunk.stream_id,
                                DisplayStreamEvent::Chunk {
                                    data: chunk.data,
                                    message_kind: chunk.message_kind,
                                },
                            )
                            .await;
                        }
                        RelayEnvelope::DaemonDisplayTunnelClose { stream_id, error } => {
                            let Some(current_daemon_key) = registered_daemon_key.clone() else {
                                send_close(
                                    &outgoing_tx,
                                    "daemon must register before display tunnel close".to_string(),
                                );
                                break;
                            };
                            close_display_stream_from_daemon(
                                &registry,
                                &current_daemon_key,
                                &stream_id,
                                error,
                            )
                            .await;
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
                            let client_sender = routes
                                .subscription(&subscription_id)
                                .filter(|route| route.daemon_key == current_daemon_key)
                                .map(|route| route.client_sender);
                            if let Some(client_sender) = client_sender {
                                if send_envelope(
                                    &client_sender,
                                    &RelayEnvelope::ClientEvent {
                                        subscription_id: subscription_id.clone(),
                                        event_id,
                                        encrypted_event,
                                    },
                                )
                                .is_err()
                                {
                                    close_slow_subscription(
                                        &registry,
                                        &routes,
                                        &subscription_id,
                                        &current_daemon_key,
                                        &relay_request_counter,
                                    )
                                    .await;
                                }
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
                Message::Close(frame) => {
                    let _ = outgoing_tx.try_send(Message::Close(frame));
                    break;
                }
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
        daemon_subscription_cleanups,
        dropped_client_pending_requests,
    ) = remove_peer(
        &registry,
        &routes,
        peer_addr,
        registered_daemon_key.as_ref(),
        &relay_request_counter,
    )
    .await;
    if connection_result.is_err()
        || registered_daemon_key.is_some()
        || !disconnect_errors.is_empty()
        || !disconnect_peer_errors.is_empty()
        || !disconnect_subscription_senders.is_empty()
        || !disconnect_display_stream_senders.is_empty()
        || daemon_subscription_cleanups > 0
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
                "daemon_subscription_cleanups": daemon_subscription_cleanups,
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
    if let Some(mut writer_task) = writer_task {
        if tokio::time::timeout(RELAY_WEBSOCKET_CLOSE_TIMEOUT, &mut writer_task)
            .await
            .is_err()
        {
            writer_task.abort();
            let _ = writer_task.await;
        }
    }
    connection_result
}

fn relay_outgoing_queue_capacity() -> usize {
    static CAPACITY: OnceLock<usize> = OnceLock::new();
    *CAPACITY.get_or_init(|| {
        parse_relay_outgoing_queue_capacity(
            std::env::var("CHARIOX_RELAY_OUTGOING_QUEUE_CAPACITY")
                .ok()
                .as_deref(),
        )
    })
}

fn parse_relay_outgoing_queue_capacity(value: Option<&str>) -> usize {
    value
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|capacity| *capacity > 0)
        .unwrap_or(DEFAULT_RELAY_OUTGOING_QUEUE_CAPACITY)
}
