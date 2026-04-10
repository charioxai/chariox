use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::RwLock;

use crate::config::RelayConfig;
use crate::protocol::{DaemonRegistration, RelayConnectionRole};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedPeer {
    pub role: RelayConnectionRole,
    pub daemon_registration: Option<DaemonRegistration>,
}

#[derive(Debug, Default)]
pub struct RelayRegistry {
    peers: BTreeMap<SocketAddr, ConnectedPeer>,
}

impl RelayRegistry {
    pub fn peer_count(&self) -> usize {
        self.peers.len()
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

    pub async fn run(&self) -> Result<(), std::io::Error> {
        let listener = self.bind_listener().await?;
        loop {
            let (_stream, _peer_addr) = listener.accept().await?;
            // M5 slice 1 intentionally stops at the scaffold and registry boundary.
            // Connection role negotiation and envelope forwarding land in follow-up slices.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn server_binds_listener() {
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
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
}
