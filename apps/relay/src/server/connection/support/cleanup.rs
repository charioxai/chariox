use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::protocol::RelayConnectionRole;
use crate::registry::{DaemonKey, DisplayStreamSender, RelayRegistry, RelaySender};

use super::resolve_daemon_sender_locked;

pub(in crate::server::connection) async fn remove_peer(
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
