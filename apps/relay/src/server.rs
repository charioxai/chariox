use std::future::Future;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::RwLock;

use crate::auth::RelayAuthVerifier;
use crate::config::RelayConfig;

mod connection;
use connection::handle_connection;

pub use crate::registry::{ConnectedPeer, RelayRegistry};

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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::auth::{
        RelayAction, RelayAuthVerifier, RelaySubjectKind, RelayTokenClaims, ScopedTokenVerifier,
        DEFAULT_RELAY_REALM_ID,
    };
    use crate::protocol::{
        ClientTarget, DaemonRegistration, EncryptedRelayPayload, RelayDisplayTunnelRegistration,
        RelayEnvelope, RelayMetadataQuery,
    };
    use crate::registry::DaemonKey;
    use futures_util::{SinkExt, StreamExt};
    use std::collections::BTreeMap;
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
