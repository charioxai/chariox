use std::collections::BTreeMap;
use std::net::SocketAddr;

use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::auth::DEFAULT_RELAY_REALM_ID;
use crate::protocol::{
    DaemonRegistration, RelayCallerIdentity, RelayConnectionRole, RelayKernelPresence,
    RelayMachinePresence,
};

pub(crate) type RelaySender = mpsc::Sender<Message>;

#[derive(Debug, Clone)]
pub(crate) struct PeerHandle {
    pub(crate) sender: RelaySender,
    pub(crate) role: RelayConnectionRole,
    pub(crate) realm_id: Option<String>,
    pub(crate) identity: Option<RelayCallerIdentity>,
    pub(crate) daemon_registration: Option<DaemonRegistration>,
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

#[derive(Debug, Default)]
pub struct RelayRegistry {
    pub(crate) peers: BTreeMap<SocketAddr, PeerHandle>,
    pub(crate) daemons: BTreeMap<DaemonKey, DaemonRegistration>,
    pub(crate) pending_requests: BTreeMap<String, PendingClientRequest>,
    pub(crate) pending_daemon_peer_requests: BTreeMap<String, PendingDaemonPeerRequest>,
    pub(crate) subscriptions: BTreeMap<String, ActiveSubscription>,
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

fn normalized_kernel_started_at_ms(registration: &DaemonRegistration) -> u64 {
    if registration.kernel_started_at_ms == 0 {
        u64::MAX
    } else {
        registration.kernel_started_at_ms
    }
}
