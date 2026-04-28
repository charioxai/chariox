use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use crate::auth::{
    RelayAction, RelayAuthError, RelayAuthRequest, RelayAuthVerifier, VerifiedRelayIdentity,
    DEFAULT_RELAY_REALM_ID,
};
use crate::config::RelayConfig;
use crate::protocol::{
    ClientTarget, DaemonRegistration, RelayCallerIdentity, RelayConnectionRole, RelayEnvelope,
    RelayError, RelayKernelPresence, RelayMachinePresence, RelayMetadataQuery,
};

#[derive(Debug, Clone)]
struct PeerHandle {
    sender: mpsc::UnboundedSender<Message>,
    role: RelayConnectionRole,
    realm_id: Option<String>,
    identity: Option<RelayCallerIdentity>,
    daemon_registration: Option<DaemonRegistration>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DaemonKey {
    realm_id: String,
    daemon_id: String,
}

impl DaemonKey {
    fn new(realm_id: impl Into<String>, daemon_id: impl Into<String>) -> Self {
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
struct PendingClientRequest {
    client_addr: SocketAddr,
    client_request_id: String,
    daemon_key: DaemonKey,
    kind: PendingRequestKind,
}

#[derive(Debug, Clone)]
struct PendingDaemonPeerRequest {
    requester_daemon_key: DaemonKey,
    requester_request_id: String,
    target_daemon_key: DaemonKey,
}

#[derive(Debug, Clone)]
enum PendingRequestKind {
    Request,
    Subscribe { subscription_id: String },
    Unsubscribe { subscription_id: String },
}

#[derive(Debug, Clone)]
struct ActiveSubscription {
    client_addr: SocketAddr,
    daemon_key: DaemonKey,
}

#[derive(Debug, Default)]
pub struct RelayRegistry {
    peers: BTreeMap<SocketAddr, PeerHandle>,
    daemons: BTreeMap<DaemonKey, DaemonRegistration>,
    pending_requests: BTreeMap<String, PendingClientRequest>,
    pending_daemon_peer_requests: BTreeMap<String, PendingDaemonPeerRequest>,
    subscriptions: BTreeMap<String, ActiveSubscription>,
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

    fn live_machines_in_realm(&self, realm_id: &str) -> Vec<RelayMachinePresence> {
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

    fn live_kernels_for_machine_in_realm(
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

    fn live_kernel_in_realm(
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

#[derive(Debug)]
pub struct RelayServer {
    config: RelayConfig,
    registry: Arc<RwLock<RelayRegistry>>,
    relay_request_counter: Arc<AtomicU64>,
    auth_verifier: RelayAuthVerifier,
}

impl RelayServer {
    pub fn new(config: RelayConfig) -> Self {
        Self::with_auth_verifier(
            config.clone(),
            RelayAuthVerifier::shared(config.shared_token.clone()),
        )
    }

    pub fn with_auth_verifier(config: RelayConfig, auth_verifier: RelayAuthVerifier) -> Self {
        Self {
            auth_verifier,
            config,
            registry: Arc::new(RwLock::new(RelayRegistry::default())),
            relay_request_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn config(&self) -> &RelayConfig {
        &self.config
    }

    pub fn registry(&self) -> Arc<RwLock<RelayRegistry>> {
        Arc::clone(&self.registry)
    }

    pub async fn bind_listener(&self) -> Result<TcpListener, std::io::Error> {
        TcpListener::bind((self.config.host.as_str(), self.config.port)).await
    }

    pub async fn run_until<F>(&self, shutdown: F) -> Result<(), std::io::Error>
    where
        F: Future<Output = ()>,
    {
        let listener = self.bind_listener().await?;
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accept = listener.accept() => {
                    let (stream, peer_addr) = accept?;
                    let registry = Arc::clone(&self.registry);
                    let auth_verifier = self.auth_verifier.clone();
                    let relay_request_counter = Arc::clone(&self.relay_request_counter);
                    tokio::spawn(async move {
                        let _ = handle_connection(stream, peer_addr, registry, auth_verifier, relay_request_counter).await;
                    });
                }
            }
        }
        Ok(())
    }

    pub async fn run(&self) -> Result<(), std::io::Error> {
        self.run_until(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer_addr: SocketAddr,
    registry: Arc<RwLock<RelayRegistry>>,
    auth_verifier: RelayAuthVerifier,
    relay_request_counter: Arc<AtomicU64>,
) -> Result<(), std::io::Error> {
    let socket = accept_async(stream)
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let (mut writer, mut reader) = socket.split();
    let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<Message>();
    let writer_task = tokio::spawn(async move {
        while let Some(message) = outgoing_rx.recv().await {
            if writer.send(message).await.is_err() {
                break;
            }
        }
    });
    let mut registered_daemon_key: Option<DaemonKey> = None;
    let token_expiry_generation = Arc::new(AtomicU64::new(0));

    while let Some(message) = reader.next().await {
        let message = message.map_err(|error| std::io::Error::other(error.to_string()))?;
        match message {
            Message::Text(text) => {
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
                        schedule_token_expiry_close(
                            &outgoing_tx,
                            &token_expiry_generation,
                            identity.expires_at_ms,
                        );
                        let daemon_key = DaemonKey::new(
                            identity.realm_id.clone(),
                            registration.daemon_id.clone(),
                        );
                        registered_daemon_key = Some(daemon_key.clone());
                        let mut replaced_senders = Vec::new();
                        let mut guard = registry.write().await;
                        guard.peers.retain(|_, peer| {
                            let replace = peer.role == RelayConnectionRole::Daemon
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
                        guard.daemons.insert(daemon_key, registration);
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
                            schedule_token_expiry_close(
                                &outgoing_tx,
                                &token_expiry_generation,
                                identity.expires_at_ms,
                            );
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
                        schedule_token_expiry_close(
                            &outgoing_tx,
                            &token_expiry_generation,
                            identity.expires_at_ms,
                        );
                        let Some(daemon_key) =
                            resolve_target_daemon_key(&registry, &identity.realm_id, &target).await
                        else {
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
                        let realm_id = peer_realm_id(&registry, peer_addr).await;
                        let Some(daemon_key) =
                            resolve_target_daemon_key(&registry, &realm_id, &target).await
                        else {
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
                        resume_from_event_id,
                    } => {
                        let realm_id = peer_realm_id(&registry, peer_addr).await;
                        let Some(daemon_key) =
                            resolve_target_daemon_key(&registry, &realm_id, &target).await
                        else {
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
                                    kind: PendingRequestKind::Subscribe {
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
                            &RelayEnvelope::DaemonSubscribe {
                                relay_request_id,
                                relay_subscription_id: subscription_id,
                                caller_identity: peer_identity(&registry, peer_addr).await,
                                session_id,
                                attachment_id,
                                client_public_key,
                                resume_from_event_id,
                            },
                        )?;
                    }
                    RelayEnvelope::ClientUnsubscribe {
                        request_id,
                        subscription_id,
                        client_public_key,
                    } => {
                        let relay_request_id = format!(
                            "relay-request-{}",
                            relay_request_counter.fetch_add(1, Ordering::Relaxed) + 1
                        );
                        let daemon_sender = {
                            let mut guard = registry.write().await;
                            let Some(active) = guard.subscriptions.get(&subscription_id).cloned()
                            else {
                                drop(guard);
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
                            guard.pending_requests.insert(
                                relay_request_id.clone(),
                                PendingClientRequest {
                                    client_addr: peer_addr,
                                    client_request_id: request_id.clone(),
                                    daemon_key: active.daemon_key.clone(),
                                    kind: PendingRequestKind::Unsubscribe {
                                        subscription_id: subscription_id.clone(),
                                    },
                                },
                            );
                            resolve_daemon_sender_locked(&guard, &active.daemon_key)
                        };
                        let Some(daemon_sender) = daemon_sender else {
                            registry
                                .write()
                                .await
                                .pending_requests
                                .remove(&relay_request_id);
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
                        let client_target = {
                            let mut guard = registry.write().await;
                            let pending = guard.pending_requests.remove(&relay_request_id);
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
                                guard
                                    .peers
                                    .get(&pending.client_addr)
                                    .map(|peer| (peer.sender.clone(), pending.client_request_id))
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
                        let daemon_target = {
                            let mut guard = registry.write().await;
                            let pending =
                                guard.pending_daemon_peer_requests.remove(&relay_request_id);
                            pending.and_then(|pending| {
                                resolve_daemon_sender_locked(&guard, &pending.requester_daemon_key)
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
                    RelayEnvelope::DaemonEvent {
                        subscription_id,
                        event_id,
                        encrypted_event,
                    } => {
                        let client_sender = {
                            let guard = registry.read().await;
                            guard
                                .subscriptions
                                .get(&subscription_id)
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
                        let _ = outgoing_tx.send(Message::Close(None));
                        break;
                    }
                    RelayEnvelope::ClientConnected { .. }
                    | RelayEnvelope::ClientMetadataResponse { .. }
                    | RelayEnvelope::DaemonPeerResponse { .. }
                    | RelayEnvelope::DaemonIncomingPeerRequest { .. }
                    | RelayEnvelope::DaemonIncomingPeerEvent { .. }
                    | RelayEnvelope::ClientResponse { .. }
                    | RelayEnvelope::DaemonRequest { .. }
                    | RelayEnvelope::DaemonSubscribe { .. }
                    | RelayEnvelope::DaemonUnsubscribe { .. }
                    | RelayEnvelope::ClientEvent { .. } => {}
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    let (disconnect_errors, disconnect_peer_errors) =
        remove_peer(&registry, peer_addr, registered_daemon_key.as_ref()).await;
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
    writer_task.abort();
    Ok(())
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

async fn peer_realm_id(registry: &Arc<RwLock<RelayRegistry>>, peer_addr: SocketAddr) -> String {
    registry
        .read()
        .await
        .peers
        .get(&peer_addr)
        .and_then(|peer| peer.realm_id.clone())
        .unwrap_or_else(|| DEFAULT_RELAY_REALM_ID.to_string())
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
) -> Option<mpsc::UnboundedSender<Message>> {
    let registration = registry.daemons.get(daemon_key)?;
    registry
        .peers
        .values()
        .find(|peer| {
            peer.role == RelayConnectionRole::Daemon
                && peer.realm_id.as_deref() == Some(daemon_key.realm_id.as_str())
                && peer
                    .daemon_registration
                    .as_ref()
                    .map(|candidate| candidate.daemon_id.as_str())
                    == Some(registration.daemon_id.as_str())
        })
        .map(|peer| peer.sender.clone())
}

async fn remove_peer(
    registry: &Arc<RwLock<RelayRegistry>>,
    peer_addr: SocketAddr,
    daemon_key: Option<&DaemonKey>,
) -> (
    Vec<(mpsc::UnboundedSender<Message>, String)>,
    Vec<(mpsc::UnboundedSender<Message>, String, String)>,
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
            return (Vec::new(), Vec::new());
        }
        guard.daemons.remove(daemon_key);
        let daemon_subscription_ids = guard
            .subscriptions
            .iter()
            .filter(|(_, active)| &active.daemon_key == daemon_key)
            .map(|(subscription_id, _)| subscription_id.clone())
            .collect::<Vec<_>>();
        for subscription_id in daemon_subscription_ids {
            guard.subscriptions.remove(&subscription_id);
        }
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
        return (client_errors, daemon_errors);
    }
    (Vec::new(), Vec::new())
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

fn send_envelope(
    sender: &mpsc::UnboundedSender<Message>,
    envelope: &RelayEnvelope,
) -> Result<(), std::io::Error> {
    let payload = serde_json::to_string(envelope)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    sender
        .send(Message::Text(payload.into()))
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::BrokenPipe, error.to_string()))
}

fn send_close(sender: &mpsc::UnboundedSender<Message>, reason: String) {
    let _ = send_envelope(sender, &RelayEnvelope::Close { reason });
}

fn schedule_token_expiry_close(
    sender: &mpsc::UnboundedSender<Message>,
    generation: &Arc<AtomicU64>,
    expires_at_ms: u64,
) {
    if expires_at_ms == u64::MAX {
        return;
    }
    let generation_id = generation.fetch_add(1, Ordering::SeqCst) + 1;
    let sender = sender.clone();
    let generation = Arc::clone(generation);
    let now_ms = current_unix_ms();
    let delay = Duration::from_millis(expires_at_ms.saturating_sub(now_ms));
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        if generation.load(Ordering::SeqCst) == generation_id {
            send_close(&sender, "relay token expired".to_string());
        }
    });
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn relay_error(code: &str, message: &str, retryable: bool) -> RelayError {
    RelayError {
        code: code.to_string(),
        message: message.to_string(),
        retryable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::auth::{RelaySubjectKind, RelayTokenClaims, ScopedTokenVerifier};
    use crate::protocol::EncryptedRelayPayload;
    use tokio::time::{sleep, Duration};
    use tokio_tungstenite::connect_async;

    fn test_registration(
        daemon_id: &str,
        machine_id: &str,
        os_name: &str,
        kernel_started_at_ms: u64,
    ) -> DaemonRegistration {
        test_registration_with_token(
            daemon_id,
            machine_id,
            os_name,
            kernel_started_at_ms,
            "secret",
        )
    }

    fn test_registration_with_token(
        daemon_id: &str,
        machine_id: &str,
        os_name: &str,
        kernel_started_at_ms: u64,
        auth_token: &str,
    ) -> DaemonRegistration {
        DaemonRegistration {
            auth_token: auth_token.to_string(),
            daemon_id: daemon_id.to_string(),
            machine_id: machine_id.to_string(),
            machine_alias: None,
            os_name: Some(os_name.to_string()),
            kernel_started_at_ms,
            daemon_alias: None,
            kernel_alias: None,
            public_key: format!("public-key-{daemon_id}"),
            capabilities: vec!["kernel_ws".to_string()],
            available_providers: vec!["codex".to_string()],
            accepting_remote_leases: true,
            leased_agent_count: 0,
            local_session_count: 0,
        }
    }

    fn scoped_claim(
        token_id: &str,
        subject: &str,
        subject_kind: RelaySubjectKind,
        realm_id: &str,
        actions: Vec<RelayAction>,
        targets: Option<Vec<&str>>,
    ) -> RelayTokenClaims {
        RelayTokenClaims {
            issuer: "test-issuer".to_string(),
            subject: subject.to_string(),
            subject_kind,
            realm_id: realm_id.to_string(),
            allowed_actions: actions,
            allowed_targets: targets.map(|values| {
                values
                    .into_iter()
                    .map(std::string::ToString::to_string)
                    .collect()
            }),
            issued_at_ms: 1,
            expires_at_ms: 100,
            token_id: token_id.to_string(),
            account_id: None,
            organization_id: None,
            user_id: None,
            device_id: None,
            machine_id: None,
            client_id: None,
            public_key_thumbprint: None,
            entitlements_version: None,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn server_binds_listener() {
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: None,
        });
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let local_addr = listener
            .local_addr()
            .expect("listener should have local addr");
        assert_eq!(local_addr.ip().to_string(), "127.0.0.1");
    }

    #[test]
    fn relay_aliases_differentiate_kernels_on_same_machine() {
        let mut registry = RelayRegistry::default();
        registry.daemons.insert(
            DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-a"),
            test_registration("daemon-a", "shared-machine", "macOS", 10),
        );
        registry.daemons.insert(
            DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-b"),
            test_registration("daemon-b", "shared-machine", "macOS", 20),
        );

        let machines = registry.live_machines();
        assert_eq!(machines.len(), 1);
        assert_eq!(machines[0].machine_id, "shared-machine");
        assert_eq!(machines[0].kernel_count, 2);
        assert_eq!(
            machines[0].machine_alias.as_deref(),
            Some("machine 1 (macOS)")
        );

        let kernels = registry.live_kernels_for_machine("shared-machine");
        assert_eq!(kernels.len(), 2);
        assert_eq!(kernels[0].kernel_id, "daemon-a");
        assert_eq!(kernels[0].relay_alias.as_deref(), Some("machine 1 (macOS)"));
        assert_eq!(kernels[1].kernel_id, "daemon-b");
        assert_eq!(kernels[1].relay_alias.as_deref(), Some("machine 2 (macOS)"));

        let exact = registry.live_kernels_for_machine("machine 2 (macOS)");
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].kernel_id, "daemon-b");
        assert_eq!(
            registry
                .live_kernel("machine 2 (macOS)")
                .expect("relay alias should resolve to a kernel")
                .kernel_id,
            "daemon-b"
        );
    }

    #[test]
    fn relay_registry_scopes_metadata_and_aliases_by_realm() {
        let mut registry = RelayRegistry::default();
        let mut realm_a = test_registration("daemon-a", "machine-a", "Linux", 10);
        realm_a.daemon_alias = Some("shared".to_string());
        realm_a.public_key = "public-key-a".to_string();
        let mut realm_b = test_registration("daemon-b", "machine-b", "Linux", 10);
        realm_b.daemon_alias = Some("shared".to_string());
        realm_b.public_key = "public-key-b".to_string();

        registry
            .daemons
            .insert(DaemonKey::new("realm-a", "daemon-a"), realm_a);
        registry
            .daemons
            .insert(DaemonKey::new("realm-b", "daemon-b"), realm_b);

        assert_eq!(registry.daemon_count(), 2);
        assert_eq!(registry.live_machines_in_realm("realm-a").len(), 1);
        assert_eq!(
            registry.live_machines_in_realm("realm-a")[0].machine_id,
            "machine-a"
        );
        assert_eq!(registry.live_machines_in_realm("realm-b").len(), 1);
        assert_eq!(
            registry.live_machines_in_realm("realm-b")[0].machine_id,
            "machine-b"
        );

        assert_eq!(
            registry
                .live_kernel_in_realm("realm-a", "shared")
                .expect("realm A alias should resolve")
                .public_key,
            "public-key-a"
        );
        assert_eq!(
            registry
                .live_kernel_in_realm("realm-b", "shared")
                .expect("realm B alias should resolve")
                .public_key,
            "public-key-b"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_registration_is_tracked_and_removed_on_disconnect() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_until(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut socket, _) = connect_async(&url)
            .await
            .expect("client should connect to relay");
        let register = RelayEnvelope::DaemonRegister {
            registration: DaemonRegistration {
                auth_token: "secret".to_string(),
                daemon_id: "daemon-1".to_string(),
                machine_id: "machine-1".to_string(),
                machine_alias: Some("workstation".to_string()),
                os_name: Some("macOS".to_string()),
                kernel_started_at_ms: 10,
                daemon_alias: Some("mbp".to_string()),
                kernel_alias: Some("default".to_string()),
                public_key: "public-key".to_string(),
                capabilities: vec!["kernel_ws".to_string()],
                available_providers: vec!["opencode".to_string()],
                accepting_remote_leases: false,
                leased_agent_count: 0,
                local_session_count: 1,
            },
        };
        socket
            .send(Message::Text(
                serde_json::to_string(&register)
                    .expect("register envelope should serialize")
                    .into(),
            ))
            .await
            .expect("register frame should send");
        sleep(Duration::from_millis(50)).await;

        {
            let guard = registry.read().await;
            assert_eq!(guard.daemon_count(), 1);
            assert!(guard.daemon("daemon-1").is_some());
        }

        socket.close(None).await.expect("socket should close");
        sleep(Duration::from_millis(50)).await;

        {
            let guard = registry.read().await;
            assert_eq!(guard.daemon_count(), 0);
        }

        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reconnecting_daemon_replaces_stale_socket_without_removing_live_registration() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_until(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut first_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("first daemon should connect to relay");
        let register = RelayEnvelope::DaemonRegister {
            registration: test_registration("daemon-1", "machine-1", "macOS", 10),
        };
        first_socket
            .send(Message::Text(
                serde_json::to_string(&register)
                    .expect("register envelope should serialize")
                    .into(),
            ))
            .await
            .expect("first register frame should send");
        sleep(Duration::from_millis(50)).await;

        let (mut second_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("second daemon should connect to relay");
        second_socket
            .send(Message::Text(
                serde_json::to_string(&register)
                    .expect("register envelope should serialize")
                    .into(),
            ))
            .await
            .expect("second register frame should send");
        sleep(Duration::from_millis(50)).await;

        {
            let guard = registry.read().await;
            assert_eq!(guard.daemon_count(), 1);
            assert_eq!(guard.peer_count(), 1);
            assert!(guard.daemon("daemon-1").is_some());
        }

        first_socket
            .close(None)
            .await
            .expect("first socket should close");
        sleep(Duration::from_millis(50)).await;

        {
            let guard = registry.read().await;
            assert_eq!(guard.daemon_count(), 1);
            assert_eq!(guard.peer_count(), 1);
            assert!(guard.daemon("daemon-1").is_some());
        }

        second_socket
            .close(None)
            .await
            .expect("second socket should close");
        sleep(Duration::from_millis(50)).await;

        {
            let guard = registry.read().await;
            assert_eq!(guard.daemon_count(), 0);
            assert_eq!(guard.peer_count(), 0);
        }

        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    async fn connect_async_with_retry(
        url: &str,
    ) -> Result<
        (
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            tokio_tungstenite::tungstenite::handshake::client::Response,
        ),
        tokio_tungstenite::tungstenite::Error,
    > {
        let mut last_error = None;
        for _ in 0..20 {
            match connect_async(url).await {
                Ok(socket) => return Ok(socket),
                Err(error) => {
                    last_error = Some(error);
                    sleep(Duration::from_millis(10)).await;
                }
            }
        }
        Err(last_error.expect("connect retry should record an error"))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metadata_queries_return_live_machines_and_kernels() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_until(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async(&url)
            .await
            .expect("daemon should connect to relay");
        let register = RelayEnvelope::DaemonRegister {
            registration: DaemonRegistration {
                auth_token: "secret".to_string(),
                daemon_id: "daemon-1".to_string(),
                machine_id: "machine-1".to_string(),
                machine_alias: Some("workstation".to_string()),
                os_name: Some("macOS".to_string()),
                kernel_started_at_ms: 10,
                daemon_alias: Some("mbp".to_string()),
                kernel_alias: Some("default".to_string()),
                public_key: "public-key".to_string(),
                capabilities: vec!["kernel_ws".to_string()],
                available_providers: vec!["opencode".to_string(), "codex".to_string()],
                accepting_remote_leases: true,
                leased_agent_count: 2,
                local_session_count: 3,
            },
        };
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&register)
                    .expect("register envelope should serialize")
                    .into(),
            ))
            .await
            .expect("register frame should send");

        let (mut client_socket, _) = connect_async(&url)
            .await
            .expect("client should connect to relay");
        let machines_request = RelayEnvelope::ClientMetadataRequest {
            request_id: "machines-1".to_string(),
            auth_token: "secret".to_string(),
            query: RelayMetadataQuery::ListLiveMachines,
        };
        client_socket
            .send(Message::Text(
                serde_json::to_string(&machines_request)
                    .expect("machines request should serialize")
                    .into(),
            ))
            .await
            .expect("machines request should send");
        let machines_payload = match client_socket.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected machines response: {other:?}"),
        };
        let machines_response: RelayEnvelope =
            serde_json::from_str(&machines_payload).expect("machines response should decode");
        match machines_response {
            RelayEnvelope::ClientMetadataResponse {
                request_id,
                machines: Some(machines),
                kernels: None,
                kernel: None,
                error: None,
            } => {
                assert_eq!(request_id, "machines-1");
                assert_eq!(machines.len(), 1);
                assert_eq!(machines[0].machine_id, "machine-1");
                assert_eq!(
                    machines[0].machine_alias.as_deref(),
                    Some("machine 1 (macOS)")
                );
                assert_eq!(machines[0].available_providers, vec!["codex", "opencode"]);
            }
            other => panic!("unexpected machines response envelope: {other:?}"),
        }

        let kernels_request = RelayEnvelope::ClientMetadataRequest {
            request_id: "kernels-1".to_string(),
            auth_token: "secret".to_string(),
            query: RelayMetadataQuery::ListLiveKernelsForMachine {
                machine_ref: "machine 1 (macOS)".to_string(),
            },
        };
        client_socket
            .send(Message::Text(
                serde_json::to_string(&kernels_request)
                    .expect("kernels request should serialize")
                    .into(),
            ))
            .await
            .expect("kernels request should send");
        let kernels_payload = match client_socket.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected kernels response: {other:?}"),
        };
        let kernels_response: RelayEnvelope =
            serde_json::from_str(&kernels_payload).expect("kernels response should decode");
        match kernels_response {
            RelayEnvelope::ClientMetadataResponse {
                request_id,
                machines: None,
                kernels: Some(kernels),
                kernel: None,
                error: None,
            } => {
                assert_eq!(request_id, "kernels-1");
                assert_eq!(kernels.len(), 1);
                assert_eq!(kernels[0].kernel_id, "daemon-1");
                assert_eq!(
                    kernels[0].machine_alias.as_deref(),
                    Some("machine 1 (macOS)")
                );
                assert_eq!(kernels[0].relay_alias.as_deref(), Some("machine 1 (macOS)"));
                assert_eq!(kernels[0].available_providers, vec!["opencode", "codex"]);
                assert!(kernels[0].accepting_remote_leases);
                assert_eq!(kernels[0].leased_agent_count, 2);
                assert_eq!(kernels[0].local_session_count, 3);
            }
            other => panic!("unexpected kernels response envelope: {other:?}"),
        }

        let kernel_request = RelayEnvelope::ClientMetadataRequest {
            request_id: "kernel-1".to_string(),
            auth_token: "secret".to_string(),
            query: RelayMetadataQuery::GetLiveKernel {
                kernel_ref: "default".to_string(),
            },
        };
        client_socket
            .send(Message::Text(
                serde_json::to_string(&kernel_request)
                    .expect("kernel request should serialize")
                    .into(),
            ))
            .await
            .expect("kernel request should send");
        let kernel_payload = match client_socket.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected kernel response: {other:?}"),
        };
        let kernel_response: RelayEnvelope =
            serde_json::from_str(&kernel_payload).expect("kernel response should decode");
        match kernel_response {
            RelayEnvelope::ClientMetadataResponse {
                request_id,
                machines: None,
                kernels: None,
                kernel: Some(kernel),
                error: None,
            } => {
                assert_eq!(request_id, "kernel-1");
                assert_eq!(kernel.kernel_id, "daemon-1");
                assert_eq!(kernel.public_key, "public-key");
            }
            other => panic!("unexpected kernel response envelope: {other:?}"),
        }

        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scoped_tokens_route_and_list_only_within_their_realm() {
        let mut claims = BTreeMap::new();
        claims.insert(
            "daemon-a-token".to_string(),
            scoped_claim(
                "daemon-a-token",
                "daemon-a",
                RelaySubjectKind::Kernel,
                "realm-a",
                vec![RelayAction::DaemonRegister],
                None,
            ),
        );
        claims.insert(
            "daemon-b-token".to_string(),
            scoped_claim(
                "daemon-b-token",
                "daemon-b",
                RelaySubjectKind::Kernel,
                "realm-b",
                vec![RelayAction::DaemonRegister],
                None,
            ),
        );
        claims.insert(
            "client-a-token".to_string(),
            scoped_claim(
                "client-a-token",
                "client-a",
                RelaySubjectKind::Client,
                "realm-a",
                vec![RelayAction::ClientConnect, RelayAction::ClientMetadataRead],
                None,
            ),
        );
        claims.insert(
            "client-b-token".to_string(),
            scoped_claim(
                "client-b-token",
                "client-b",
                RelaySubjectKind::Client,
                "realm-b",
                vec![RelayAction::ClientConnect, RelayAction::ClientMetadataRead],
                None,
            ),
        );
        let auth_verifier = RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(
            claims,
            BTreeMap::new(),
            Some(10),
        ));
        let server = RelayServer::with_auth_verifier(
            RelayConfig {
                host: "127.0.0.1".to_string(),
                port: 0,
                shared_token: None,
            },
            auth_verifier,
        );
        let listener = server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        drop(listener);

        let auth_verifier = server.auth_verifier.clone();
        let server = RelayServer::with_auth_verifier(
            RelayConfig {
                host: addr.ip().to_string(),
                port: addr.port(),
                shared_token: None,
            },
            auth_verifier,
        );
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_until(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_a, _) = connect_async(&url).await.expect("daemon A should connect");
        let (mut daemon_b, _) = connect_async(&url).await.expect("daemon B should connect");
        let mut registration_a =
            test_registration_with_token("daemon-a", "machine-a", "Linux", 10, "daemon-a-token");
        registration_a.daemon_alias = Some("shared".to_string());
        registration_a.public_key = "public-key-a".to_string();
        let mut registration_b =
            test_registration_with_token("daemon-b", "machine-b", "Linux", 10, "daemon-b-token");
        registration_b.daemon_alias = Some("shared".to_string());
        registration_b.public_key = "public-key-b".to_string();
        for (socket, registration) in [
            (&mut daemon_a, registration_a),
            (&mut daemon_b, registration_b),
        ] {
            socket
                .send(Message::Text(
                    serde_json::to_string(&RelayEnvelope::DaemonRegister { registration })
                        .expect("register should serialize")
                        .into(),
                ))
                .await
                .expect("register should send");
        }

        let (mut client_a, _) = connect_async(&url).await.expect("client A should connect");
        client_a
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientMetadataRequest {
                    request_id: "machines-a".to_string(),
                    auth_token: "client-a-token".to_string(),
                    query: RelayMetadataQuery::ListLiveMachines,
                })
                .expect("metadata request should serialize")
                .into(),
            ))
            .await
            .expect("metadata request should send");
        let machines_payload = match client_a.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected machines response: {other:?}"),
        };
        match serde_json::from_str::<RelayEnvelope>(&machines_payload)
            .expect("machines response should decode")
        {
            RelayEnvelope::ClientMetadataResponse {
                machines: Some(machines),
                error: None,
                ..
            } => {
                assert_eq!(machines.len(), 1);
                assert_eq!(machines[0].machine_id, "machine-a");
            }
            other => panic!("unexpected machines response envelope: {other:?}"),
        }

        client_a
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "client-a-token".to_string(),
                    target: ClientTarget {
                        daemon_id: None,
                        daemon_alias: Some("shared".to_string()),
                    },
                })
                .expect("client connect should serialize")
                .into(),
            ))
            .await
            .expect("client connect should send");
        let connect_payload = match client_a.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected connect response: {other:?}"),
        };
        match serde_json::from_str::<RelayEnvelope>(&connect_payload)
            .expect("connect response should decode")
        {
            RelayEnvelope::ClientConnected {
                daemon_public_key, ..
            } => assert_eq!(daemon_public_key, "public-key-a"),
            other => panic!("unexpected connect response envelope: {other:?}"),
        }

        let (mut client_b, _) = connect_async(&url).await.expect("client B should connect");
        client_b
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "client-b-token".to_string(),
                    target: ClientTarget {
                        daemon_id: None,
                        daemon_alias: Some("shared".to_string()),
                    },
                })
                .expect("client connect should serialize")
                .into(),
            ))
            .await
            .expect("client connect should send");
        let connect_payload = match client_b.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected connect response: {other:?}"),
        };
        match serde_json::from_str::<RelayEnvelope>(&connect_payload)
            .expect("connect response should decode")
        {
            RelayEnvelope::ClientConnected {
                daemon_public_key, ..
            } => assert_eq!(daemon_public_key, "public-key-b"),
            other => panic!("unexpected connect response envelope: {other:?}"),
        }

        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_peer_requests_are_routed_between_registered_kernels() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_until(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_a, _) = connect_async(&url)
            .await
            .expect("daemon A should connect to relay");
        let (mut daemon_b, _) = connect_async(&url)
            .await
            .expect("daemon B should connect to relay");

        for (socket, daemon_id, daemon_alias, public_key) in [
            (&mut daemon_a, "daemon-a", "alpha", "public-key-a"),
            (&mut daemon_b, "daemon-b", "beta", "public-key-b"),
        ] {
            let register = RelayEnvelope::DaemonRegister {
                registration: DaemonRegistration {
                    auth_token: "secret".to_string(),
                    daemon_id: daemon_id.to_string(),
                    machine_id: format!("machine-{daemon_id}"),
                    machine_alias: None,
                    os_name: Some("Linux".to_string()),
                    kernel_started_at_ms: 10,
                    daemon_alias: Some(daemon_alias.to_string()),
                    kernel_alias: Some(daemon_alias.to_string()),
                    public_key: public_key.to_string(),
                    capabilities: vec!["kernel_ws".to_string()],
                    available_providers: vec!["opencode".to_string()],
                    accepting_remote_leases: true,
                    leased_agent_count: 0,
                    local_session_count: 0,
                },
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&register)
                        .expect("register envelope should serialize")
                        .into(),
                ))
                .await
                .expect("register frame should send");
        }
        sleep(Duration::from_millis(50)).await;

        let encrypted_request = EncryptedRelayPayload {
            sender_public_key: "client-public".to_string(),
            nonce: "nonce".to_string(),
            ciphertext: "ciphertext".to_string(),
        };
        daemon_a
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonPeerRequest {
                    request_id: "peer-1".to_string(),
                    target: ClientTarget {
                        daemon_id: None,
                        daemon_alias: Some("beta".to_string()),
                    },
                    encrypted_request: encrypted_request.clone(),
                })
                .expect("peer request should serialize")
                .into(),
            ))
            .await
            .expect("peer request should send");

        let incoming_payload = match daemon_b.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected incoming peer request: {other:?}"),
        };
        let relay_request_id = match serde_json::from_str::<RelayEnvelope>(&incoming_payload)
            .expect("incoming peer request should decode")
        {
            RelayEnvelope::DaemonIncomingPeerRequest {
                relay_request_id,
                from_daemon_id,
                caller_identity,
                encrypted_request: forwarded,
            } => {
                assert_eq!(from_daemon_id, "daemon-a");
                assert_eq!(
                    caller_identity
                        .as_ref()
                        .map(|identity| identity.subject.as_str()),
                    Some("shared-token-bootstrap")
                );
                assert_eq!(forwarded, encrypted_request);
                relay_request_id
            }
            other => panic!("unexpected incoming peer envelope: {other:?}"),
        };

        let encrypted_response = EncryptedRelayPayload {
            sender_public_key: "daemon-b-public".to_string(),
            nonce: "nonce-2".to_string(),
            ciphertext: "ciphertext-2".to_string(),
        };
        daemon_b
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonIncomingPeerResponse {
                    relay_request_id,
                    encrypted_response: Some(encrypted_response.clone()),
                    error: None,
                })
                .expect("peer response should serialize")
                .into(),
            ))
            .await
            .expect("peer response should send");

        let response_payload = match daemon_a.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected routed peer response: {other:?}"),
        };
        match serde_json::from_str::<RelayEnvelope>(&response_payload)
            .expect("routed peer response should decode")
        {
            RelayEnvelope::DaemonPeerResponse {
                request_id,
                from_daemon_id,
                encrypted_response: Some(forwarded),
                error: None,
            } => {
                assert_eq!(request_id, "peer-1");
                assert_eq!(from_daemon_id, "daemon-b");
                assert_eq!(forwarded, encrypted_response);
            }
            other => panic!("unexpected routed peer response envelope: {other:?}"),
        }

        let _ = daemon_a.close(None).await;
        let _ = daemon_b.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }
}
