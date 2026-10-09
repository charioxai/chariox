//! MD-DISPLAY-02/04: opt-in real kernel websocket + focused MCP browser fixture.
use super::*;
use futures_util::FutureExt;
use std::time::Instant;
#[test]
#[ignore = "MD-DISPLAY: requires external disposable state, sandboxed Chromium and presenter drill"]
fn kernel_browser_display_protocol_drill() {
    run_test(|| Box::pin(live_display()));
}
async fn live_display() {
    let root = std::path::PathBuf::from(std::env::var_os("CHARIOX_DISPLAY_DRILL_ROOT").unwrap());
    let home = std::path::PathBuf::from(std::env::var_os("CHARIOX_HOME").unwrap());
    assert!(root.is_absolute() && home.starts_with(&root));
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = drill_config(&home);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    config.kernel_websocket_port = listener.local_addr().unwrap().port();
    config.runtime_mcp_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    // MD-DISPLAY: disposable local relay, using the product scoped-token API.
    // Issuer material exists only in this run's memory, never output/evidence.
    use chariox_relay::auth::{
        encode_scoped_hmac_token, RelayAction, RelayAuthVerifier, RelaySubjectKind,
        RelayTokenClaims,
    };
    let issuer_secret: String = rand::random::<[u8; 32]>()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let now = crate::session::unix_epoch_ms();
    let claims = |client: bool| RelayTokenClaims {
        issuer: "md-display-disposable".into(),
        subject: if client {
            "display-viewer".into()
        } else {
            config.daemon_id.clone()
        },
        subject_kind: if client {
            RelaySubjectKind::Client
        } else {
            RelaySubjectKind::Kernel
        },
        realm_id: "default".into(),
        allowed_actions: if client {
            vec![RelayAction::ClientConnect, RelayAction::PacketRoute]
        } else {
            vec![
                RelayAction::DaemonRegister,
                RelayAction::DaemonHeartbeat,
                RelayAction::PacketRoute,
            ]
        },
        allowed_targets: Some(vec![config.daemon_id.clone()]),
        issued_at_ms: now,
        // MP-08/MP-10/MP-11: the disposable grant must outlive the 600s
        // server watchdog plus startup; expired grants still fail closed.
        expires_at_ms: now + 900_000,
        token_id: if client {
            "md-display-client".into()
        } else {
            "md-display-kernel".into()
        },
        account_id: None,
        organization_id: None,
        user_id: Some(crate::session::DEFAULT_LOCAL_USER_ID.into()),
        device_id: None,
        machine_id: None,
        client_id: None,
        session_id: None,
        public_key_thumbprint: None,
        entitlements_version: None,
    };
    let client_token = encode_scoped_hmac_token(&claims(true), &issuer_secret).unwrap();
    let daemon_token = encode_scoped_hmac_token(&claims(false), &issuer_secret).unwrap();
    let relay = chariox_relay::server::RelayServer::with_auth_verifier(
        chariox_relay::config::RelayConfig {
            host: "127.0.0.1".into(),
            port: 0,
            shared_token: None,
        },
        RelayAuthVerifier::scoped_hmac(
            std::collections::BTreeMap::from([("md-display-disposable".into(), issuer_secret)]),
            None,
        ),
    );
    let relay_listener = relay.bind_listener().await.unwrap();
    let relay_url = format!("ws://{}", relay_listener.local_addr().unwrap());
    config.relay_url = Some(relay_url.clone());
    config.relay_token = Some(daemon_token.clone());
    let daemon_id = config.daemon_id.clone();
    let (relay_stop_tx, relay_stop_rx) = tokio::sync::oneshot::channel();
    let relay_task = tokio::spawn(async move {
        relay
            .run_listener_until(relay_listener, async {
                let _ = relay_stop_rx.await;
            })
            .await
            .unwrap();
    });
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "default",
    );
    let token = run.runtime_mcp_auth_token().unwrap().to_string();
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        4,
    ));
    let (connector_stop_tx, connector_stop_rx) = tokio::sync::watch::channel(false);
    let connector_task = tokio::spawn(
        crate::transport::relay_client::run_daemon_relay_connector_with_router_and_static_relay(
            router.clone(),
            Arc::new(tokio::sync::RwLock::new(
                crate::transport::relay_client::RelayClientState::default(),
            )),
            connector_stop_rx,
            relay_url.clone(),
            daemon_token,
        ),
    );
    let outcome = std::panic::AssertUnwindSafe(async {
        focus(&router, session.id(), agent.id()).await;
        router.dispatch_authenticated_runtime_tool_call(&token, "chariox.load_kernel_browser", json!({})).await.unwrap();
        let opened = router.dispatch_authenticated_runtime_tool_call(&token, "chariox.kernel_browser",
            json!({"command":{"op":"open","url":std::env::var("CHARIOX_DISPLAY_FIXTURE_URL").unwrap()}})).await.unwrap().payload;
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        // Bootstrap is private disposable runtime state, never validation evidence.
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            use std::io::Write;
            let mut bootstrap = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(root.join("relay-bootstrap.private.json")).unwrap();
            bootstrap.write_all(&serde_json::to_vec(&json!({"relay_url":relay_url,"daemon_id":daemon_id,"client_token":client_token})).unwrap()).unwrap();
        }
        std::fs::write(root.join("ready.json"), serde_json::to_vec(&json!({"endpoint":endpoint,"tab_id":opened["tab_id"],"generation":opened["generation"],"protocol":crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,"opened_by":"focused-runtime-MCP/dev-stub"})).unwrap()).unwrap();
        let stop = root.join("STOP");
        let probe_router = router.clone();
        crate::runtime_transport::run_kernel_websocket_server_with_router_on_listener(router.clone(), listener, async move {
            // MP-08/MP-10/MP-11: DPR2 masking campaigns can exceed four
            // minutes. STOP still ends the owned drill immediately; keep a
            // finite watchdog without truncating the unchanged 70-cycle gate.
            let deadline = Instant::now() + Duration::from_secs(600);
            while !stop.exists() && Instant::now() < deadline {
                for name in ["PROBE_TAKEOVER", "PROBE_RELEASE"] {
                    let result_file = root.join(format!("{name}.json"));
                    if root.join(name).exists() && !result_file.exists() {
                        // MP-08/MP-10: a viewer-scale Chromium restart (DPR2) rotates
                        // the generation; an agent probe refreshes state like an agent.
                        let state = probe_router.dispatch_authenticated_runtime_tool_call(&token, "chariox.kernel_browser", json!({"command":{"op":"state"}})).await.unwrap().payload;
                        let generation = state["generation"].clone();
                        let observed = probe_router.dispatch_authenticated_runtime_tool_call(&token, "chariox.kernel_browser", json!({"command":{"op":"screenshot","tab_id":opened["tab_id"],"generation":generation}})).await.unwrap().payload;
                        let result = probe_router.dispatch_authenticated_runtime_tool_call(&token, "chariox.kernel_browser", json!({"document_id":observed["document_id"],"command":{"op":"input","tab_id":opened["tab_id"],"generation":generation,"input":{"kind":"key","key":"Tab"}}})).await;
                        let fenced = result.as_ref().err().is_some_and(|error| matches!(error, crate::error::DaemonError::UserDomainRefused { reason: crate::error::UserDomainRefusalReason::NotGranted }));
                        std::fs::write(result_file, serde_json::to_vec(&json!({"rejected":result.is_err(),"takeover_fenced":fenced,"observed_document":observed["document_id"].is_string()})).unwrap()).unwrap();
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }).await.unwrap();
    }).catch_unwind().await;
    let _ = connector_stop_tx.send(true);
    connector_task.await.unwrap();
    let _ = relay_stop_tx.send(());
    relay_task.await.unwrap();
    router.runtime_state.shutdown_cleanup().await.unwrap();
    if let Err(error) = outcome {
        std::panic::resume_unwind(error);
    }
}
