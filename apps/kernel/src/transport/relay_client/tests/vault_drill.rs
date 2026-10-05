//! Real scoped local relay used by the opt-in native Vault drill.
use super::support::*;
use chariox_relay::auth::{
    RelayAction, RelayAuthVerifier, RelaySubjectKind, RelayTokenClaims, ScopedTokenVerifier,
};

pub(crate) struct VaultRelayDrill {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    daemon: String,
    public_key: String,
    next: u64,
    shutdown: watch::Sender<bool>,
    stop: Option<oneshot::Sender<()>>,
    connector: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
}
impl VaultRelayDrill {
    pub(crate) async fn start(router: Arc<CommandRouter>, config: &DaemonConfig) -> Self {
        let claim = |name: &str, kind, actions| RelayTokenClaims {
            issuer: "md-vault-fixture".into(),
            subject: name.into(),
            subject_kind: kind,
            realm_id: "default".into(),
            allowed_actions: actions,
            allowed_targets: Some(vec![config.daemon_id.clone()]),
            issued_at_ms: 1,
            expires_at_ms: 100,
            token_id: name.into(),
            account_id: None,
            organization_id: None,
            user_id: Some("local".into()),
            device_id: None,
            machine_id: None,
            client_id: Some(name.into()),
            session_id: None,
            public_key_thumbprint: None,
            entitlements_version: None,
        };
        // Synthetic admissions exist only in fixture memory. Never load an account.
        let claims = BTreeMap::from([
            (
                "fixture-kernel".into(),
                claim(
                    &config.daemon_id,
                    RelaySubjectKind::Kernel,
                    vec![
                        RelayAction::DaemonRegister,
                        RelayAction::DaemonHeartbeat,
                        RelayAction::PacketRoute,
                    ],
                ),
            ),
            (
                "fixture-owner".into(),
                claim(
                    "owner",
                    RelaySubjectKind::Client,
                    vec![RelayAction::ClientConnect, RelayAction::PacketRoute],
                ),
            ),
        ]);
        let server = Arc::new(RelayServer::with_auth_verifier(
            RelayConfig {
                host: "127.0.0.1".into(),
                port: 0,
                shared_token: None,
            },
            RelayAuthVerifier::ScopedToken(ScopedTokenVerifier::new(
                claims,
                BTreeMap::new(),
                Some(10),
            )),
        ));
        let listener = server.bind_listener().await.unwrap();
        let address = listener.local_addr().unwrap();
        println!("MD-Vault local relay ws://{address} (disposable owner fixture)");
        let registry = server.registry();
        let (stop, stopped) = oneshot::channel();
        let server = tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let (shutdown, shutdown_rx) = watch::channel(false);
        let url = format!("ws://{address}");
        let connector = tokio::spawn(run_daemon_relay_connector_with_router_and_static_relay(
            router,
            Arc::new(RwLock::new(RelayClientState::default())),
            shutdown_rx,
            url.clone(),
            "fixture-kernel".into(),
        ));
        wait_for_daemon_registration(registry, &config.daemon_id).await;
        let (mut socket, _) = connect_async(url).await.unwrap();
        send_client_envelope(
            &mut socket,
            &RelayEnvelope::ClientConnect {
                auth_token: "fixture-owner".into(),
                target: ClientTarget {
                    daemon_id: Some(config.daemon_id.clone()),
                    daemon_alias: None,
                },
            },
        )
        .await;
        let public_key = expect_client_connected(&mut socket).await;
        Self {
            socket,
            daemon: config.daemon_id.clone(),
            public_key,
            next: 0,
            shutdown,
            stop: Some(stop),
            connector,
            server,
        }
    }
    pub(crate) async fn request(&mut self, request: LocalDaemonRequest) -> LocalDaemonResponse {
        self.next += 1;
        let id = format!("md-vault-relay-{}", self.next);
        let private_key = send_client_request(
            &mut self.socket,
            &id,
            &self.daemon,
            &self.public_key,
            request,
        )
        .await;
        tokio::time::timeout(
            Duration::from_secs(15),
            expect_client_response(&mut self.socket, &id, &private_key),
        )
        .await
        .expect("real kernel relay response required")
    }
    pub(crate) async fn close(mut self) {
        let _ = self.socket.close(None).await;
        let _ = self.shutdown.send(true);
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        tokio::time::timeout(Duration::from_secs(5), &mut self.connector)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), &mut self.server)
            .await
            .unwrap()
            .unwrap();
    }
}
