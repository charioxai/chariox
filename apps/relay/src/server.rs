use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

use crate::auth::RelayAuthVerifier;
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
        Self {
            auth_verifier,
            config,
            registry: Arc::new(RwLock::new(RelayRegistry::default())),
            relay_request_counter: Arc::new(AtomicU64::new(0)),
            draining: Arc::new(AtomicBool::new(false)),
        }
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
    let body = r#"{"status":"draining","error":"relay is draining"}"#;
    let response = format!(
        "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    let _ = stream.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::auth::{
        RelayAction, RelayAuthVerifier, RelaySubjectKind, RelayTokenClaims, ScopedTokenVerifier,
        DEFAULT_RELAY_REALM_ID,
    };
    use crate::protocol::{
        ClientTarget, DaemonRegistration, EncryptedRelayPayload, RelayDisplayTunnelHeader,
        RelayDisplayTunnelRegistration, RelayDisplayTunnelStreamChunk, RelayEnvelope,
        RelayMetadataQuery,
    };
    use crate::registry::DaemonKey;
    use base64::Engine as _;
    use futures_util::{SinkExt, StreamExt};
    use std::collections::BTreeMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::time::{sleep, timeout, Duration};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

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
            provider_accounts: Vec::new(),
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
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
                provider_accounts: Vec::new(),
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_socket_cannot_switch_registered_identity() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect to relay");
        let first_registration = test_registration("daemon-1", "machine-1", "macOS", 10);
        socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: first_registration,
                })
                .expect("first register should serialize")
                .into(),
            ))
            .await
            .expect("first register should send");
        sleep(Duration::from_millis(50)).await;

        let mut refreshed_registration = test_registration("daemon-1", "machine-1", "macOS", 20);
        refreshed_registration.public_key = "refreshed-public-key".to_string();
        socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: refreshed_registration,
                })
                .expect("refresh register should serialize")
                .into(),
            ))
            .await
            .expect("refresh register should send");
        sleep(Duration::from_millis(50)).await;
        assert_no_relay_close(&mut socket).await;

        {
            let guard = registry.read().await;
            assert_eq!(guard.daemon_count(), 1);
            assert_eq!(guard.peer_count(), 1);
            assert_eq!(
                guard
                    .daemon("daemon-1")
                    .map(|registration| registration.public_key.as_str()),
                Some("refreshed-public-key")
            );
        }

        socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration("daemon-2", "machine-1", "macOS", 30),
                })
                .expect("identity switch register should serialize")
                .into(),
            ))
            .await
            .expect("identity switch register should send");

        let close_payload = match timeout(Duration::from_millis(500), socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => text,
            Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => String::new().into(),
            Ok(other) => panic!("unexpected identity switch response: {other:?}"),
            Err(_) => panic!("identity switch did not close promptly"),
        };
        if !close_payload.is_empty() {
            match serde_json::from_str::<RelayEnvelope>(&close_payload)
                .expect("relay close should decode")
            {
                RelayEnvelope::Close { reason } => {
                    assert_eq!(reason, "daemon connection already registered");
                }
                other => panic!("unexpected identity switch envelope: {other:?}"),
            }
        }
        sleep(Duration::from_millis(50)).await;
        {
            let guard = registry.read().await;
            assert_eq!(guard.daemon_count(), 0);
            assert_eq!(guard.peer_count(), 0);
            assert!(guard.daemon("daemon-1").is_none());
            assert!(guard.daemon("daemon-2").is_none());
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
    async fn daemon_display_tunnel_registration_is_tracked_revoked_and_disconnected() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect to relay");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration("daemon-1", "machine-1", "macOS", 10),
                })
                .expect("register envelope should serialize")
                .into(),
            ))
            .await
            .expect("register should send");

        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRegister {
                    registration: RelayDisplayTunnelRegistration {
                        tunnel_id: "display-opaque-1".to_string(),
                        expires_at_ms: u64::MAX,
                        capabilities: vec!["view".to_string(), "keyboard".to_string()],
                    },
                })
                .expect("display tunnel register should serialize")
                .into(),
            ))
            .await
            .expect("display tunnel register should send");

        let registered_payload = match daemon_socket.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected display tunnel registration response: {other:?}"),
        };
        match serde_json::from_str::<RelayEnvelope>(&registered_payload)
            .expect("display tunnel registration response should decode")
        {
            RelayEnvelope::DaemonDisplayTunnelRegistered {
                tunnel_id,
                expires_at_ms,
                error: None,
            } => {
                assert_eq!(tunnel_id, "display-opaque-1");
                assert_eq!(expires_at_ms, u64::MAX);
            }
            other => panic!("unexpected display tunnel registration envelope: {other:?}"),
        }

        {
            let guard = registry.read().await;
            assert_eq!(guard.display_tunnel_count(), 1);
        }

        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRevoke {
                    tunnel_id: "display-opaque-1".to_string(),
                })
                .expect("display tunnel revoke should serialize")
                .into(),
            ))
            .await
            .expect("display tunnel revoke should send");
        sleep(Duration::from_millis(50)).await;
        assert_eq!(registry.read().await.display_tunnel_count(), 0);

        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRegister {
                    registration: RelayDisplayTunnelRegistration {
                        tunnel_id: "display-opaque-2".to_string(),
                        expires_at_ms: u64::MAX,
                        capabilities: vec!["view".to_string()],
                    },
                })
                .expect("display tunnel register should serialize")
                .into(),
            ))
            .await
            .expect("second display tunnel register should send");
        let _ = daemon_socket.next().await;
        assert_eq!(registry.read().await.display_tunnel_count(), 1);

        daemon_socket
            .close(None)
            .await
            .expect("daemon should close");
        sleep(Duration::from_millis(50)).await;
        assert_eq!(registry.read().await.display_tunnel_count(), 0);

        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn display_http_route_resolves_registered_tunnel_state() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let missing = relay_http_get(addr, "/display/missing/vnc.html").await;
        assert!(missing.starts_with("HTTP/1.1 404 Not Found"));

        {
            let mut guard = registry.write().await;
            guard.register_display_tunnel(
                DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-expired"),
                "expired".to_string(),
                1,
                Vec::new(),
            );
            guard.register_display_tunnel(
                DaemonKey::new(DEFAULT_RELAY_REALM_ID, "daemon-disconnected"),
                "disconnected".to_string(),
                u64::MAX,
                Vec::new(),
            );
        }
        let expired = relay_http_get(addr, "/display/expired/vnc.html").await;
        assert!(expired.starts_with("HTTP/1.1 410 Gone"));

        let disconnected = relay_http_get(addr, "/display/disconnected/vnc.html").await;
        assert!(disconnected.starts_with("HTTP/1.1 502 Bad Gateway"));

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect to relay");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration("daemon-live", "machine-1", "macOS", 10),
                })
                .expect("register envelope should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRegister {
                    registration: RelayDisplayTunnelRegistration {
                        tunnel_id: "live".to_string(),
                        expires_at_ms: u64::MAX,
                        capabilities: vec!["view".to_string()],
                    },
                })
                .expect("display tunnel register should serialize")
                .into(),
            ))
            .await
            .expect("display tunnel register should send");
        let _ = daemon_socket.next().await;

        let response_task =
            tokio::spawn(async move { relay_http_get(addr, "/display/live/vnc.html").await });
        let open_payload = match daemon_socket.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected display tunnel open request: {other:?}"),
        };
        let stream_id = match serde_json::from_str::<RelayEnvelope>(&open_payload)
            .expect("display tunnel open should decode")
        {
            RelayEnvelope::DaemonDisplayTunnelOpen { request } => {
                assert_eq!(request.tunnel_id, "live");
                assert_eq!(request.method, "GET");
                assert_eq!(request.path, "/display/live/vnc.html");
                request.stream_id
            }
            other => panic!("unexpected display tunnel open envelope: {other:?}"),
        };
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelResponseStart {
                    response: crate::protocol::RelayDisplayTunnelResponseStart {
                        stream_id: stream_id.clone(),
                        status: 200,
                        headers: vec![RelayDisplayTunnelHeader {
                            name: "content-type".to_string(),
                            value: "text/plain".to_string(),
                        }],
                    },
                })
                .expect("display response start should serialize")
                .into(),
            ))
            .await
            .expect("display response start should send");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelChunk {
                    chunk: RelayDisplayTunnelStreamChunk {
                        stream_id: stream_id.clone(),
                        data: "aGVsbG8=".to_string(),
                        message_kind: None,
                    },
                })
                .expect("display chunk should serialize")
                .into(),
            ))
            .await
            .expect("display chunk should send");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelClose {
                    stream_id,
                    error: None,
                })
                .expect("display close should serialize")
                .into(),
            ))
            .await
            .expect("display close should send");
        let live = response_task
            .await
            .expect("display response task should join");
        assert!(live.starts_with("HTTP/1.1 200 OK"));
        assert!(live.contains("content-type: text/plain"));
        assert!(live.ends_with("\r\n\r\nhello"));

        let _ = daemon_socket.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn display_http_request_closes_promptly_when_daemon_disconnects() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect to relay");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration("daemon-live", "machine-1", "macOS", 10),
                })
                .expect("register envelope should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRegister {
                    registration: RelayDisplayTunnelRegistration {
                        tunnel_id: "live-disconnect".to_string(),
                        expires_at_ms: u64::MAX,
                        capabilities: vec!["view".to_string()],
                    },
                })
                .expect("display tunnel register should serialize")
                .into(),
            ))
            .await
            .expect("display tunnel register should send");
        let _ = daemon_socket.next().await;

        let response_task =
            tokio::spawn(
                async move { relay_http_get(addr, "/display/live-disconnect/vnc.html").await },
            );
        match daemon_socket.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("display tunnel open should decode")
            {
                RelayEnvelope::DaemonDisplayTunnelOpen { request } => {
                    assert_eq!(request.tunnel_id, "live-disconnect");
                }
                other => panic!("unexpected display tunnel open envelope: {other:?}"),
            },
            other => panic!("unexpected display tunnel open request: {other:?}"),
        }

        daemon_socket
            .close(None)
            .await
            .expect("daemon socket should close");
        let response = response_task
            .await
            .expect("display response task should join");
        assert!(response.starts_with("HTTP/1.1 502 Bad Gateway"));
        assert!(response.contains("display tunnel failed: target daemon disconnected from relay"));

        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    async fn relay_http_get(addr: std::net::SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(addr)
            .await
            .expect("relay HTTP connection should open");
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nhost: relay.test\r\n\r\n").as_bytes())
            .await
            .expect("relay HTTP request should write");
        let mut response = String::new();
        timeout(Duration::from_secs(2), stream.read_to_string(&mut response))
            .await
            .expect("relay HTTP response should complete")
            .expect("relay HTTP response should read");
        response
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_endpoint_reports_healthy_and_draining_status() {
        let healthy_server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = healthy_server
            .bind_listener()
            .await
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");

        let healthy_server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            healthy_server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let healthy = relay_http_get(addr, "/healthz").await;
        assert!(healthy.starts_with("HTTP/1.1 200 OK"));
        assert!(healthy.contains("\"status\":\"healthy\""));
        assert!(healthy.contains("\"draining\":false"));
        let _ = shutdown_tx.send(());
        server_task.await.expect("healthy server task should join");

        let draining_server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = draining_server
            .bind_listener()
            .await
            .expect("draining relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");

        let draining_server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        draining_server.set_draining(true);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            draining_server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("draining relay server should run");
        });

        let draining_health = relay_http_get(addr, "/healthz").await;
        assert!(draining_health.starts_with("HTTP/1.1 200 OK"));
        assert!(draining_health.contains("\"status\":\"draining\""));
        assert!(draining_health.contains("\"draining\":true"));

        let draining_ready = relay_http_get(addr, "/readyz").await;
        assert!(draining_ready.starts_with("HTTP/1.1 503 Service Unavailable"));
        assert!(draining_ready.contains("\"status\":\"draining\""));
        assert!(draining_ready.contains("\"draining\":true"));

        let websocket_error = connect_async(format!("ws://{addr}"))
            .await
            .expect_err("draining relay should reject new websocket admissions");
        assert!(
            websocket_error.to_string().contains("503"),
            "unexpected websocket error: {websocket_error}"
        );

        let _ = shutdown_tx.send(());
        server_task.await.expect("draining server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn display_websocket_route_bridges_browser_and_daemon_frames() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let relay_url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&relay_url)
            .await
            .expect("daemon should connect to relay");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration("daemon-live", "machine-1", "macOS", 10),
                })
                .expect("register envelope should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelRegister {
                    registration: RelayDisplayTunnelRegistration {
                        tunnel_id: "live".to_string(),
                        expires_at_ms: u64::MAX,
                        capabilities: vec!["view".to_string(), "keyboard".to_string()],
                    },
                })
                .expect("display tunnel register should serialize")
                .into(),
            ))
            .await
            .expect("display tunnel register should send");
        let _ = daemon_socket.next().await;

        let browser_url = format!("ws://{}:{}/display/live/websockify", addr.ip(), addr.port());
        let browser_task = tokio::spawn(async move {
            connect_async(browser_url)
                .await
                .expect("browser display websocket should connect")
                .0
        });
        let open_payload = match daemon_socket.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected display tunnel websocket open: {other:?}"),
        };
        let stream_id = match serde_json::from_str::<RelayEnvelope>(&open_payload)
            .expect("display websocket open should decode")
        {
            RelayEnvelope::DaemonDisplayTunnelOpen { request } => {
                assert_eq!(request.tunnel_id, "live");
                assert_eq!(request.path, "/display/live/websockify");
                assert!(request
                    .headers
                    .iter()
                    .any(|header| header.name.eq_ignore_ascii_case("upgrade")
                        && header.value.eq_ignore_ascii_case("websocket")));
                request.stream_id
            }
            other => panic!("unexpected display tunnel open envelope: {other:?}"),
        };
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelResponseStart {
                    response: crate::protocol::RelayDisplayTunnelResponseStart {
                        stream_id: stream_id.clone(),
                        status: 101,
                        headers: Vec::new(),
                    },
                })
                .expect("display websocket response start should serialize")
                .into(),
            ))
            .await
            .expect("display websocket response start should send");
        let mut browser_socket = browser_task.await.expect("browser task should join");

        browser_socket
            .send(Message::Binary(Vec::from("from-browser").into()))
            .await
            .expect("browser frame should send");
        let client_chunk_payload = match daemon_socket.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected display tunnel client chunk: {other:?}"),
        };
        match serde_json::from_str::<RelayEnvelope>(&client_chunk_payload)
            .expect("display client chunk should decode")
        {
            RelayEnvelope::DaemonDisplayTunnelClientChunk { chunk } => {
                assert_eq!(chunk.stream_id, stream_id);
                assert_eq!(chunk.message_kind.as_deref(), Some("binary"));
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(chunk.data)
                    .expect("chunk data should decode");
                assert_eq!(decoded, b"from-browser");
            }
            other => panic!("unexpected display client chunk envelope: {other:?}"),
        };

        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonDisplayTunnelChunk {
                    chunk: RelayDisplayTunnelStreamChunk {
                        stream_id: stream_id.clone(),
                        data: base64::engine::general_purpose::STANDARD.encode("from-daemon"),
                        message_kind: Some("binary".to_string()),
                    },
                })
                .expect("display daemon chunk should serialize")
                .into(),
            ))
            .await
            .expect("display daemon chunk should send");
        match browser_socket.next().await {
            Some(Ok(Message::Binary(data))) => assert_eq!(data.as_ref(), b"from-daemon"),
            other => panic!("unexpected browser display frame: {other:?}"),
        }

        let _ = browser_socket.close(None).await;
        let _ = daemon_socket.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
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
                provider_accounts: Vec::new(),
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
        sleep(Duration::from_millis(50)).await;

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
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_a, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon A should connect");
        let (mut daemon_b, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon B should connect");
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
        sleep(Duration::from_millis(50)).await;

        let (mut client_a, _) = connect_async_with_retry(&url)
            .await
            .expect("client A should connect");
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

        let (mut client_b, _) = connect_async_with_retry(&url)
            .await
            .expect("client B should connect");
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
    async fn accepted_daemon_socket_is_not_closed_when_initial_token_expires() {
        let mut claims = BTreeMap::new();
        claims.insert(
            "daemon-token".to_string(),
            scoped_claim(
                "daemon-token",
                "daemon-1",
                RelaySubjectKind::Kernel,
                "realm-a",
                vec![RelayAction::DaemonRegister],
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

        let auth_verifier = server.auth_verifier.clone();
        let server = RelayServer::with_auth_verifier(
            RelayConfig {
                host: addr.ip().to_string(),
                port: addr.port(),
                shared_token: None,
            },
            auth_verifier,
        );
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration_with_token(
                        "daemon-1",
                        "machine-1",
                        "Linux",
                        10,
                        "daemon-token",
                    ),
                })
                .expect("register should serialize")
                .into(),
            ))
            .await
            .expect("register should send");

        sleep(Duration::from_millis(75)).await;
        {
            let guard = registry.read().await;
            assert_eq!(guard.daemon_count(), 1);
            assert_eq!(guard.peer_count(), 1);
        }
        assert_no_relay_close(&mut daemon_socket).await;

        let _ = daemon_socket.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn accepted_client_socket_is_not_closed_when_initial_token_expires() {
        let mut claims = BTreeMap::new();
        claims.insert(
            "daemon-token".to_string(),
            scoped_claim(
                "daemon-token",
                "daemon-1",
                RelaySubjectKind::Kernel,
                "realm-a",
                vec![RelayAction::DaemonRegister],
                None,
            ),
        );
        claims.insert(
            "client-token".to_string(),
            scoped_claim(
                "client-token",
                "client-1",
                RelaySubjectKind::Client,
                "realm-a",
                vec![RelayAction::ClientConnect],
                Some(vec!["daemon-1"]),
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
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration_with_token(
                        "daemon-1",
                        "machine-1",
                        "Linux",
                        10,
                        "daemon-token",
                    ),
                })
                .expect("register should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
        sleep(Duration::from_millis(50)).await;

        let (mut client_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("client should connect");
        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "client-token".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                })
                .expect("client connect should serialize")
                .into(),
            ))
            .await
            .expect("client connect should send");
        let connect_payload = match client_socket.next().await {
            Some(Ok(Message::Text(text))) => text,
            other => panic!("unexpected connect response: {other:?}"),
        };
        match serde_json::from_str::<RelayEnvelope>(&connect_payload)
            .expect("connect response should decode")
        {
            RelayEnvelope::ClientConnected {
                daemon_public_key, ..
            } => assert_eq!(daemon_public_key, "public-key-daemon-1"),
            other => panic!("unexpected connect response envelope: {other:?}"),
        }

        sleep(Duration::from_millis(75)).await;
        assert_no_relay_close(&mut client_socket).await;

        let _ = client_socket.close(None).await;
        let _ = daemon_socket.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn expired_token_is_rejected_for_new_client_connection() {
        let mut claims = BTreeMap::new();
        claims.insert(
            "daemon-token".to_string(),
            scoped_claim(
                "daemon-token",
                "daemon-1",
                RelaySubjectKind::Kernel,
                "realm-a",
                vec![RelayAction::DaemonRegister],
                None,
            ),
        );
        let mut expired_client_claim = scoped_claim(
            "expired-client-token",
            "client-1",
            RelaySubjectKind::Client,
            "realm-a",
            vec![RelayAction::ClientConnect],
            Some(vec!["daemon-1"]),
        );
        expired_client_claim.expires_at_ms = 5;
        claims.insert("expired-client-token".to_string(), expired_client_claim);
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
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration_with_token(
                        "daemon-1",
                        "machine-1",
                        "Linux",
                        10,
                        "daemon-token",
                    ),
                })
                .expect("register should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
        sleep(Duration::from_millis(50)).await;

        let (mut client_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("client should connect");
        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "expired-client-token".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                })
                .expect("client connect should serialize")
                .into(),
            ))
            .await
            .expect("client connect should send");

        match timeout(Duration::from_millis(500), client_socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let envelope = serde_json::from_str::<RelayEnvelope>(&text)
                    .expect("relay response should decode");
                assert!(
                    !matches!(envelope, RelayEnvelope::ClientConnected { .. }),
                    "expired token must not connect a client"
                );
            }
            Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => {}
            Ok(other) => panic!("unexpected expired-token response: {other:?}"),
            Err(_) => panic!("expired token did not close or reject promptly"),
        }

        let _ = daemon_socket.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_client_frames_require_accepted_client_connect() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration("daemon-1", "machine-1", "Linux", 10),
                })
                .expect("register should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
        sleep(Duration::from_millis(50)).await;

        let (mut client_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("client should connect");
        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientRequest {
                    request_id: "request-before-connect".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                    encrypted_request: EncryptedRelayPayload {
                        sender_public_key: "client-public".to_string(),
                        nonce: "nonce".to_string(),
                        ciphertext: "ciphertext".to_string(),
                    },
                })
                .expect("client request should serialize")
                .into(),
            ))
            .await
            .expect("client request should send");

        let close_payload = match timeout(Duration::from_millis(500), client_socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => text,
            Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => String::new().into(),
            Ok(other) => panic!("unexpected pre-connect client response: {other:?}"),
            Err(_) => panic!("pre-connect client request did not close promptly"),
        };
        if !close_payload.is_empty() {
            match serde_json::from_str::<RelayEnvelope>(&close_payload)
                .expect("relay close should decode")
            {
                RelayEnvelope::Close { reason } => {
                    assert_eq!(reason, "client must connect before sending requests");
                }
                other => panic!("unexpected pre-connect response envelope: {other:?}"),
            }
        }
        match timeout(Duration::from_millis(100), daemon_socket.next()).await {
            Err(_) => {}
            Ok(other) => panic!("pre-connect request reached daemon: {other:?}"),
        }
        assert_eq!(registry.read().await.pending_request_count(), 0);

        let _ = daemon_socket.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_client_frames_reject_empty_identifiers_without_pending_state() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration("daemon-1", "machine-1", "Linux", 10),
                })
                .expect("register should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
        sleep(Duration::from_millis(50)).await;

        let (mut client_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("client should connect");
        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "secret".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                })
                .expect("client connect should serialize")
                .into(),
            ))
            .await
            .expect("client connect should send");
        match client_socket.next().await {
            Some(Ok(Message::Text(text))) => assert!(matches!(
                serde_json::from_str::<RelayEnvelope>(&text).expect("connect should decode"),
                RelayEnvelope::ClientConnected { .. }
            )),
            other => panic!("unexpected client connect response: {other:?}"),
        }

        let encrypted_request = EncryptedRelayPayload {
            sender_public_key: "client-public".to_string(),
            nonce: "nonce".to_string(),
            ciphertext: "ciphertext".to_string(),
        };
        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientRequest {
                    request_id: "   ".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                    encrypted_request: encrypted_request.clone(),
                })
                .expect("invalid request should serialize")
                .into(),
            ))
            .await
            .expect("invalid request should send");
        match timeout(Duration::from_millis(500), client_socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("invalid request response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: Some(error),
                } => {
                    assert_eq!(request_id, "   ");
                    assert_eq!(error.code, "invalid_runtime_identifier");
                    assert!(!error.retryable);
                }
                other => panic!("unexpected invalid request response: {other:?}"),
            },
            Ok(other) => panic!("unexpected invalid request frame: {other:?}"),
            Err(_) => panic!("invalid request response was not delivered"),
        }
        match timeout(Duration::from_millis(100), daemon_socket.next()).await {
            Err(_) => {}
            Ok(other) => panic!("invalid request reached daemon: {other:?}"),
        }
        assert_eq!(registry.read().await.pending_request_count(), 0);

        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                    request_id: "invalid-subscription".to_string(),
                    subscription_id: "\t".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                    session_id: "session-1".to_string(),
                    attachment_id: "terminal".to_string(),
                    client_public_key: "client-public".to_string(),
                    subscription_scope: None,
                    resume_from_event_id: None,
                })
                .expect("invalid subscribe should serialize")
                .into(),
            ))
            .await
            .expect("invalid subscribe should send");
        match timeout(Duration::from_millis(500), client_socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("invalid subscribe response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: Some(error),
                } => {
                    assert_eq!(request_id, "invalid-subscription");
                    assert_eq!(error.code, "invalid_runtime_identifier");
                    assert_eq!(error.message, "subscription_id must not be empty");
                    assert!(!error.retryable);
                }
                other => panic!("unexpected invalid subscribe response: {other:?}"),
            },
            Ok(other) => panic!("unexpected invalid subscribe frame: {other:?}"),
            Err(_) => panic!("invalid subscribe response was not delivered"),
        }
        match timeout(Duration::from_millis(100), daemon_socket.next()).await {
            Err(_) => {}
            Ok(other) => panic!("invalid subscribe reached daemon: {other:?}"),
        }
        {
            let guard = registry.read().await;
            assert_eq!(guard.pending_request_count(), 0);
            assert_eq!(guard.subscription_count(), 0);
        }

        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientUnsubscribe {
                    request_id: "invalid-unsubscribe".to_string(),
                    subscription_id: "".to_string(),
                    client_public_key: "client-public".to_string(),
                })
                .expect("invalid unsubscribe should serialize")
                .into(),
            ))
            .await
            .expect("invalid unsubscribe should send");
        match timeout(Duration::from_millis(500), client_socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("invalid unsubscribe response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: Some(error),
                } => {
                    assert_eq!(request_id, "invalid-unsubscribe");
                    assert_eq!(error.code, "invalid_runtime_identifier");
                    assert_eq!(error.message, "subscription_id must not be empty");
                    assert!(!error.retryable);
                }
                other => panic!("unexpected invalid unsubscribe response: {other:?}"),
            },
            Ok(other) => panic!("unexpected invalid unsubscribe frame: {other:?}"),
            Err(_) => panic!("invalid unsubscribe response was not delivered"),
        }
        match timeout(Duration::from_millis(100), daemon_socket.next()).await {
            Err(_) => {}
            Ok(other) => panic!("invalid unsubscribe reached daemon: {other:?}"),
        }
        {
            let guard = registry.read().await;
            assert_eq!(guard.pending_request_count(), 0);
            assert_eq!(guard.subscription_count(), 0);
        }

        let _ = client_socket.close(None).await;
        let _ = daemon_socket.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_client_frames_must_match_connected_target() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_a, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon A should connect");
        let (mut daemon_b, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon B should connect");
        for (socket, daemon_id) in [(&mut daemon_a, "daemon-a"), (&mut daemon_b, "daemon-b")] {
            socket
                .send(Message::Text(
                    serde_json::to_string(&RelayEnvelope::DaemonRegister {
                        registration: test_registration(daemon_id, "machine-1", "Linux", 10),
                    })
                    .expect("register should serialize")
                    .into(),
                ))
                .await
                .expect("register should send");
        }
        sleep(Duration::from_millis(50)).await;

        let (mut client_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("client should connect");
        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "secret".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-a".to_string()),
                        daemon_alias: None,
                    },
                })
                .expect("client connect should serialize")
                .into(),
            ))
            .await
            .expect("client connect should send");
        match client_socket.next().await {
            Some(Ok(Message::Text(text))) => assert!(matches!(
                serde_json::from_str::<RelayEnvelope>(&text).expect("connect should decode"),
                RelayEnvelope::ClientConnected { .. }
            )),
            other => panic!("unexpected client connect response: {other:?}"),
        }

        let encrypted_request = EncryptedRelayPayload {
            sender_public_key: "client-public".to_string(),
            nonce: "nonce".to_string(),
            ciphertext: "ciphertext".to_string(),
        };
        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientRequest {
                    request_id: "request-wrong-target".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-b".to_string()),
                        daemon_alias: None,
                    },
                    encrypted_request: encrypted_request.clone(),
                })
                .expect("wrong-target request should serialize")
                .into(),
            ))
            .await
            .expect("wrong-target request should send");
        match client_socket.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("wrong-target response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: Some(error),
                } => {
                    assert_eq!(request_id, "request-wrong-target");
                    assert_eq!(error.code, "target_mismatch");
                }
                other => panic!("unexpected wrong-target response: {other:?}"),
            },
            other => panic!("unexpected wrong-target frame: {other:?}"),
        }
        match timeout(Duration::from_millis(100), daemon_b.next()).await {
            Err(_) => {}
            Ok(other) => panic!("wrong-target request reached daemon B: {other:?}"),
        }

        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                    request_id: "subscribe-wrong-target".to_string(),
                    subscription_id: "wrong-target-subscription".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-b".to_string()),
                        daemon_alias: None,
                    },
                    session_id: "session-1".to_string(),
                    attachment_id: "terminal".to_string(),
                    client_public_key: "client-public".to_string(),
                    subscription_scope: None,
                    resume_from_event_id: None,
                })
                .expect("wrong-target subscribe should serialize")
                .into(),
            ))
            .await
            .expect("wrong-target subscribe should send");
        match client_socket.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("wrong-target subscribe response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: Some(error),
                } => {
                    assert_eq!(request_id, "subscribe-wrong-target");
                    assert_eq!(error.code, "target_mismatch");
                }
                other => panic!("unexpected wrong-target subscribe response: {other:?}"),
            },
            other => panic!("unexpected wrong-target subscribe frame: {other:?}"),
        }
        assert_eq!(registry.read().await.pending_request_count(), 0);

        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientRequest {
                    request_id: "request-right-target".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-a".to_string()),
                        daemon_alias: None,
                    },
                    encrypted_request,
                })
                .expect("right-target request should serialize")
                .into(),
            ))
            .await
            .expect("right-target request should send");
        match daemon_a.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("right-target daemon request should decode")
            {
                RelayEnvelope::DaemonRequest { .. } => {}
                other => panic!("unexpected right-target daemon envelope: {other:?}"),
            },
            other => panic!("unexpected right-target daemon frame: {other:?}"),
        }

        let _ = client_socket.close(None).await;
        let _ = daemon_a.close(None).await;
        let _ = daemon_b.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn duplicate_daemon_aliases_do_not_bind_clients_arbitrarily() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_a, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon A should connect");
        let (mut daemon_b, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon B should connect");
        let mut registration_a = test_registration("daemon-a", "machine-1", "Linux", 10);
        registration_a.daemon_alias = Some("shared-alias".to_string());
        registration_a.public_key = "public-key-a".to_string();
        let mut registration_b = test_registration("daemon-b", "machine-2", "Linux", 20);
        registration_b.daemon_alias = Some("shared-alias".to_string());
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
        sleep(Duration::from_millis(50)).await;

        let (mut ambiguous_client, _) = connect_async_with_retry(&url)
            .await
            .expect("ambiguous client should connect");
        ambiguous_client
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "secret".to_string(),
                    target: ClientTarget {
                        daemon_id: None,
                        daemon_alias: Some("shared-alias".to_string()),
                    },
                })
                .expect("ambiguous connect should serialize")
                .into(),
            ))
            .await
            .expect("ambiguous connect should send");
        let close_payload = match timeout(Duration::from_millis(500), ambiguous_client.next()).await
        {
            Ok(Some(Ok(Message::Text(text)))) => text,
            Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => String::new().into(),
            Ok(other) => panic!("unexpected ambiguous connect response: {other:?}"),
            Err(_) => panic!("ambiguous connect did not close promptly"),
        };
        if !close_payload.is_empty() {
            match serde_json::from_str::<RelayEnvelope>(&close_payload)
                .expect("ambiguous close should decode")
            {
                RelayEnvelope::Close { reason } => {
                    assert_eq!(reason, "target daemon is not connected to relay");
                }
                RelayEnvelope::ClientConnected {
                    daemon_public_key, ..
                } => {
                    panic!("ambiguous alias connected to daemon key {daemon_public_key}")
                }
                other => panic!("unexpected ambiguous connect envelope: {other:?}"),
            }
        }

        let (mut exact_client, _) = connect_async_with_retry(&url)
            .await
            .expect("exact client should connect");
        exact_client
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "secret".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-a".to_string()),
                        daemon_alias: None,
                    },
                })
                .expect("exact connect should serialize")
                .into(),
            ))
            .await
            .expect("exact connect should send");
        match exact_client.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("exact connect response should decode")
            {
                RelayEnvelope::ClientConnected {
                    daemon_public_key, ..
                } => assert_eq!(daemon_public_key, "public-key-a"),
                other => panic!("unexpected exact connect envelope: {other:?}"),
            },
            other => panic!("unexpected exact connect response: {other:?}"),
        }

        let _ = ambiguous_client.close(None).await;
        let _ = exact_client.close(None).await;
        let _ = daemon_a.close(None).await;
        let _ = daemon_b.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scoped_client_tokens_gate_packet_routing() {
        let mut claims = BTreeMap::new();
        claims.insert(
            "daemon-token".to_string(),
            scoped_claim(
                "daemon-token",
                "daemon-1",
                RelaySubjectKind::Kernel,
                "realm-a",
                vec![RelayAction::DaemonRegister],
                None,
            ),
        );
        claims.insert(
            "client-connect-only-token".to_string(),
            scoped_claim(
                "client-connect-only-token",
                "client-1",
                RelaySubjectKind::Client,
                "realm-a",
                vec![RelayAction::ClientConnect],
                Some(vec!["daemon-1"]),
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

        let auth_verifier = server.auth_verifier.clone();
        let server = RelayServer::with_auth_verifier(
            RelayConfig {
                host: addr.ip().to_string(),
                port: addr.port(),
                shared_token: None,
            },
            auth_verifier,
        );
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration_with_token(
                        "daemon-1",
                        "machine-1",
                        "Linux",
                        10,
                        "daemon-token",
                    ),
                })
                .expect("register should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
        sleep(Duration::from_millis(50)).await;

        let (mut client_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("client should connect");
        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "client-connect-only-token".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                })
                .expect("client connect should serialize")
                .into(),
            ))
            .await
            .expect("client connect should send");
        match client_socket.next().await {
            Some(Ok(Message::Text(text))) => assert!(matches!(
                serde_json::from_str::<RelayEnvelope>(&text).expect("connect should decode"),
                RelayEnvelope::ClientConnected { .. }
            )),
            other => panic!("unexpected client connect response: {other:?}"),
        }

        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientRequest {
                    request_id: "packet-route-denied".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                    encrypted_request: EncryptedRelayPayload {
                        sender_public_key: "client-public".to_string(),
                        nonce: "nonce".to_string(),
                        ciphertext: "ciphertext".to_string(),
                    },
                })
                .expect("client request should serialize")
                .into(),
            ))
            .await
            .expect("client request should send");
        match client_socket.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("packet-route denial should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: Some(error),
                } => {
                    assert_eq!(request_id, "packet-route-denied");
                    assert_eq!(error.code, "action_not_allowed");
                }
                other => panic!("unexpected packet-route denial envelope: {other:?}"),
            },
            other => panic!("unexpected packet-route denial frame: {other:?}"),
        }
        match timeout(Duration::from_millis(100), daemon_socket.next()).await {
            Err(_) => {}
            Ok(other) => panic!("unauthorized packet route reached daemon: {other:?}"),
        }
        assert_eq!(registry.read().await.pending_request_count(), 0);

        let _ = client_socket.close(None).await;
        let _ = daemon_socket.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_responses_must_match_pending_request_owner() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_a, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon A should connect");
        let (mut daemon_b, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon B should connect");
        for (socket, daemon_id) in [(&mut daemon_a, "daemon-a"), (&mut daemon_b, "daemon-b")] {
            socket
                .send(Message::Text(
                    serde_json::to_string(&RelayEnvelope::DaemonRegister {
                        registration: test_registration(daemon_id, "machine-1", "Linux", 10),
                    })
                    .expect("register should serialize")
                    .into(),
                ))
                .await
                .expect("register should send");
        }
        sleep(Duration::from_millis(50)).await;

        let (mut client_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("client should connect");
        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "secret".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-a".to_string()),
                        daemon_alias: None,
                    },
                })
                .expect("client connect should serialize")
                .into(),
            ))
            .await
            .expect("client connect should send");
        match client_socket.next().await {
            Some(Ok(Message::Text(text))) => {
                assert!(matches!(
                    serde_json::from_str::<RelayEnvelope>(&text)
                        .expect("client connected should decode"),
                    RelayEnvelope::ClientConnected { .. }
                ));
            }
            other => panic!("unexpected client connect response: {other:?}"),
        }

        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientRequest {
                    request_id: "client-request-1".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-a".to_string()),
                        daemon_alias: None,
                    },
                    encrypted_request: EncryptedRelayPayload {
                        sender_public_key: "client-public".to_string(),
                        nonce: "nonce".to_string(),
                        ciphertext: "ciphertext".to_string(),
                    },
                })
                .expect("client request should serialize")
                .into(),
            ))
            .await
            .expect("client request should send");

        let relay_request_id = match daemon_a.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("daemon request should decode")
            {
                RelayEnvelope::DaemonRequest {
                    relay_request_id, ..
                } => relay_request_id,
                other => panic!("unexpected daemon request envelope: {other:?}"),
            },
            other => panic!("unexpected daemon request frame: {other:?}"),
        };

        let encrypted_response = EncryptedRelayPayload {
            sender_public_key: "daemon-public".to_string(),
            nonce: "nonce-response".to_string(),
            ciphertext: "ciphertext-response".to_string(),
        };
        daemon_b
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonResponse {
                    relay_request_id: relay_request_id.clone(),
                    encrypted_response: Some(encrypted_response.clone()),
                    error: None,
                })
                .expect("wrong daemon response should serialize")
                .into(),
            ))
            .await
            .expect("wrong daemon response should send");
        match timeout(Duration::from_millis(100), client_socket.next()).await {
            Err(_) => {}
            Ok(other) => panic!("wrong daemon completed client request: {other:?}"),
        }
        assert_eq!(registry.read().await.pending_request_count(), 1);

        daemon_a
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: Some(encrypted_response.clone()),
                    error: None,
                })
                .expect("owner daemon response should serialize")
                .into(),
            ))
            .await
            .expect("owner daemon response should send");
        match timeout(Duration::from_millis(500), client_socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("client response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: Some(response),
                    error: None,
                } => {
                    assert_eq!(request_id, "client-request-1");
                    assert_eq!(response, encrypted_response);
                }
                other => panic!("unexpected client response envelope: {other:?}"),
            },
            Ok(other) => panic!("unexpected client response frame: {other:?}"),
            Err(_) => panic!("owner daemon response was not delivered"),
        }
        assert_eq!(registry.read().await.pending_request_count(), 0);

        let _ = client_socket.close(None).await;
        let _ = daemon_a.close(None).await;
        let _ = daemon_b.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_events_must_match_subscription_owner() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_a, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon A should connect");
        let (mut daemon_b, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon B should connect");
        for (socket, daemon_id) in [(&mut daemon_a, "daemon-a"), (&mut daemon_b, "daemon-b")] {
            socket
                .send(Message::Text(
                    serde_json::to_string(&RelayEnvelope::DaemonRegister {
                        registration: test_registration(daemon_id, "machine-1", "Linux", 10),
                    })
                    .expect("register should serialize")
                    .into(),
                ))
                .await
                .expect("register should send");
        }
        sleep(Duration::from_millis(50)).await;

        let (mut client_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("client should connect");
        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "secret".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-a".to_string()),
                        daemon_alias: None,
                    },
                })
                .expect("client connect should serialize")
                .into(),
            ))
            .await
            .expect("client connect should send");
        match client_socket.next().await {
            Some(Ok(Message::Text(text))) => {
                assert!(matches!(
                    serde_json::from_str::<RelayEnvelope>(&text)
                        .expect("client connected should decode"),
                    RelayEnvelope::ClientConnected { .. }
                ));
            }
            other => panic!("unexpected client connect response: {other:?}"),
        }

        client_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                    request_id: "subscribe-1".to_string(),
                    subscription_id: "subscription-1".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-a".to_string()),
                        daemon_alias: None,
                    },
                    session_id: "session-1".to_string(),
                    attachment_id: "terminal".to_string(),
                    client_public_key: "client-public".to_string(),
                    subscription_scope: None,
                    resume_from_event_id: None,
                })
                .expect("client subscribe should serialize")
                .into(),
            ))
            .await
            .expect("client subscribe should send");

        let subscribe_relay_request_id = match daemon_a.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("daemon subscribe should decode")
            {
                RelayEnvelope::DaemonSubscribe {
                    relay_request_id,
                    relay_subscription_id,
                    ..
                } => {
                    assert_eq!(relay_subscription_id, "subscription-1");
                    relay_request_id
                }
                other => panic!("unexpected daemon subscribe envelope: {other:?}"),
            },
            other => panic!("unexpected daemon subscribe frame: {other:?}"),
        };
        daemon_a
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonResponse {
                    relay_request_id: subscribe_relay_request_id,
                    encrypted_response: None,
                    error: None,
                })
                .expect("subscribe response should serialize")
                .into(),
            ))
            .await
            .expect("subscribe response should send");
        match timeout(Duration::from_millis(500), client_socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("subscribe response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: None,
                } => assert_eq!(request_id, "subscribe-1"),
                other => panic!("unexpected subscribe response envelope: {other:?}"),
            },
            Ok(other) => panic!("unexpected subscribe response frame: {other:?}"),
            Err(_) => panic!("subscribe response was not delivered"),
        }
        assert_eq!(registry.read().await.subscription_count(), 1);

        let encrypted_event = EncryptedRelayPayload {
            sender_public_key: "daemon-public".to_string(),
            nonce: "nonce-event".to_string(),
            ciphertext: "ciphertext-event".to_string(),
        };
        daemon_b
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonEvent {
                    subscription_id: "subscription-1".to_string(),
                    event_id: 1,
                    encrypted_event: encrypted_event.clone(),
                })
                .expect("wrong daemon event should serialize")
                .into(),
            ))
            .await
            .expect("wrong daemon event should send");
        match timeout(Duration::from_millis(100), client_socket.next()).await {
            Err(_) => {}
            Ok(other) => panic!("wrong daemon emitted subscription event: {other:?}"),
        }

        daemon_a
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonEvent {
                    subscription_id: "subscription-1".to_string(),
                    event_id: 2,
                    encrypted_event: encrypted_event.clone(),
                })
                .expect("owner daemon event should serialize")
                .into(),
            ))
            .await
            .expect("owner daemon event should send");
        match timeout(Duration::from_millis(500), client_socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("client event should decode")
            {
                RelayEnvelope::ClientEvent {
                    subscription_id,
                    event_id,
                    encrypted_event: response,
                } => {
                    assert_eq!(subscription_id, "subscription-1");
                    assert_eq!(event_id, 2);
                    assert_eq!(response, encrypted_event);
                }
                other => panic!("unexpected client event envelope: {other:?}"),
            },
            Ok(other) => panic!("unexpected client event frame: {other:?}"),
            Err(_) => panic!("owner daemon event was not delivered"),
        }

        let _ = client_socket.close(None).await;
        let _ = daemon_a.close(None).await;
        let _ = daemon_b.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn subscription_ids_are_owned_by_connected_client() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration("daemon-1", "machine-1", "Linux", 10),
                })
                .expect("register should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
        sleep(Duration::from_millis(50)).await;

        let (mut client_a, _) = connect_async_with_retry(&url)
            .await
            .expect("client A should connect");
        let (mut client_b, _) = connect_async_with_retry(&url)
            .await
            .expect("client B should connect");
        for (socket, label) in [(&mut client_a, "a"), (&mut client_b, "b")] {
            socket
                .send(Message::Text(
                    serde_json::to_string(&RelayEnvelope::ClientConnect {
                        auth_token: "secret".to_string(),
                        target: ClientTarget {
                            daemon_id: Some("daemon-1".to_string()),
                            daemon_alias: None,
                        },
                    })
                    .expect("client connect should serialize")
                    .into(),
                ))
                .await
                .unwrap_or_else(|error| panic!("client {label} connect should send: {error}"));
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    assert!(matches!(
                        serde_json::from_str::<RelayEnvelope>(&text)
                            .expect("client connected should decode"),
                        RelayEnvelope::ClientConnected { .. }
                    ));
                }
                other => panic!("unexpected client {label} connect response: {other:?}"),
            }
        }

        client_a
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                    request_id: "subscribe-a".to_string(),
                    subscription_id: "shared-subscription-id".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                    session_id: "session-1".to_string(),
                    attachment_id: "terminal".to_string(),
                    client_public_key: "client-a-public".to_string(),
                    subscription_scope: None,
                    resume_from_event_id: None,
                })
                .expect("client A subscribe should serialize")
                .into(),
            ))
            .await
            .expect("client A subscribe should send");

        let subscribe_relay_request_id = match daemon_socket.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("daemon subscribe should decode")
            {
                RelayEnvelope::DaemonSubscribe {
                    relay_request_id,
                    relay_subscription_id,
                    ..
                } => {
                    assert_eq!(relay_subscription_id, "shared-subscription-id");
                    relay_request_id
                }
                other => panic!("unexpected daemon subscribe envelope: {other:?}"),
            },
            other => panic!("unexpected daemon subscribe frame: {other:?}"),
        };

        client_b
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                    request_id: "subscribe-b-pending-collision".to_string(),
                    subscription_id: "shared-subscription-id".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                    session_id: "session-1".to_string(),
                    attachment_id: "terminal".to_string(),
                    client_public_key: "client-b-public".to_string(),
                    subscription_scope: None,
                    resume_from_event_id: None,
                })
                .expect("client B subscribe should serialize")
                .into(),
            ))
            .await
            .expect("client B subscribe should send");
        match client_b.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("client B subscribe response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: Some(error),
                } => {
                    assert_eq!(request_id, "subscribe-b-pending-collision");
                    assert_eq!(error.code, "subscription_conflict");
                }
                other => panic!("unexpected client B collision response: {other:?}"),
            },
            other => panic!("unexpected client B collision frame: {other:?}"),
        }
        match timeout(Duration::from_millis(100), daemon_socket.next()).await {
            Err(_) => {}
            Ok(other) => panic!("conflicting subscribe reached daemon: {other:?}"),
        }
        assert_eq!(registry.read().await.pending_request_count(), 1);

        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonResponse {
                    relay_request_id: subscribe_relay_request_id,
                    encrypted_response: None,
                    error: None,
                })
                .expect("subscribe response should serialize")
                .into(),
            ))
            .await
            .expect("subscribe response should send");
        match client_a.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("client A subscribe response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: None,
                } => assert_eq!(request_id, "subscribe-a"),
                other => panic!("unexpected client A subscribe response: {other:?}"),
            },
            other => panic!("unexpected client A subscribe frame: {other:?}"),
        }
        assert_eq!(registry.read().await.subscription_count(), 1);

        client_b
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientUnsubscribe {
                    request_id: "unsubscribe-b".to_string(),
                    subscription_id: "shared-subscription-id".to_string(),
                    client_public_key: "client-b-public".to_string(),
                })
                .expect("client B unsubscribe should serialize")
                .into(),
            ))
            .await
            .expect("client B unsubscribe should send");
        match client_b.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("client B unsubscribe response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: Some(error),
                } => {
                    assert_eq!(request_id, "unsubscribe-b");
                    assert_eq!(error.code, "subscription_not_found");
                }
                other => panic!("unexpected client B unsubscribe response: {other:?}"),
            },
            other => panic!("unexpected client B unsubscribe frame: {other:?}"),
        }
        match timeout(Duration::from_millis(100), daemon_socket.next()).await {
            Err(_) => {}
            Ok(other) => panic!("cross-client unsubscribe reached daemon: {other:?}"),
        }
        assert_eq!(registry.read().await.subscription_count(), 1);

        client_b
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                    request_id: "subscribe-b-active-collision".to_string(),
                    subscription_id: "shared-subscription-id".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                    session_id: "session-1".to_string(),
                    attachment_id: "terminal".to_string(),
                    client_public_key: "client-b-public".to_string(),
                    subscription_scope: None,
                    resume_from_event_id: None,
                })
                .expect("client B subscribe should serialize")
                .into(),
            ))
            .await
            .expect("client B subscribe should send");
        match client_b.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("client B active collision response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: Some(error),
                } => {
                    assert_eq!(request_id, "subscribe-b-active-collision");
                    assert_eq!(error.code, "subscription_conflict");
                }
                other => panic!("unexpected client B active collision response: {other:?}"),
            },
            other => panic!("unexpected client B active collision frame: {other:?}"),
        }

        let encrypted_event = EncryptedRelayPayload {
            sender_public_key: "daemon-public".to_string(),
            nonce: "nonce-event".to_string(),
            ciphertext: "ciphertext-event".to_string(),
        };
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonEvent {
                    subscription_id: "shared-subscription-id".to_string(),
                    event_id: 1,
                    encrypted_event: encrypted_event.clone(),
                })
                .expect("daemon event should serialize")
                .into(),
            ))
            .await
            .expect("daemon event should send");
        match client_a.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("client A event should decode")
            {
                RelayEnvelope::ClientEvent {
                    subscription_id,
                    event_id,
                    encrypted_event: response,
                } => {
                    assert_eq!(subscription_id, "shared-subscription-id");
                    assert_eq!(event_id, 1);
                    assert_eq!(response, encrypted_event);
                }
                other => panic!("unexpected client A event response: {other:?}"),
            },
            other => panic!("unexpected client A event frame: {other:?}"),
        }
        match timeout(Duration::from_millis(100), client_b.next()).await {
            Err(_) => {}
            Ok(other) => panic!("event leaked to non-owner client: {other:?}"),
        }

        let _ = client_a.close(None).await;
        let _ = client_b.close(None).await;
        let _ = daemon_socket.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disconnecting_client_drops_pending_requests_before_reconnect() {
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_socket, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon should connect");
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonRegister {
                    registration: test_registration("daemon-1", "machine-1", "Linux", 10),
                })
                .expect("register should serialize")
                .into(),
            ))
            .await
            .expect("register should send");
        sleep(Duration::from_millis(50)).await;

        let (mut first_client, _) = connect_async_with_retry(&url)
            .await
            .expect("first client should connect");
        first_client
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "secret".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                })
                .expect("first client connect should serialize")
                .into(),
            ))
            .await
            .expect("first client connect should send");
        match first_client.next().await {
            Some(Ok(Message::Text(text))) => assert!(matches!(
                serde_json::from_str::<RelayEnvelope>(&text).expect("connect should decode"),
                RelayEnvelope::ClientConnected { .. }
            )),
            other => panic!("unexpected first client connect response: {other:?}"),
        }

        first_client
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                    request_id: "first-subscribe".to_string(),
                    subscription_id: "recoverable-subscription".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                    session_id: "session-1".to_string(),
                    attachment_id: "terminal".to_string(),
                    client_public_key: "first-client-public".to_string(),
                    subscription_scope: None,
                    resume_from_event_id: None,
                })
                .expect("first subscribe should serialize")
                .into(),
            ))
            .await
            .expect("first subscribe should send");
        let stale_relay_request_id = match daemon_socket.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("daemon subscribe should decode")
            {
                RelayEnvelope::DaemonSubscribe {
                    relay_request_id,
                    relay_subscription_id,
                    ..
                } => {
                    assert_eq!(relay_subscription_id, "recoverable-subscription");
                    relay_request_id
                }
                other => panic!("unexpected first subscribe envelope: {other:?}"),
            },
            other => panic!("unexpected first subscribe frame: {other:?}"),
        };
        assert_eq!(registry.read().await.pending_request_count(), 1);

        let _ = first_client.close(None).await;
        sleep(Duration::from_millis(50)).await;
        assert_eq!(
            registry.read().await.pending_request_count(),
            0,
            "disconnecting clients must not leave stale pending relay requests"
        );
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonResponse {
                    relay_request_id: stale_relay_request_id,
                    encrypted_response: None,
                    error: None,
                })
                .expect("stale subscribe response should serialize")
                .into(),
            ))
            .await
            .expect("stale subscribe response should send");
        sleep(Duration::from_millis(50)).await;
        assert_eq!(registry.read().await.subscription_count(), 0);

        let (mut reconnecting_client, _) = connect_async_with_retry(&url)
            .await
            .expect("reconnecting client should connect");
        reconnecting_client
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientConnect {
                    auth_token: "secret".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                })
                .expect("reconnecting client connect should serialize")
                .into(),
            ))
            .await
            .expect("reconnecting client connect should send");
        match reconnecting_client.next().await {
            Some(Ok(Message::Text(text))) => assert!(matches!(
                serde_json::from_str::<RelayEnvelope>(&text).expect("connect should decode"),
                RelayEnvelope::ClientConnected { .. }
            )),
            other => panic!("unexpected reconnecting client connect response: {other:?}"),
        }
        reconnecting_client
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::ClientSubscribe {
                    request_id: "reconnect-subscribe".to_string(),
                    subscription_id: "recoverable-subscription".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-1".to_string()),
                        daemon_alias: None,
                    },
                    session_id: "session-1".to_string(),
                    attachment_id: "terminal".to_string(),
                    client_public_key: "reconnecting-client-public".to_string(),
                    subscription_scope: None,
                    resume_from_event_id: None,
                })
                .expect("reconnect subscribe should serialize")
                .into(),
            ))
            .await
            .expect("reconnect subscribe should send");
        let relay_request_id = match daemon_socket.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("reconnect daemon subscribe should decode")
            {
                RelayEnvelope::DaemonSubscribe {
                    relay_request_id,
                    relay_subscription_id,
                    ..
                } => {
                    assert_eq!(relay_subscription_id, "recoverable-subscription");
                    relay_request_id
                }
                other => panic!("unexpected reconnect subscribe envelope: {other:?}"),
            },
            other => panic!("unexpected reconnect subscribe frame: {other:?}"),
        };
        daemon_socket
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonResponse {
                    relay_request_id,
                    encrypted_response: None,
                    error: None,
                })
                .expect("reconnect subscribe response should serialize")
                .into(),
            ))
            .await
            .expect("reconnect subscribe response should send");
        match reconnecting_client.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("reconnect subscribe response should decode")
            {
                RelayEnvelope::ClientResponse {
                    request_id,
                    encrypted_response: None,
                    error: None,
                } => assert_eq!(request_id, "reconnect-subscribe"),
                other => panic!("unexpected reconnect subscribe response: {other:?}"),
            },
            other => panic!("unexpected reconnect subscribe response frame: {other:?}"),
        }
        assert_eq!(registry.read().await.subscription_count(), 1);

        let _ = reconnecting_client.close(None).await;
        let _ = daemon_socket.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }

    async fn assert_no_relay_close(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) {
        match timeout(Duration::from_millis(100), socket.next()).await {
            Err(_) => {}
            Ok(Some(Ok(Message::Text(text)))) => {
                let envelope = serde_json::from_str::<RelayEnvelope>(&text)
                    .expect("relay text frame should decode");
                assert!(
                    !matches!(envelope, RelayEnvelope::Close { .. }),
                    "active socket received relay close after accepted token expiry"
                );
            }
            Ok(other) => panic!("active socket closed unexpectedly: {other:?}"),
        }
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

        let server = RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        });
        let registry = server.registry();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_a, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon A should connect to relay");
        let (mut daemon_b, _) = connect_async_with_retry(&url)
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
                    provider_accounts: Vec::new(),
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
                    request_id: "".to_string(),
                    target: ClientTarget {
                        daemon_id: None,
                        daemon_alias: Some("beta".to_string()),
                    },
                    encrypted_request: encrypted_request.clone(),
                })
                .expect("invalid peer request should serialize")
                .into(),
            ))
            .await
            .expect("invalid peer request should send");
        match timeout(Duration::from_millis(500), daemon_a.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("invalid peer response should decode")
            {
                RelayEnvelope::DaemonPeerResponse {
                    request_id,
                    from_daemon_id,
                    encrypted_response: None,
                    error: Some(error),
                } => {
                    assert_eq!(request_id, "");
                    assert_eq!(from_daemon_id, "");
                    assert_eq!(error.code, "invalid_runtime_identifier");
                    assert!(!error.retryable);
                }
                other => panic!("unexpected invalid peer response: {other:?}"),
            },
            Ok(other) => panic!("unexpected invalid peer response frame: {other:?}"),
            Err(_) => panic!("invalid peer response was not delivered"),
        }
        match timeout(Duration::from_millis(100), daemon_b.next()).await {
            Err(_) => {}
            Ok(other) => panic!("invalid peer request reached target daemon: {other:?}"),
        }
        assert_eq!(registry.read().await.pending_request_count(), 0);

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

    #[tokio::test(flavor = "multi_thread")]
    async fn scoped_daemon_tokens_gate_peer_routing_actions() {
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
                "realm-a",
                vec![RelayAction::DaemonRegister],
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
                .run_listener_until(listener, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        });

        let url = format!("ws://{}:{}", addr.ip(), addr.port());
        let (mut daemon_a, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon A should connect");
        let (mut daemon_b, _) = connect_async_with_retry(&url)
            .await
            .expect("daemon B should connect");
        for (socket, daemon_id, auth_token) in [
            (&mut daemon_a, "daemon-a", "daemon-a-token"),
            (&mut daemon_b, "daemon-b", "daemon-b-token"),
        ] {
            socket
                .send(Message::Text(
                    serde_json::to_string(&RelayEnvelope::DaemonRegister {
                        registration: test_registration_with_token(
                            daemon_id,
                            "machine-1",
                            "Linux",
                            10,
                            auth_token,
                        ),
                    })
                    .expect("register should serialize")
                    .into(),
                ))
                .await
                .expect("register should send");
        }
        sleep(Duration::from_millis(50)).await;

        let encrypted_request = EncryptedRelayPayload {
            sender_public_key: "daemon-a-public".to_string(),
            nonce: "nonce".to_string(),
            ciphertext: "ciphertext".to_string(),
        };
        daemon_a
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonPeerRequest {
                    request_id: "peer-denied".to_string(),
                    target: ClientTarget {
                        daemon_id: Some("daemon-b".to_string()),
                        daemon_alias: None,
                    },
                    encrypted_request: encrypted_request.clone(),
                })
                .expect("peer request should serialize")
                .into(),
            ))
            .await
            .expect("peer request should send");
        match daemon_a.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<RelayEnvelope>(&text)
                .expect("peer denial should decode")
            {
                RelayEnvelope::DaemonPeerResponse {
                    request_id,
                    encrypted_response: None,
                    error: Some(error),
                    ..
                } => {
                    assert_eq!(request_id, "peer-denied");
                    assert_eq!(error.code, "action_not_allowed");
                }
                other => panic!("unexpected peer denial envelope: {other:?}"),
            },
            other => panic!("unexpected peer denial frame: {other:?}"),
        }
        match timeout(Duration::from_millis(100), daemon_b.next()).await {
            Err(_) => {}
            Ok(other) => panic!("unauthorized peer request reached target daemon: {other:?}"),
        }

        daemon_a
            .send(Message::Text(
                serde_json::to_string(&RelayEnvelope::DaemonPeerEvent {
                    target: ClientTarget {
                        daemon_id: Some("daemon-b".to_string()),
                        daemon_alias: None,
                    },
                    encrypted_event: encrypted_request,
                })
                .expect("peer event should serialize")
                .into(),
            ))
            .await
            .expect("peer event should send");
        match timeout(Duration::from_millis(100), daemon_b.next()).await {
            Err(_) => {}
            Ok(other) => panic!("unauthorized peer event reached target daemon: {other:?}"),
        }

        let _ = daemon_a.close(None).await;
        let _ = daemon_b.close(None).await;
        let _ = shutdown_tx.send(());
        server_task.await.expect("server task should join");
    }
}
