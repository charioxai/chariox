use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_tungstenite::{accept_async, tungstenite::Message};

use crate::config::RelayConfig;
use crate::protocol::{DaemonRegistration, RelayConnectionRole, RelayEnvelope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedPeer {
    pub role: RelayConnectionRole,
    pub daemon_registration: Option<DaemonRegistration>,
}

#[derive(Debug, Default)]
pub struct RelayRegistry {
    peers: BTreeMap<SocketAddr, ConnectedPeer>,
    daemons: BTreeMap<String, DaemonRegistration>,
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
}

#[derive(Debug)]
pub struct RelayServer {
    config: RelayConfig,
    registry: Arc<RwLock<RelayRegistry>>,
}

impl RelayServer {
    pub fn new(config: RelayConfig) -> Self {
        Self {
            config,
            registry: Arc::new(RwLock::new(RelayRegistry::default())),
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
                _ = &mut shutdown => {
                    break;
                }
                accept = listener.accept() => {
                    let (stream, peer_addr) = accept?;
                    let registry = Arc::clone(&self.registry);
                    let shared_token = self.config.shared_token.clone();
                    tokio::spawn(async move {
                        let _ = handle_connection(stream, peer_addr, registry, shared_token).await;
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
) -> Result<(), std::io::Error> {
    let mut socket = accept_async(stream)
        .await
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error.to_string()))?;
    let mut registered_daemon_id: Option<String> = None;

    while let Some(message) = socket.next().await {
        let message = message
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error.to_string()))?;
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
                            ConnectedPeer {
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
                    RelayEnvelope::ClientConnect { auth_token, .. } => {
                        validate_shared_token(shared_token.as_deref(), &auth_token)?;
                        let mut guard = registry.write().await;
                        guard.peers.insert(
                            peer_addr,
                            ConnectedPeer {
                                role: RelayConnectionRole::Client,
                                daemon_registration: None,
                            },
                        );
                    }
                    RelayEnvelope::Close { reason } => {
                        let _ = socket.send(Message::Close(None)).await;
                        let _ = reason;
                        break;
                    }
                    _ => {}
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    let mut guard = registry.write().await;
    guard.peers.remove(&peer_addr);
    if let Some(daemon_id) = registered_daemon_id {
        guard.daemons.remove(&daemon_id);
    }
    Ok(())
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
