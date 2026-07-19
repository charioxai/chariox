use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::protocol::RelayConnectionRole;
use crate::registry::{
    DaemonKey, DisplayStreamSender, PendingRequestKind, RelayRegistry, RelayRouteIndex, RelaySender,
};

use super::send_daemon_subscription_cleanup;

pub(in crate::server::connection) async fn remove_peer(
    registry: &Arc<RwLock<RelayRegistry>>,
    routes: &Arc<RelayRouteIndex>,
    peer_addr: SocketAddr,
    daemon_key: Option<&DaemonKey>,
    relay_request_counter: &AtomicU64,
) -> (
    Vec<(RelaySender, String)>,
    Vec<(RelaySender, String, String)>,
    Vec<RelaySender>,
    Vec<DisplayStreamSender>,
    usize,
    usize,
) {
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
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                daemon_subscription_cleanups,
                dropped_client_pending_requests,
            );
        }
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
        for pending in
            routes.drain_pending_clients_where(|pending| &pending.daemon_key == daemon_key)
        {
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
        return (
            client_errors,
            daemon_errors,
            subscription_client_senders,
            display_stream_senders,
            daemon_subscription_cleanups,
            dropped_client_pending_requests,
        );
    }
    (
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        daemon_subscription_cleanups,
        dropped_client_pending_requests,
    )
}
