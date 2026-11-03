use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::RelayConnectionRole;
use crate::registry::{
    DaemonKey, DisplayStreamSender, PendingRequestKind, RelayRegistry, RelayRouteIndex, RelaySender,
};

use super::{relay_error, send_daemon_subscription_cleanup, send_envelope};
use crate::protocol::RelayEnvelope;
use crate::registry::DisplayStreamEvent;

pub(in crate::server::connection) async fn remove_peer(
    registry: &Arc<RwLock<RelayRegistry>>,
    routes: &Arc<RelayRouteIndex>,
    peer_addr: SocketAddr,
    daemon_key: Option<&DaemonKey>,
    relay_request_counter: &AtomicU64,
) -> (ReleasedDaemon, usize, usize) {
    let mut guard = registry.write().await;
    let removed_peer = guard.peers.remove(&peer_addr);
    let active_client_subscriptions = guard
        .subscriptions
        .iter()
        .filter(|(_, active)| active.client_addr == peer_addr)
        .map(|(subscription_id, active)| {
            (
                subscription_id.clone(),
                active.daemon_key.clone(),
                active.client_public_key.clone(),
            )
        })
        .collect::<Vec<_>>();
    for (subscription_id, _, _) in &active_client_subscriptions {
        guard.subscriptions.remove(subscription_id);
        routes.remove_subscription(subscription_id);
    }
    routes.remove_client_sender(&peer_addr);
    let dropped_client_pending = if removed_peer
        .as_ref()
        .is_some_and(|peer| peer.role == RelayConnectionRole::Client)
    {
        routes.drain_pending_clients_where(|pending| pending.client_addr == peer_addr)
    } else {
        Vec::new()
    };
    let dropped_client_pending_requests = dropped_client_pending.len();
    let mut abandoned_subscriptions = active_client_subscriptions;
    abandoned_subscriptions.extend(dropped_client_pending.iter().filter_map(|pending| {
        let PendingRequestKind::Subscribe {
            subscription_id,
            client_public_key,
            ..
        } = &pending.kind
        else {
            return None;
        };
        Some((
            subscription_id.clone(),
            pending.daemon_key.clone(),
            client_public_key.clone(),
        ))
    }));
    let daemon_subscription_cleanups = abandoned_subscriptions
        .into_iter()
        .filter(|(subscription_id, daemon_key, client_public_key)| {
            routes.daemon_sender(daemon_key).is_some_and(|sender| {
                send_daemon_subscription_cleanup(
                    &sender,
                    relay_request_counter,
                    subscription_id.clone(),
                    client_public_key.clone(),
                )
                .is_ok()
            })
        })
        .count();
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
                ReleasedDaemon::default(),
                daemon_subscription_cleanups,
                dropped_client_pending_requests,
            );
        }
        let released = release_daemon(&mut guard, routes, daemon_key);
        return (
            released,
            daemon_subscription_cleanups,
            dropped_client_pending_requests,
        );
    }
    (
        ReleasedDaemon::default(),
        daemon_subscription_cleanups,
        dropped_client_pending_requests,
    )
}

/// What a daemon registration leaves behind when it ends: requests nobody will
/// answer, peers waiting on it, and client streams bound to it.
#[derive(Default)]
pub(in crate::server::connection) struct ReleasedDaemon {
    pub(in crate::server::connection) client_errors: Vec<(RelaySender, String)>,
    pub(in crate::server::connection) peer_errors: Vec<(RelaySender, String, String)>,
    pub(in crate::server::connection) subscription_senders: Vec<RelaySender>,
    pub(in crate::server::connection) display_stream_senders: Vec<DisplayStreamSender>,
}

impl ReleasedDaemon {
    pub(in crate::server::connection) fn is_empty(&self) -> bool {
        self.client_errors.is_empty()
            && self.peer_errors.is_empty()
            && self.subscription_senders.is_empty()
            && self.display_stream_senders.is_empty()
    }
}

/// Ends `daemon_key`'s registration: on disconnect, and when the daemon
/// registers again from a new socket while the old one is silently gone.
pub(in crate::server::connection) fn release_daemon(
    guard: &mut RelayRegistry,
    routes: &RelayRouteIndex,
    daemon_key: &DaemonKey,
) -> ReleasedDaemon {
    guard.daemons.remove(daemon_key);
    guard.daemon_peers.remove(daemon_key);
    routes.remove_daemon_sender(daemon_key);
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
        routes.remove_subscription(&subscription_id);
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
    let mut client_errors = Vec::new();
    for pending in routes.drain_pending_clients_where(|pending| &pending.daemon_key == daemon_key) {
        if let Some(sender) = routes.client_sender(&pending.client_addr) {
            client_errors.push((sender, pending.client_request_id));
        }
    }
    let doomed_peer_requests = routes.drain_pending_daemons_where(|pending| {
        &pending.target_daemon_key == daemon_key || &pending.requester_daemon_key == daemon_key
    });
    let mut daemon_errors = Vec::new();
    for pending in doomed_peer_requests {
        if &pending.requester_daemon_key == daemon_key {
            continue;
        }
        if let Some(sender) = routes.daemon_sender(&pending.requester_daemon_key) {
            daemon_errors.push((
                sender,
                pending.requester_request_id,
                pending.target_daemon_key.daemon_id,
            ));
        }
    }
    ReleasedDaemon {
        client_errors,
        peer_errors: daemon_errors,
        subscription_senders: subscription_client_senders,
        display_stream_senders,
    }
}

/// Tells everyone waiting on a released daemon that it is gone, so clients
/// retry on its next registration instead of waiting for their timeouts.
/// `kernel_reload`: the daemon said it ends for a hot reload; its subscribers
/// hear so and retry soon.
pub(in crate::server::connection) fn announce_released_daemon(
    released: ReleasedDaemon,
    kernel_reload: bool,
) {
    for (sender, request_id) in released.client_errors {
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
    for (sender, request_id, target_daemon_id) in released.peer_errors {
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
    for sender in released.subscription_senders {
        let _ = send_envelope(
            &sender,
            &RelayEnvelope::Close {
                reason: "target daemon disconnected from relay".to_string(),
                kernel_reload,
            },
        );
        let _ = sender.try_send(Message::Close(None));
    }
    for sender in released.display_stream_senders {
        // Removing the registry sender and dropping this clone also closes a full
        // queue after it drains. Never wait on an old viewer in a new daemon's reader.
        let _ = sender.try_send(DisplayStreamEvent::Close {
            error: Some(relay_error(
                "target_disconnected",
                "target daemon disconnected from relay",
                true,
            )),
        });
    }
}
