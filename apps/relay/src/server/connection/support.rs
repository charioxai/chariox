use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;

use crate::auth::{
    RelayAction, RelayAuthError, RelayAuthRequest, RelayAuthVerifier, VerifiedRelayIdentity,
};
use crate::protocol::{
    ClientTarget, RelayCallerIdentity, RelayConnectionRole, RelayEnvelope, RelayError,
};
use crate::registry::{
    DaemonKey, PendingClientRequest, PendingRequestKind, RelayRegistry, RelaySender,
};

mod cleanup;

pub(super) use cleanup::remove_peer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectionAction {
    Continue,
    Break,
}

impl ConnectionAction {
    pub(super) fn should_break(self) -> bool {
        matches!(self, Self::Break)
    }
}

pub(super) async fn handle_client_packet_route_envelope(
    envelope: RelayEnvelope,
    registry: &Arc<RwLock<RelayRegistry>>,
    peer_addr: SocketAddr,
    outgoing_tx: &RelaySender,
    relay_request_counter: &AtomicU64,
) -> Result<ConnectionAction, std::io::Error> {
    match envelope {
        RelayEnvelope::ClientRequest {
            request_id,
            target,
            encrypted_request,
        } => {
            let Some((realm_id, connected_daemon_key)) =
                connected_client_binding(&registry, peer_addr).await
            else {
                send_close(
                    &outgoing_tx,
                    "client must connect before sending requests".to_string(),
                );
                return Ok(ConnectionAction::Break);
            };
            if !peer_allows_action(&registry, peer_addr, RelayAction::PacketRoute).await {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(relay_error(
                            "action_not_allowed",
                            "client token does not allow packet routing",
                            false,
                        )),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
            if let Some(error) = invalid_runtime_identifier("request_id", &request_id) {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(error),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
            let Some(daemon_key) = resolve_target_daemon_key(&registry, &realm_id, &target).await
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
                return Ok(ConnectionAction::Continue);
            };
            if daemon_key != connected_daemon_key {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(relay_error(
                            "target_mismatch",
                            "client connection is bound to another relay target",
                            false,
                        )),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
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
                return Ok(ConnectionAction::Continue);
            };
            if send_envelope(
                &daemon_sender,
                &RelayEnvelope::DaemonRequest {
                    relay_request_id: relay_request_id.clone(),
                    caller_identity: peer_identity(&registry, peer_addr).await,
                    encrypted_request,
                },
            )
            .is_err()
            {
                reject_client_pending_on_target_backpressure(
                    &registry,
                    &outgoing_tx,
                    &relay_request_id,
                    request_id,
                )
                .await?;
            }
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
            let Some((realm_id, connected_daemon_key)) =
                connected_client_binding(&registry, peer_addr).await
            else {
                send_close(
                    &outgoing_tx,
                    "client must connect before subscribing".to_string(),
                );
                return Ok(ConnectionAction::Break);
            };
            if !peer_allows_action(&registry, peer_addr, RelayAction::PacketRoute).await {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(relay_error(
                            "action_not_allowed",
                            "client token does not allow packet routing",
                            false,
                        )),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
            if let Some(error) = invalid_runtime_identifier("request_id", &request_id) {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(error),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
            if let Some(error) = invalid_runtime_identifier("subscription_id", &subscription_id) {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(error),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
            if let Some(error) = invalid_runtime_identifier("session_id", &session_id) {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(error),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
            if let Some(error) = invalid_runtime_identifier("attachment_id", &attachment_id) {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(error),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
            if let Some(error) = invalid_runtime_identifier("client_public_key", &client_public_key)
            {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(error),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
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
            let Some(daemon_key) = resolve_target_daemon_key(&registry, &realm_id, &target).await
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
                return Ok(ConnectionAction::Continue);
            };
            if daemon_key != connected_daemon_key {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(relay_error(
                            "target_mismatch",
                            "client connection is bound to another relay target",
                            false,
                        )),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
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
                if subscription_owned_by_other_client(&guard, &subscription_id, peer_addr) {
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
                return Ok(ConnectionAction::Continue);
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
                return Ok(ConnectionAction::Continue);
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
            if send_envelope(
                &daemon_sender,
                &RelayEnvelope::DaemonSubscribe {
                    relay_request_id: relay_request_id.clone(),
                    relay_subscription_id: subscription_id.clone(),
                    caller_identity: peer_identity(&registry, peer_addr).await,
                    session_id,
                    attachment_id,
                    client_public_key,
                    subscription_scope,
                    resume_from_event_id,
                },
            )
            .is_err()
            {
                reject_client_pending_on_target_backpressure(
                    &registry,
                    &outgoing_tx,
                    &relay_request_id,
                    request_id,
                )
                .await?;
            }
        }
        RelayEnvelope::ClientUnsubscribe {
            request_id,
            subscription_id,
            client_public_key,
        } => {
            if connected_client_binding(&registry, peer_addr)
                .await
                .is_none()
            {
                send_close(
                    &outgoing_tx,
                    "client must connect before unsubscribing".to_string(),
                );
                return Ok(ConnectionAction::Break);
            }
            if !peer_allows_action(&registry, peer_addr, RelayAction::PacketRoute).await {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(relay_error(
                            "action_not_allowed",
                            "client token does not allow packet routing",
                            false,
                        )),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
            if let Some(error) = invalid_runtime_identifier("request_id", &request_id) {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(error),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
            if let Some(error) = invalid_runtime_identifier("subscription_id", &subscription_id) {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(error),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
            }
            if let Some(error) = invalid_runtime_identifier("client_public_key", &client_public_key)
            {
                send_envelope(
                    &outgoing_tx,
                    &RelayEnvelope::ClientResponse {
                        request_id,
                        encrypted_response: None,
                        error: Some(error),
                    },
                )?;
                return Ok(ConnectionAction::Continue);
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
                return Ok(ConnectionAction::Continue);
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
                return Ok(ConnectionAction::Continue);
            };
            if send_envelope(
                &daemon_sender,
                &RelayEnvelope::DaemonUnsubscribe {
                    relay_request_id: relay_request_id.clone(),
                    relay_subscription_id: subscription_id,
                    caller_identity: peer_identity(&registry, peer_addr).await,
                    client_public_key,
                },
            )
            .is_err()
            {
                reject_client_pending_on_target_backpressure(
                    &registry,
                    &outgoing_tx,
                    &relay_request_id,
                    request_id,
                )
                .await?;
            }
        }
        _ => unreachable!("non client packet route envelope"),
    }
    Ok(ConnectionAction::Continue)
}

pub(super) async fn resolve_target_daemon_key(
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
    let mut matches = guard
        .daemons
        .iter()
        .filter(|(key, registration)| {
            key.realm_id == realm_id && registration.daemon_alias.as_ref() == Some(alias)
        })
        .map(|(key, _)| key.clone());
    let daemon_key = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(daemon_key)
    }
}

pub(super) async fn log_target_not_connected(
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

pub(super) async fn log_daemon_sender_missing(
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

pub(super) fn log_daemon_sender_backpressure(
    operation: &str,
    peer_addr: SocketAddr,
    daemon_key: &DaemonKey,
) {
    relay_log(
        "warn",
        "relay_daemon_sender_backpressure",
        json!({
            "operation": operation,
            "peer_addr": peer_addr.to_string(),
            "daemon_key": daemon_key_log_value(daemon_key),
        }),
    );
}

pub(super) async fn connected_client_binding(
    registry: &Arc<RwLock<RelayRegistry>>,
    peer_addr: SocketAddr,
) -> Option<(String, DaemonKey)> {
    registry
        .read()
        .await
        .peers
        .get(&peer_addr)
        .filter(|peer| peer.role == RelayConnectionRole::Client)
        .and_then(|peer| peer.realm_id.clone().zip(peer.client_daemon_key.clone()))
}

pub(super) async fn close_slow_subscription(
    registry: &Arc<RwLock<RelayRegistry>>,
    subscription_id: &str,
    daemon_key: &DaemonKey,
) {
    let sender = {
        let mut guard = registry.write().await;
        let active = guard
            .subscriptions
            .get(subscription_id)
            .cloned()
            .filter(|active| active.daemon_key == *daemon_key);
        if let Some(active) = active {
            guard.subscriptions.remove(subscription_id);
            guard
                .peers
                .get(&active.client_addr)
                .map(|peer| peer.sender.clone())
        } else {
            None
        }
    };
    if let Some(sender) = sender {
        send_close(&sender, "relay event consumer is too slow".to_string());
        let _ = sender.try_send(Message::Close(None));
        registry.write().await.record_slow_subscription_close();
    }
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

pub(super) async fn peer_identity(
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

pub(super) async fn peer_allows_action(
    registry: &Arc<RwLock<RelayRegistry>>,
    peer_addr: SocketAddr,
    action: RelayAction,
) -> bool {
    let guard = registry.read().await;
    let Some(peer) = guard.peers.get(&peer_addr) else {
        return false;
    };
    let scoped_token = peer
        .identity
        .as_ref()
        .and_then(|identity| identity.token_id.as_ref())
        .is_some();
    !scoped_token || peer.allowed_actions.contains(&action)
}

pub(super) async fn reject_client_pending_on_target_backpressure(
    registry: &Arc<RwLock<RelayRegistry>>,
    client_sender: &RelaySender,
    relay_request_id: &str,
    client_request_id: String,
) -> Result<(), std::io::Error> {
    {
        let mut guard = registry.write().await;
        guard.pending_requests.remove(relay_request_id);
        guard.record_target_queue_full();
    }
    send_envelope(
        client_sender,
        &RelayEnvelope::ClientResponse {
            request_id: client_request_id,
            encrypted_response: None,
            error: Some(target_backpressure_error()),
        },
    )
}

pub(super) async fn reject_peer_pending_on_target_backpressure(
    registry: &Arc<RwLock<RelayRegistry>>,
    requester_sender: &RelaySender,
    relay_request_id: &str,
    requester_request_id: String,
    target_daemon_id: String,
) -> Result<(), std::io::Error> {
    {
        let mut guard = registry.write().await;
        guard.pending_daemon_peer_requests.remove(relay_request_id);
        guard.record_target_queue_full();
    }
    send_envelope(
        requester_sender,
        &RelayEnvelope::DaemonPeerResponse {
            request_id: requester_request_id,
            from_daemon_id: target_daemon_id,
            encrypted_response: None,
            error: Some(target_backpressure_error()),
        },
    )
}

pub(super) fn resolve_daemon_sender_locked(
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

pub(super) fn verify_relay_token(
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

pub(super) fn relay_auth_error(error: RelayAuthError) -> std::io::Error {
    let kind = match error {
        RelayAuthError::InvalidToken
        | RelayAuthError::ActionNotAllowed
        | RelayAuthError::TargetNotAllowed
        | RelayAuthError::TokenExpired
        | RelayAuthError::TokenRevoked
        | RelayAuthError::ScopedTokensUnavailable => std::io::ErrorKind::PermissionDenied,
    };
    std::io::Error::new(kind, error.to_string())
}

pub(super) fn send_envelope(
    sender: &RelaySender,
    envelope: &RelayEnvelope,
) -> Result<(), std::io::Error> {
    let payload = serde_json::to_string(envelope)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    sender
        .try_send(Message::Text(payload.into()))
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::BrokenPipe, error.to_string()))
}

pub(super) fn send_close(sender: &RelaySender, reason: String) {
    let _ = send_envelope(sender, &RelayEnvelope::Close { reason });
}

pub(super) fn relay_error(code: &str, message: &str, retryable: bool) -> RelayError {
    RelayError {
        code: code.to_string(),
        message: message.to_string(),
        retryable,
    }
}

pub(super) fn invalid_runtime_identifier(field: &str, value: &str) -> Option<RelayError> {
    if value.trim().is_empty() {
        Some(relay_error(
            "invalid_runtime_identifier",
            &format!("{field} must not be empty"),
            false,
        ))
    } else {
        None
    }
}

pub(super) fn target_backpressure_error() -> RelayError {
    relay_error(
        "target_backpressure",
        "target daemon relay queue is full",
        true,
    )
}

pub(super) fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn relay_log(level: &str, event: &str, fields: Value) {
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

pub(super) fn target_log_value(target: &ClientTarget) -> Value {
    json!({
        "daemon_id": target.daemon_id,
        "daemon_alias": target.daemon_alias,
    })
}

pub(super) fn daemon_key_log_value(daemon_key: &DaemonKey) -> Value {
    json!({
        "realm_id": daemon_key.realm_id,
        "daemon_id": daemon_key.daemon_id,
    })
}
