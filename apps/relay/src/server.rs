use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use crate::config::RelayConfig;
use crate::protocol::{
    ClientTarget, DaemonRegistration, RelayConnectionRole, RelayEnvelope, RelayError,
};

#[derive(Debug, Clone)]
struct PeerHandle {
    sender: mpsc::UnboundedSender<Message>,
    role: RelayConnectionRole,
    daemon_registration: Option<DaemonRegistration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedPeer {
    pub role: RelayConnectionRole,
    pub daemon_registration: Option<DaemonRegistration>,
}

#[derive(Debug, Clone)]
struct PendingClientRequest {
    client_addr: SocketAddr,
    client_request_id: String,
    daemon_id: String,
}

#[derive(Debug, Default)]
pub struct RelayRegistry {
    peers: BTreeMap<SocketAddr, PeerHandle>,
    daemons: BTreeMap<String, DaemonRegistration>,
    pending_requests: BTreeMap<String, PendingClientRequest>,
}

impl RelayRegistry {
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    pub fn daemon_count(&self) -> usize {
        self.daemons.len()
    }

    pub fn daemon(&self, daemon_id: &str) -> Option<&DaemonRegistration> {
        self.daemons.get(daemon_id)
    }

    pub fn pending_request_count(&self) -> usize {
        self.pending_requests.len()
    }

    pub fn connected_peer(&self, peer_addr: &SocketAddr) -> Option<ConnectedPeer> {
        self.peers.get(peer_addr).map(|peer| ConnectedPeer {
            role: peer.role.clone(),
            daemon_registration: peer.daemon_registration.clone(),
        })
    }
}

#[derive(Debug)]
pub struct RelayServer {
    config: RelayConfig,
    registry: Arc<RwLock<RelayRegistry>>,
    relay_request_counter: Arc<AtomicU64>,
}

impl RelayServer {
    pub fn new(config: RelayConfig) -> Self {
        Self {
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
                    let shared_token = self.config.shared_token.clone();
                    let relay_request_counter = Arc::clone(&self.relay_request_counter);
                    tokio::spawn(async move {
                        let _ = handle_connection(stream, peer_addr, registry, shared_token, relay_request_counter).await;
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
    shared_token: Option<String>,
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
    let mut registered_daemon_id: Option<String> = None;

    while let Some(message) = reader.next().await {
        let message = message.map_err(|error| std::io::Error::other(error.to_string()))?;
        match message {
            Message::Text(text) => {
                let envelope: RelayEnvelope = serde_json::from_str(&text).map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
                })?;
                match envelope {
                    RelayEnvelope::DaemonRegister { registration } => {
                        validate_shared_token(shared_token.as_deref(), &registration.auth_token)?;
                        registered_daemon_id = Some(registration.daemon_id.clone());
                        let mut guard = registry.write().await;
                        guard.peers.insert(
                            peer_addr,
                            PeerHandle {
                                sender: outgoing_tx.clone(),
                                role: RelayConnectionRole::Daemon,
                                daemon_registration: Some(registration.clone()),
                            },
                        );
                        guard
                            .daemons
                            .insert(registration.daemon_id.clone(), registration);
                    }
                    RelayEnvelope::DaemonHeartbeat { daemon_id } => {
                        if registered_daemon_id.as_deref() != Some(daemon_id.as_str()) {
                            break;
                        }
                    }
                    RelayEnvelope::ClientConnect { auth_token, target } => {
                        validate_shared_token(shared_token.as_deref(), &auth_token)?;
                        let Some(daemon_id) = resolve_target_daemon_id(&registry, &target).await
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
                                .get(&daemon_id)
                                .map(|registration| registration.public_key.clone())
                        };
                        let mut guard = registry.write().await;
                        guard.peers.insert(
                            peer_addr,
                            PeerHandle {
                                sender: outgoing_tx.clone(),
                                role: RelayConnectionRole::Client,
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
                    RelayEnvelope::ClientRequest {
                        request_id,
                        target,
                        encrypted_request,
                    } => {
                        let Some(daemon_id) = resolve_target_daemon_id(&registry, &target).await
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
                                    daemon_id: daemon_id.clone(),
                                },
                            );
                            resolve_daemon_sender_locked(&guard, &daemon_id)
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
                                encrypted_request,
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
                    RelayEnvelope::Close { .. } => {
                        let _ = outgoing_tx.send(Message::Close(None));
                        break;
                    }
                    RelayEnvelope::ClientConnected { .. }
                    | RelayEnvelope::ClientResponse { .. }
                    | RelayEnvelope::DaemonRequest { .. }
                    | RelayEnvelope::ClientSubscribe { .. }
                    | RelayEnvelope::ClientUnsubscribe { .. }
                    | RelayEnvelope::DaemonEvent { .. } => {}
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    let disconnect_errors =
        remove_peer(&registry, peer_addr, registered_daemon_id.as_deref()).await;
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
    writer_task.abort();
    Ok(())
}

async fn resolve_target_daemon_id(
    registry: &Arc<RwLock<RelayRegistry>>,
    target: &ClientTarget,
) -> Option<String> {
    let guard = registry.read().await;
    if let Some(daemon_id) = target.daemon_id.as_ref() {
        return guard.daemons.get(daemon_id).map(|_| daemon_id.clone());
    }
    let alias = target.daemon_alias.as_ref()?;
    guard
        .daemons
        .iter()
        .find(|(_, registration)| registration.daemon_alias.as_ref() == Some(alias))
        .map(|(daemon_id, _)| daemon_id.clone())
}

fn resolve_daemon_sender_locked(
    registry: &RelayRegistry,
    daemon_id: &str,
) -> Option<mpsc::UnboundedSender<Message>> {
    let registration = registry.daemons.get(daemon_id)?;
    registry
        .peers
        .values()
        .find(|peer| {
            peer.role == RelayConnectionRole::Daemon
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
    daemon_id: Option<&str>,
) -> Vec<(mpsc::UnboundedSender<Message>, String)> {
    let mut guard = registry.write().await;
    guard.peers.remove(&peer_addr);
    if let Some(daemon_id) = daemon_id {
        guard.daemons.remove(daemon_id);
        let doomed_request_ids = guard
            .pending_requests
            .iter()
            .filter(|(_, pending)| pending.daemon_id == daemon_id)
            .map(|(relay_request_id, _)| relay_request_id.clone())
            .collect::<Vec<_>>();
        let mut errors = Vec::new();
        for relay_request_id in doomed_request_ids {
            if let Some(pending) = guard.pending_requests.remove(&relay_request_id) {
                if let Some(peer) = guard.peers.get(&pending.client_addr) {
                    errors.push((peer.sender.clone(), pending.client_request_id));
                }
            }
        }
        return errors;
    }
    Vec::new()
}

fn validate_shared_token(expected: Option<&str>, provided: &str) -> Result<(), std::io::Error> {
    if let Some(expected) = expected {
        if expected != provided {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "invalid relay token",
            ));
        }
    }
    Ok(())
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

    use tokio::time::{sleep, Duration};
    use tokio_tungstenite::connect_async;

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
                daemon_alias: Some("mbp".to_string()),
                public_key: "public-key".to_string(),
                capabilities: vec!["kernel_ws".to_string()],
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
}
