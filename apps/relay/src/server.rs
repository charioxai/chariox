use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

use crate::auth::{RelayAuthVerifier, RelayRevocationRegistry};
use crate::config::RelayConfig;

mod connection;
mod display;
mod health;
use connection::handle_connection;
use display::{handle_display_connection, is_display_http_request};
use health::{handle_health_connection, is_health_http_request};

pub use crate::registry::{ConnectedPeer, RelayRegistry};

#[derive(Debug)]
pub struct RelayServer {
    config: RelayConfig,
    registry: Arc<RwLock<RelayRegistry>>,
    relay_request_counter: Arc<AtomicU64>,
    auth_verifier: RelayAuthVerifier,
    revocations: RelayRevocationRegistry,
    draining: Arc<AtomicBool>,
}

impl RelayServer {
    pub fn new(config: RelayConfig) -> Self {
        Self::with_auth_verifier(
            config.clone(),
            RelayAuthVerifier::shared(config.shared_token.clone()),
        )
    }

    pub fn with_auth_verifier(config: RelayConfig, auth_verifier: RelayAuthVerifier) -> Self {
        // Every server carries a live revocation registry attached to the
        // verifier; it is empty (a no-op) until revocations are fed in, so
        // scoped-token verification can reject revoked tokens without any
        // additional wiring at the call sites.
        let revocations = RelayRevocationRegistry::new();
        Self {
            auth_verifier: auth_verifier.with_revocations(revocations.clone()),
            config,
            registry: Arc::new(RwLock::new(RelayRegistry::default())),
            relay_request_counter: Arc::new(AtomicU64::new(0)),
            revocations,
            draining: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The live revocation registry consulted during scoped-token verification.
    /// Feed it (`revoke_token_id`/`revoke_account`/`revoke_subject`/`prune`)
    /// from the hosted control plane's revocation state.
    pub fn revocations(&self) -> RelayRevocationRegistry {
        self.revocations.clone()
    }

    #[cfg(test)]
    pub(crate) fn auth_verifier(&self) -> &RelayAuthVerifier {
        &self.auth_verifier
    }

    pub fn config(&self) -> &RelayConfig {
        &self.config
    }

    pub fn registry(&self) -> Arc<RwLock<RelayRegistry>> {
        Arc::clone(&self.registry)
    }

    pub fn set_draining(&self, draining: bool) {
        self.draining.store(draining, Ordering::Relaxed);
    }

    pub async fn bind_listener(&self) -> Result<TcpListener, std::io::Error> {
        TcpListener::bind((self.config.host.as_str(), self.config.port)).await
    }

    pub async fn run_until<F>(&self, shutdown: F) -> Result<(), std::io::Error>
    where
        F: Future<Output = ()>,
    {
        let listener = self.bind_listener().await?;
        self.run_listener_until(listener, shutdown).await
    }

    pub async fn run_listener_until<F>(
        &self,
        listener: TcpListener,
        shutdown: F,
    ) -> Result<(), std::io::Error>
    where
        F: Future<Output = ()>,
    {
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accept = listener.accept() => {
                    let (stream, peer_addr) = accept?;
                    let registry = Arc::clone(&self.registry);
                    let auth_verifier = self.auth_verifier.clone();
                    let relay_request_counter = Arc::clone(&self.relay_request_counter);
                    let draining = Arc::clone(&self.draining);
                    tokio::spawn(async move {
                        if is_health_http_request(&stream).await {
                            let _ = handle_health_connection(stream, registry, draining).await;
                        } else if draining.load(Ordering::Relaxed) {
                            let _ = reject_draining_connection(stream).await;
                        } else if is_display_http_request(&stream).await {
                            let _ = handle_display_connection(stream, peer_addr, registry, relay_request_counter).await;
                        } else {
                            let _ = handle_connection(stream, peer_addr, registry, auth_verifier, relay_request_counter).await;
                        }
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

async fn reject_draining_connection(mut stream: TcpStream) -> Result<(), std::io::Error> {
    let body = r#"{"status":"draining","error":"relay is draining","retry_after_seconds":5}"#;
    let response = format!(
        "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\ncontent-length: {}\r\ncache-control: no-store\r\nretry-after: 5\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    let _ = stream.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests;
