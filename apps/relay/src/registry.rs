use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;

use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::auth::{RelayAction, DEFAULT_RELAY_REALM_ID};
use crate::protocol::{
    DaemonRegistration, RelayCallerIdentity, RelayConnectionRole, RelayDisplayTunnelHeader,
    RelayError, RelayKernelPresence, RelayMachinePresence, RelayProviderAccountSummary,
};

pub(crate) type RelaySender = mpsc::Sender<Message>;
pub(crate) type DisplayStreamSender = mpsc::Sender<DisplayStreamEvent>;

#[derive(Debug, Clone)]
pub(crate) struct PeerHandle {
    pub(crate) sender: RelaySender,
    pub(crate) role: RelayConnectionRole,
    pub(crate) realm_id: Option<String>,
    pub(crate) identity: Option<RelayCallerIdentity>,
    pub(crate) allowed_actions: Vec<RelayAction>,
    pub(crate) daemon_registration: Option<DaemonRegistration>,
    pub(crate) client_daemon_key: Option<DaemonKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DaemonKey {
    pub(crate) realm_id: String,
    pub(crate) daemon_id: String,
}

impl DaemonKey {
    pub(crate) fn new(realm_id: impl Into<String>, daemon_id: impl Into<String>) -> Self {
        Self {
            realm_id: realm_id.into(),
            daemon_id: daemon_id.into(),
        }
    }
}

pub(crate) fn daemon_registration_is_kernel_target(registration: &DaemonRegistration) -> bool {
    registration
        .capabilities
        .iter()
        .any(|capability| capability == "kernel_websocket" || capability == "kernel_ws")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedPeer {
    pub role: RelayConnectionRole,
    pub identity: Option<RelayCallerIdentity>,
    pub daemon_registration: Option<DaemonRegistration>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingClientRequest {
    pub(crate) client_addr: SocketAddr,
    pub(crate) client_request_id: String,
    pub(crate) daemon_key: DaemonKey,
    pub(crate) kind: PendingRequestKind,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingDaemonPeerRequest {
    pub(crate) requester_daemon_key: DaemonKey,
    pub(crate) requester_request_id: String,
    pub(crate) target_daemon_key: DaemonKey,
}

#[derive(Debug, Clone)]
pub(crate) enum PendingRequestKind {
    Request,
    Subscribe { subscription_id: String },
    Unsubscribe { subscription_id: String },
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveSubscription {
    pub(crate) client_addr: SocketAddr,
    pub(crate) daemon_key: DaemonKey,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelayBackpressureMetrics {
    pub target_queue_full_count: u64,
    pub slow_subscription_close_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisplayTunnelRegistration {
    pub(crate) daemon_key: DaemonKey,
    pub(crate) expires_at_ms: u64,
    pub(crate) capabilities: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum DisplayTunnelLookup {
    Active {
        daemon_key: DaemonKey,
        daemon_sender: Option<RelaySender>,
    },
    Expired,
    Missing,
}

#[derive(Debug, Clone)]
pub(crate) enum DisplayStreamEvent {
    ResponseStart {
        status: u16,
        headers: Vec<RelayDisplayTunnelHeader>,
    },
    Chunk {
        data: String,
        message_kind: Option<String>,
    },
    Close {
        error: Option<RelayError>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct PendingDisplayStream {
    pub(crate) daemon_key: DaemonKey,
    pub(crate) sender: DisplayStreamSender,
}

#[derive(Debug, Default)]
pub struct RelayRegistry {
    pub(crate) peers: BTreeMap<SocketAddr, PeerHandle>,
    pub(crate) daemons: BTreeMap<DaemonKey, DaemonRegistration>,
    pub(crate) daemon_peers: BTreeMap<DaemonKey, SocketAddr>,
    pub(crate) pending_requests: BTreeMap<String, PendingClientRequest>,
    pub(crate) pending_daemon_peer_requests: BTreeMap<String, PendingDaemonPeerRequest>,
    pub(crate) subscriptions: BTreeMap<String, ActiveSubscription>,
    pub(crate) display_tunnels: BTreeMap<String, DisplayTunnelRegistration>,
    pub(crate) pending_display_streams: BTreeMap<String, PendingDisplayStream>,
    backpressure_metrics: RelayBackpressureMetrics,
}

impl RelayRegistry {
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    pub fn daemon_count(&self) -> usize {
        self.daemons.len()
    }

    pub fn daemon(&self, daemon_id: &str) -> Option<&DaemonRegistration> {
        self.daemon_in_realm(DEFAULT_RELAY_REALM_ID, daemon_id)
    }

    pub fn daemon_in_realm(&self, realm_id: &str, daemon_id: &str) -> Option<&DaemonRegistration> {
        self.daemons
            .get(&DaemonKey::new(realm_id.to_string(), daemon_id.to_string()))
    }

    pub fn pending_request_count(&self) -> usize {
        self.pending_requests.len() + self.pending_daemon_peer_requests.len()
    }

    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    pub fn display_tunnel_count(&self) -> usize {
        self.display_tunnels.len()
    }

    pub fn backpressure_metrics(&self) -> RelayBackpressureMetrics {
        self.backpressure_metrics
    }

    pub(crate) fn record_target_queue_full(&mut self) {
        self.backpressure_metrics.target_queue_full_count = self
            .backpressure_metrics
            .target_queue_full_count
            .saturating_add(1);
    }

    pub(crate) fn record_slow_subscription_close(&mut self) {
        self.backpressure_metrics.slow_subscription_close_count = self
            .backpressure_metrics
            .slow_subscription_close_count
            .saturating_add(1);
    }

    pub(crate) fn register_display_tunnel(
        &mut self,
        daemon_key: DaemonKey,
        tunnel_id: String,
        expires_at_ms: u64,
        capabilities: Vec<String>,
    ) {
        self.display_tunnels.insert(
            tunnel_id,
            DisplayTunnelRegistration {
                daemon_key,
                expires_at_ms,
                capabilities,
            },
        );
    }

    pub(crate) fn revoke_display_tunnel(&mut self, tunnel_id: &str) -> bool {
        self.display_tunnels.remove(tunnel_id).is_some()
    }

    pub(crate) fn display_tunnel(
        &self,
        tunnel_id: &str,
        now_ms: u64,
    ) -> Option<&DisplayTunnelRegistration> {
        self.display_tunnels
            .get(tunnel_id)
            .filter(|registration| registration.expires_at_ms > now_ms)
    }

    pub(crate) fn display_tunnel_lookup(
        &self,
        tunnel_id: &str,
        now_ms: u64,
    ) -> DisplayTunnelLookup {
        let Some(registration) = self.display_tunnels.get(tunnel_id) else {
            return DisplayTunnelLookup::Missing;
        };
        if registration.expires_at_ms <= now_ms {
            return DisplayTunnelLookup::Expired;
        }
        DisplayTunnelLookup::Active {
            daemon_key: registration.daemon_key.clone(),
            daemon_sender: self.resolve_daemon_sender(&registration.daemon_key),
        }
    }

    pub(crate) fn prune_expired_display_tunnels(&mut self, now_ms: u64) -> usize {
        let before = self.display_tunnels.len();
        self.display_tunnels
            .retain(|_, registration| registration.expires_at_ms > now_ms);
        before.saturating_sub(self.display_tunnels.len())
    }

    pub(crate) fn remove_display_tunnels_for_daemon(&mut self, daemon_key: &DaemonKey) -> usize {
        let before = self.display_tunnels.len();
        self.display_tunnels
            .retain(|_, registration| registration.daemon_key != *daemon_key);
        before.saturating_sub(self.display_tunnels.len())
    }

    pub(crate) fn remove_display_streams_for_daemon(
        &mut self,
        daemon_key: &DaemonKey,
    ) -> Vec<DisplayStreamSender> {
        let doomed_stream_ids = self
            .pending_display_streams
            .iter()
            .filter(|(_, stream)| stream.daemon_key == *daemon_key)
            .map(|(stream_id, _)| stream_id.clone())
            .collect::<Vec<_>>();
        doomed_stream_ids
            .into_iter()
            .filter_map(|stream_id| {
                self.pending_display_streams
                    .remove(&stream_id)
                    .map(|stream| stream.sender)
            })
            .collect()
    }

    pub(crate) fn resolve_daemon_sender(&self, daemon_key: &DaemonKey) -> Option<RelaySender> {
        let registration = self.daemons.get(daemon_key)?;
        let peer_addr = self.daemon_peers.get(daemon_key)?;
        let peer = self.peers.get(peer_addr)?;
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

    pub(crate) fn insert_pending_display_stream(
        &mut self,
        stream_id: String,
        daemon_key: DaemonKey,
        sender: DisplayStreamSender,
    ) {
        self.pending_display_streams
            .insert(stream_id, PendingDisplayStream { daemon_key, sender });
    }

    pub(crate) fn remove_pending_display_stream(&mut self, stream_id: &str) {
        self.pending_display_streams.remove(stream_id);
    }

    pub(crate) fn display_stream_sender_for_daemon(
        &self,
        stream_id: &str,
        daemon_key: &DaemonKey,
    ) -> Option<DisplayStreamSender> {
        self.pending_display_streams
            .get(stream_id)
            .filter(|stream| stream.daemon_key == *daemon_key)
            .map(|stream| stream.sender.clone())
    }

    pub fn connected_peer(&self, peer_addr: &SocketAddr) -> Option<ConnectedPeer> {
        self.peers.get(peer_addr).map(|peer| ConnectedPeer {
            role: peer.role.clone(),
            identity: peer.identity.clone(),
            daemon_registration: peer.daemon_registration.clone(),
        })
    }

    pub fn live_machines(&self) -> Vec<RelayMachinePresence> {
        self.live_machines_in_realm(DEFAULT_RELAY_REALM_ID)
    }

    pub(crate) fn live_machines_in_realm(&self, realm_id: &str) -> Vec<RelayMachinePresence> {
        let relay_aliases = self.relay_kernel_aliases(realm_id);
        let mut grouped = BTreeMap::<String, Vec<&DaemonRegistration>>::new();
        for registration in self.daemon_registrations_in_realm(realm_id) {
            grouped
                .entry(registration.machine_id.clone())
                .or_default()
                .push(registration);
        }
        grouped
            .into_iter()
            .map(|(machine_id, registrations)| {
                let mut available_providers = registrations
                    .iter()
                    .flat_map(|registration| registration.available_providers.iter().cloned())
                    .collect::<Vec<_>>();
                available_providers.sort();
                available_providers.dedup();
                let provider_accounts = dedup_provider_accounts(
                    registrations
                        .iter()
                        .flat_map(|registration| registration.provider_accounts.iter().cloned()),
                );
                let machine_alias = registrations
                    .iter()
                    .min_by_key(|registration| normalized_kernel_started_at_ms(registration))
                    .and_then(|registration| relay_aliases.get(&registration.daemon_id))
                    .cloned();
                RelayMachinePresence {
                    machine_alias,
                    machine_id,
                    kernel_count: registrations.len(),
                    available_providers,
                    provider_accounts,
                }
            })
            .collect()
    }

    pub fn live_kernels_for_machine(&self, machine_ref: &str) -> Vec<RelayKernelPresence> {
        self.live_kernels_for_machine_in_realm(DEFAULT_RELAY_REALM_ID, machine_ref)
    }

    pub(crate) fn live_kernels_for_machine_in_realm(
        &self,
        realm_id: &str,
        machine_ref: &str,
    ) -> Vec<RelayKernelPresence> {
        let relay_aliases = self.relay_kernel_aliases(realm_id);
        self.daemon_registrations_in_realm(realm_id)
            .filter(|registration| {
                registration.machine_id == machine_ref
                    || relay_aliases
                        .get(&registration.daemon_id)
                        .map(String::as_str)
                        == Some(machine_ref)
                    || registration.machine_alias.as_deref() == Some(machine_ref)
            })
            .map(|registration| self.kernel_presence(registration, &relay_aliases))
            .collect()
    }

    pub fn live_kernel(&self, kernel_ref: &str) -> Option<RelayKernelPresence> {
        self.live_kernel_in_realm(DEFAULT_RELAY_REALM_ID, kernel_ref)
    }

    pub(crate) fn live_kernel_in_realm(
        &self,
        realm_id: &str,
        kernel_ref: &str,
    ) -> Option<RelayKernelPresence> {
        let relay_aliases = self.relay_kernel_aliases(realm_id);
        self.daemon_registrations_in_realm(realm_id)
            .find(|registration| {
                registration.daemon_id == kernel_ref
                    || registration.daemon_alias.as_deref() == Some(kernel_ref)
                    || registration.kernel_alias.as_deref() == Some(kernel_ref)
                    || relay_aliases
                        .get(&registration.daemon_id)
                        .map(String::as_str)
                        == Some(kernel_ref)
            })
            .map(|registration| self.kernel_presence(registration, &relay_aliases))
    }

    fn kernel_presence(
        &self,
        registration: &DaemonRegistration,
        relay_aliases: &BTreeMap<String, String>,
    ) -> RelayKernelPresence {
        RelayKernelPresence {
            kernel_id: registration.daemon_id.clone(),
            machine_id: registration.machine_id.clone(),
            machine_alias: relay_aliases.get(&registration.daemon_id).cloned(),
            relay_alias: relay_aliases.get(&registration.daemon_id).cloned(),
            kernel_alias: registration
                .kernel_alias
                .clone()
                .or_else(|| registration.daemon_alias.clone()),
            available_providers: registration.available_providers.clone(),
            provider_accounts: registration.provider_accounts.clone(),
            capabilities: registration.capabilities.clone(),
            accepting_remote_leases: registration.accepting_remote_leases,
            leased_agent_count: registration.leased_agent_count,
            local_session_count: registration.local_session_count,
            public_key: registration.public_key.clone(),
        }
    }

    fn daemon_registrations_in_realm<'a>(
        &'a self,
        realm_id: &'a str,
    ) -> impl Iterator<Item = &'a DaemonRegistration> + 'a {
        self.daemons
            .iter()
            .filter(move |(key, _)| key.realm_id == realm_id)
            .map(|(_, registration)| registration)
            .filter(|registration| daemon_registration_is_kernel_target(registration))
    }

    fn relay_kernel_aliases(&self, realm_id: &str) -> BTreeMap<String, String> {
        let mut registrations = self
            .daemon_registrations_in_realm(realm_id)
            .collect::<Vec<_>>();
        registrations.sort_by(|left, right| {
            normalized_kernel_started_at_ms(left)
                .cmp(&normalized_kernel_started_at_ms(right))
                .then_with(|| left.daemon_id.cmp(&right.daemon_id))
        });

        registrations
            .into_iter()
            .enumerate()
            .map(|(index, registration)| {
                let os_name = registration
                    .os_name
                    .clone()
                    .or_else(|| registration.machine_alias.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                (
                    registration.daemon_id.clone(),
                    format!("machine {} ({})", index + 1, os_name),
                )
            })
            .collect()
    }
}

fn dedup_provider_accounts(
    accounts: impl IntoIterator<Item = RelayProviderAccountSummary>,
) -> Vec<RelayProviderAccountSummary> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for account in accounts {
        let key = (
            account.provider.clone(),
            account.alias.clone(),
            account.email.clone(),
            account.account_id.clone(),
            account.auth_type.clone(),
            account.state.clone(),
        );
        if seen.insert(key) {
            deduped.push(account);
        }
    }
    deduped.sort_by(|left, right| {
        (
            left.provider.as_str(),
            left.alias.as_deref().unwrap_or(""),
            left.email.as_deref().unwrap_or(""),
            left.account_id.as_deref().unwrap_or(""),
        )
            .cmp(&(
                right.provider.as_str(),
                right.alias.as_deref().unwrap_or(""),
                right.email.as_deref().unwrap_or(""),
                right.account_id.as_deref().unwrap_or(""),
            ))
    });
    deduped
}

fn normalized_kernel_started_at_ms(registration: &DaemonRegistration) -> u64 {
    if registration.kernel_started_at_ms == 0 {
        u64::MAX
    } else {
        registration.kernel_started_at_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_tunnels_are_scoped_to_daemon_and_expire() {
        let mut registry = RelayRegistry::default();
        let daemon_key = DaemonKey::new("realm-1", "daemon-1");
        registry.register_display_tunnel(
            daemon_key.clone(),
            "tunnel-1".to_string(),
            1_000,
            vec!["view".to_string(), "keyboard".to_string()],
        );

        let tunnel = registry
            .display_tunnel("tunnel-1", 999)
            .expect("unexpired tunnel should resolve");
        assert_eq!(tunnel.daemon_key, daemon_key);
        assert_eq!(tunnel.capabilities, vec!["view", "keyboard"]);
        assert!(registry.display_tunnel("tunnel-1", 1_000).is_none());

        registry.prune_expired_display_tunnels(1_000);
        assert_eq!(registry.display_tunnel_count(), 0);
    }

    #[test]
    fn display_tunnels_revoke_and_disconnect_with_their_daemon() {
        let mut registry = RelayRegistry::default();
        let daemon_a = DaemonKey::new("realm-1", "daemon-a");
        let daemon_b = DaemonKey::new("realm-1", "daemon-b");
        registry.register_display_tunnel(daemon_a.clone(), "a-1".to_string(), 10_000, Vec::new());
        registry.register_display_tunnel(daemon_a.clone(), "a-2".to_string(), 10_000, Vec::new());
        registry.register_display_tunnel(daemon_b.clone(), "b-1".to_string(), 10_000, Vec::new());

        assert!(registry.revoke_display_tunnel("a-1"));
        assert!(!registry.revoke_display_tunnel("missing"));
        assert_eq!(registry.display_tunnel_count(), 2);

        assert_eq!(registry.remove_display_tunnels_for_daemon(&daemon_a), 1);
        assert!(registry.display_tunnel("a-2", 1).is_none());
        assert!(registry.display_tunnel("b-1", 1).is_some());
    }
}
