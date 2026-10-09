//! MP-08/MP-10/MP-11: a controlled advertisement over the real encrypted relay.
//! Both workers use the current kernel dispatcher; v70 changes only the lease
//! advertisement, avoiding any artifact/upload dispatch to an obsolete worker.
use super::support::*;
use futures_util::FutureExt;
use std::sync::atomic::{AtomicU32, Ordering};
// MP-11: the admission floor moves with relay peer bumps; test around it.
const FLOOR: u32 = crate::transport::relay_peer::MINIMUM_RELAY_PEER_RUNTIME_VERSION;

#[test]
fn mp08_mp10_mp11_browser_artifact_peer_73_encrypted_lease_admission() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack("browser-artifact-peer-version", check);
}

fn config(root: &std::path::Path, name: &str) -> DaemonConfig {
    let root = root.join(name);
    std::fs::create_dir_all(&root).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.user_config_path = root.join("config.toml");
    config.local_socket_path = root.join("kernel.sock");
    // MP-08/MP-10/MP-11: isolated test temp roots can exceed Unix sockaddr bounds.
    #[cfg(unix)]
    {
        config.local_socket_path = std::path::PathBuf::from("/tmp").join(format!(
            "cx-browser-peer-{}-{}-{name}.sock",
            std::process::id(),
            rand::random::<u64>()
        ));
    }
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    config.user_config.history.operational.path =
        Some(root.join("history.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    config.with_session_history_root(root.join("sessions"))
}

async fn check() {
    let _guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    let workspace = crate::test_support::TestWorktree::new("browser-peer-version");
    let server = Arc::new(RelayServer::new(RelayConfig {
        host: "127.0.0.1".into(),
        port: 0,
        shared_token: Some("peer-version-fixture".into()),
    }));
    let listener = server.bind_listener().await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = server.registry();
    let (stop_server, stopped) = oneshot::channel();
    let relay = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let mut worker_config = config(workspace.path(), "worker");
    worker_config.daemon_id = "artifact-version-worker".into();
    worker_config.host_machine_id = "artifact-version-machine".into();
    worker_config.accept_remote_leases = true;
    let worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(worker_config.clone()).unwrap(),
    ));
    let router = Arc::new(
        crate::runtime::router::CommandRouter::with_interactive_capacity(worker.clone(), 2),
    );
    let mut home_config = config(workspace.path(), "home");
    home_config.daemon_id = "artifact-version-home".into();
    home_config.relay_url = Some(format!("ws://{addr}"));
    home_config.relay_token = Some("peer-version-fixture".into());
    home_config.relay_request_timeout_ms = 2_000;
    let version = Arc::new(AtomicU32::new(FLOOR - 1));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (stop_worker, mut stopped) = watch::channel(false);
    let target = {
        let version = version.clone();
        let requests = requests.clone();
        let worker_config = worker_config.clone();
        let home_private_key = home_config.relay_private_key.clone();
        tokio::spawn(async move {
            let (mut socket, _) = connect_async(format!("ws://{addr}")).await.unwrap();
            let registration = chariox_relay::protocol::DaemonRegistration {
                auth_token: "peer-version-fixture".into(),
                daemon_id: worker_config.daemon_id.clone(),
                machine_id: worker_config.host_machine_id.clone(),
                machine_alias: None,
                os_name: Some(worker_config.os_name.clone()),
                kernel_started_at_ms: crate::session::unix_epoch_ms(),
                daemon_alias: None,
                kernel_alias: None,
                public_key: worker_config.relay_public_key.clone(),
                capabilities: vec!["kernel_websocket".into(), "relay_peer_transport".into()],
                available_providers: vec!["managed-dev-stub".into()],
                provider_accounts: Vec::new(),
                accepting_remote_leases: true,
                leased_agent_count: 0,
                local_session_count: 0,
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&RelayEnvelope::DaemonRegister { registration })
                        .unwrap()
                        .into(),
                ))
                .await
                .unwrap();
            let state = Arc::new(RwLock::new(RelayClientState::default()));
            let (outgoing, _priority, _events) = RelayOutgoingSender::channel(32);
            let mut heartbeat = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! {
                    _ = heartbeat.tick() => {
                        socket.send(Message::Text(serde_json::to_string(&RelayEnvelope::DaemonHeartbeat {
                            daemon_id: worker_config.daemon_id.clone(), registration: None,
                        }).unwrap().into())).await.unwrap();
                    }
                    _ = stopped.changed() => { socket.close(None).await.unwrap(); break; }
                    message = socket.next() => {
                        let Some(Ok(Message::Text(text))) = message else { continue; };
                        let RelayEnvelope::DaemonIncomingPeerRequest {
                            relay_request_id, from_daemon_id, caller_identity, encrypted_request, ..
                        } = serde_json::from_str(&text).unwrap() else { continue; };
                        let decrypted = relay_crypto::decrypt_payload_for_private_key(
                            &worker_config.relay_private_key, &encrypted_request,
                        ).unwrap();
                        let request: serde_json::Value = serde_json::from_slice(&decrypted.plaintext).unwrap();
                        requests.lock().await.push(request["kind"].as_str().unwrap().to_string());
                        // Production authorization, lease creation/spawn/destruction and encryption.
                        let outcome = super::super::peer_requests::handle_daemon_peer_request(
                            &router, &state, &outgoing, &from_daemon_id, caller_identity, encrypted_request,
                        ).await;
                        let encrypted_response = outcome.encrypted_response.map(|encrypted| {
                            let clear = relay_crypto::decrypt_payload_for_private_key(
                                &home_private_key, &encrypted,
                            ).unwrap();
                            let mut response: serde_json::Value = serde_json::from_slice(&clear.plaintext).unwrap();
                            if response["kind"] == "execution_lease_created" {
                                response["relay_peer_protocol_version"] = version.load(Ordering::SeqCst).into();
                            }
                            relay_crypto::encrypt_payload_for_peer(&worker_config.relay_private_key,
                                &decrypted.sender_public_key, &serde_json::to_vec(&response).unwrap()).unwrap()
                        });
                        socket.send(Message::Text(serde_json::to_string(&RelayEnvelope::DaemonIncomingPeerResponse {
                            relay_request_id, encrypted_response, error: outcome.error,
                        }).unwrap().into())).await.unwrap();
                    }
                }
            }
        })
    };
    wait_for_daemon_registration(registry, &worker_config.daemon_id).await;
    let assertions = std::panic::AssertUnwindSafe(async {
        for advertised in [FLOOR - 1, FLOOR] {
            version.store(advertised, Ordering::SeqCst);
            requests.lock().await.clear();
            let config = home_config.clone();
            let workspace = workspace.path().to_path_buf();
            let result = tokio::task::spawn_blocking(move || {
                let mut app = DaemonApp::bootstrap(config).unwrap();
                let room = crate::app::KernelSessionService::new(&mut app)
                    .create_session(CreateSessionRequest::new(
                        workspace.display().to_string(),
                        workspace.display().to_string(),
                    ))
                    .unwrap()
                    .0;
                let result = app.spawn_worker_agent(
                    CreateAgentRequest::new(room.id(), "managed-dev-stub"),
                    "artifact-version-worker",
                    &|| Ok(()),
                );
                if let Ok(agent) = &result {
                    let binding = agent.remote_execution().unwrap();
                    app.destroy_remote_execution_binding(binding, &|| Ok(()))
                        .unwrap();
                    if advertised == FLOOR {
                        assert_eq!(binding.relay_peer_protocol_version, Some(FLOOR));
                        app.ensure_remote_agent_binding_protocol(binding).unwrap();
                    }
                }
                result
            })
            .await
            .unwrap();
            if advertised < FLOOR {
                let error =
                    result.expect_err("v70 must be rejected before spawn or artifact dispatch");
                assert!(
                    error
                        .to_string()
                        .contains(&format!("protocol {}", FLOOR - 1)),
                    "{error}; request kinds: {:?}",
                    *requests.lock().await
                );
                assert!(error.to_string().contains(&format!("requires {FLOOR}")));
                assert_eq!(
                    *requests.lock().await,
                    ["create_execution_lease", "destroy_execution_lease"]
                );
            } else {
                assert!(
                    result.is_ok(),
                    "v73 must bind: {}",
                    result
                        .as_ref()
                        .err()
                        .map(ToString::to_string)
                        .unwrap_or_default()
                );
                assert_eq!(
                    *requests.lock().await,
                    [
                        "create_execution_lease",
                        "spawn_leased_agent",
                        "destroy_leased_agent",
                        "destroy_execution_lease"
                    ]
                );
            }
            assert_eq!(
                RemoteLeaseRuntime::new(&mut *worker.lock().await).execution_lease_count(),
                0
            );
        }
    })
    .catch_unwind()
    .await;
    let _ = stop_worker.send(true);
    target.await.unwrap();
    let _ = stop_server.send(());
    relay.await.unwrap();
    if let Err(panic) = assertions {
        std::panic::resume_unwind(panic);
    }
}
