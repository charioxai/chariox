use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::{Arc, RwLock as StdRwLock};

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
    pub(crate) allowed_targets: Option<Vec<String>>,
    pub(crate) daemon_registration: Option<DaemonRegistration>,
    pub(crate) client_daemon_key: Option<DaemonKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct DaemonKey {
    pub(crate) realm_id: String,
    pub(crate) daemon_id: String,
}

const RELAY_ROUTE_SHARD_COUNT: usize = 64;

#[derive(Debug)]
struct ShardedRouteMap<K, V> {
    shards: Box<[StdRwLock<HashMap<K, V>>]>,
}

impl<K, V> Default for ShardedRouteMap<K, V> {
    fn default() -> Self {
        Self {
            shards: (0..RELAY_ROUTE_SHARD_COUNT)
                .map(|_| StdRwLock::new(HashMap::new()))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }
}

impl<K, V> ShardedRouteMap<K, V>
where
    K: Clone + Eq + Hash,
    V: Clone,
{
    fn shard_index(key: &K) -> usize {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % RELAY_ROUTE_SHARD_COUNT
    }

    fn get(&self, key: &K) -> Option<V> {
        self.shards[Self::shard_index(key)]
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn insert(&self, key: K, value: V) {
        self.shards[Self::shard_index(&key)]
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, value);
    }

    fn remove(&self, key: &K) -> Option<V> {
        self.shards[Self::shard_index(key)]
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key)
    }

    fn remove_if(&self, key: &K, predicate: impl FnOnce(&V) -> bool) -> Option<V> {
        let mut shard = self.shards[Self::shard_index(key)]
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if shard.get(key).is_some_and(predicate) {
            shard.remove(key)
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| {
                shard
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .len()
            })
            .sum()
    }

    fn values(&self) -> Vec<V> {
        self.shards
            .iter()
            .flat_map(|shard| {
                shard
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn drain_where(&self, predicate: impl Fn(&V) -> bool) -> Vec<V> {
        let mut removed = Vec::new();
        for shard in self.shards.iter() {
            let mut shard = shard
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let keys = shard
                .iter()
                .filter(|(_, value)| predicate(value))
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            removed.extend(keys.into_iter().filter_map(|key| shard.remove(&key)));
        }
        removed
    }
}

impl<V: Clone> ShardedRouteMap<String, V> {
    fn get_str(&self, key: &str) -> Option<V> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        self.shards[(hasher.finish() as usize) % RELAY_ROUTE_SHARD_COUNT]
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn remove_str(&self, key: &str) -> Option<V> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        self.shards[(hasher.finish() as usize) % RELAY_ROUTE_SHARD_COUNT]
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveEventRoute {
    pub(crate) daemon_key: DaemonKey,
    pub(crate) client_sender: RelaySender,
}

#[derive(Debug, Default)]
pub(crate) struct RelayRouteIndex {
    daemon_senders: ShardedRouteMap<DaemonKey, RelaySender>,
    client_senders: ShardedRouteMap<SocketAddr, RelaySender>,
    subscriptions: ShardedRouteMap<String, ActiveEventRoute>,
    pending_client_requests: ShardedRouteMap<String, PendingClientRequest>,
    pending_daemon_requests: ShardedRouteMap<String, PendingDaemonPeerRequest>,
}

impl RelayRouteIndex {
    pub(crate) fn daemon_sender(&self, daemon_key: &DaemonKey) -> Option<RelaySender> {
        self.daemon_senders.get(daemon_key)
    }

    pub(crate) fn set_daemon_sender(&self, daemon_key: DaemonKey, sender: RelaySender) {
        self.daemon_senders.insert(daemon_key, sender);
    }

    pub(crate) fn remove_daemon_sender(&self, daemon_key: &DaemonKey) {
        self.daemon_senders.remove(daemon_key);
    }

    pub(crate) fn set_client_sender(&self, peer_addr: SocketAddr, sender: RelaySender) {
        self.client_senders.insert(peer_addr, sender);
    }

    pub(crate) fn client_sender(&self, peer_addr: &SocketAddr) -> Option<RelaySender> {
        self.client_senders.get(peer_addr)
    }

    pub(crate) fn remove_client_sender(&self, peer_addr: &SocketAddr) {
        self.client_senders.remove(peer_addr);
    }

    pub(crate) fn set_subscription(&self, subscription_id: String, route: ActiveEventRoute) {
        self.subscriptions.insert(subscription_id, route);
    }

    pub(crate) fn subscription(&self, subscription_id: &str) -> Option<ActiveEventRoute> {
        self.subscriptions.get_str(subscription_id)
    }

    pub(crate) fn remove_subscription(&self, subscription_id: &str) {
        self.subscriptions.remove_str(subscription_id);
    }

    pub(crate) fn insert_pending_client(
        &self,
        relay_request_id: String,
        pending: PendingClientRequest,
    ) {
        self.pending_client_requests
            .insert(relay_request_id, pending);
    }

    pub(crate) fn remove_pending_client(
        &self,
        relay_request_id: &str,
    ) -> Option<PendingClientRequest> {
        self.pending_client_requests.remove_str(relay_request_id)
    }

    pub(crate) fn take_pending_client_if(
        &self,
        relay_request_id: &str,
        predicate: impl FnOnce(&PendingClientRequest) -> bool,
    ) -> Option<PendingClientRequest> {
        let key = relay_request_id.to_string();
        self.pending_client_requests.remove_if(&key, predicate)
    }

    pub(crate) fn pending_clients(&self) -> Vec<PendingClientRequest> {
        self.pending_client_requests.values()
    }

    pub(crate) fn drain_pending_clients_where(
        &self,
        predicate: impl Fn(&PendingClientRequest) -> bool,
    ) -> Vec<PendingClientRequest> {
        self.pending_client_requests.drain_where(predicate)
    }

    pub(crate) fn insert_pending_daemon(
        &self,
        relay_request_id: String,
        pending: PendingDaemonPeerRequest,
    ) {
        self.pending_daemon_requests
            .insert(relay_request_id, pending);
    }

    pub(crate) fn remove_pending_daemon(
        &self,
        relay_request_id: &str,
    ) -> Option<PendingDaemonPeerRequest> {
        self.pending_daemon_requests.remove_str(relay_request_id)
    }

    pub(crate) fn take_pending_daemon_if(
        &self,
        relay_request_id: &str,
        predicate: impl FnOnce(&PendingDaemonPeerRequest) -> bool,
    ) -> Option<PendingDaemonPeerRequest> {
        let key = relay_request_id.to_string();
        self.pending_daemon_requests.remove_if(&key, predicate)
    }

    pub(crate) fn drain_pending_daemons_where(
        &self,
        predicate: impl Fn(&PendingDaemonPeerRequest) -> bool,
    ) -> Vec<PendingDaemonPeerRequest> {
        self.pending_daemon_requests.drain_where(predicate)
    }

    pub(crate) fn pending_request_count(&self) -> usize {
        self.pending_client_requests.len() + self.pending_daemon_requests.len()
    }
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
    Subscribe {
        subscription_id: String,
        client_public_key: String,
    },
    Unsubscribe {
        subscription_id: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveSubscription {
    pub(crate) client_addr: SocketAddr,
    pub(crate) daemon_key: DaemonKey,
    pub(crate) client_public_key: String,
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
        capabilities: Vec<String>,
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

#[derive(Debug)]
pub struct RelayRegistry {
    pub(crate) peers: BTreeMap<SocketAddr, PeerHandle>,
    pub(crate) daemons: BTreeMap<DaemonKey, DaemonRegistration>,
    pub(crate) daemon_peers: BTreeMap<DaemonKey, SocketAddr>,
    pub(crate) subscriptions: BTreeMap<String, ActiveSubscription>,
    pub(crate) display_tunnels: BTreeMap<String, DisplayTunnelRegistration>,
    pub(crate) pending_display_streams: BTreeMap<String, PendingDisplayStream>,
    backpressure_metrics: RelayBackpressureMetrics,
    routes: Arc<RelayRouteIndex>,
}

impl Default for RelayRegistry {
    fn default() -> Self {
        Self {
            peers: BTreeMap::new(),
            daemons: BTreeMap::new(),
            daemon_peers: BTreeMap::new(),
            subscriptions: BTreeMap::new(),
            display_tunnels: BTreeMap::new(),
            pending_display_streams: BTreeMap::new(),
            backpressure_metrics: RelayBackpressureMetrics::default(),
            routes: Arc::new(RelayRouteIndex::default()),
        }
    }
}

impl RelayRegistry {
    pub(crate) fn route_index(&self) -> Arc<RelayRouteIndex> {
        Arc::clone(&self.routes)
    }
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
        self.routes.pending_request_count()
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
            capabilities: registration.capabilities.clone(),
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

    pub(crate) fn live_daemon_sender(&self, daemon_key: &DaemonKey) -> Option<RelaySender> {
        self.resolve_daemon_sender(daemon_key)?;
        self.routes.daemon_sender(daemon_key)
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
            .filter(move |(key, _)| {
                key.realm_id == realm_id && self.live_daemon_sender(key).is_some()
            })
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

    #[tokio::test]
    async fn route_index_resolves_packet_paths_without_registry_state() {
        let routes = RelayRouteIndex::default();
        let daemon_key = DaemonKey::new("realm-1", "daemon-1");
        let (daemon_sender, _daemon_receiver) = mpsc::channel(1);
        let (client_sender, _client_receiver) = mpsc::channel(1);
        let client_addr = "127.0.0.1:41001".parse().expect("client address");

        routes.set_daemon_sender(daemon_key.clone(), daemon_sender.clone());
        routes.set_client_sender(client_addr, client_sender.clone());
        routes.set_subscription(
            "subscription-1".to_string(),
            ActiveEventRoute {
                daemon_key: daemon_key.clone(),
                client_sender,
            },
        );

        assert!(routes.daemon_sender(&daemon_key).is_some());
        assert!(routes.client_sender(&client_addr).is_some());
        assert_eq!(
            routes
                .subscription("subscription-1")
                .expect("event route")
                .daemon_key,
            daemon_key
        );
        routes.remove_subscription("subscription-1");
        routes.remove_client_sender(&client_addr);
        routes.remove_daemon_sender(&daemon_key);
        assert!(routes.subscription("subscription-1").is_none());
        assert!(routes.client_sender(&client_addr).is_none());
        assert!(routes.daemon_sender(&daemon_key).is_none());
    }

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
