use super::*;
use chariox_relay::protocol::RelayEnvelope;
use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify};
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_native_launch_rechecks_after_temporary_discovery() {
    revoked_native_launch(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kernel_access_native_launch_rechecks_after_connected_uncached_discovery() {
    revoked_native_launch(true).await;
}

async fn receive(socket: &mut WebSocketStream<TcpStream>) -> RelayEnvelope {
    let message = timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

async fn send(socket: &mut WebSocketStream<TcpStream>, envelope: RelayEnvelope) {
    socket
        .send(Message::Text(
            serde_json::to_string(&envelope).unwrap().into(),
        ))
        .await
        .unwrap();
}

fn worker_reply(
    worker: &crate::config::DaemonConfig,
    home_public_key: &str,
    encrypted_request: &chariox_relay::protocol::EncryptedRelayPayload,
    launches: &AtomicUsize,
) -> chariox_relay::protocol::EncryptedRelayPayload {
    let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
        &worker.relay_private_key,
        encrypted_request,
    )
    .unwrap();
    let request: crate::transport::relay_peer::RelayPeerRequest =
        serde_json::from_slice(&decrypted.plaintext).unwrap();
    assert!(matches!(
        request,
        crate::transport::relay_peer::RelayPeerRequest::LaunchLeasedNativeProviderRun { .. }
    ));
    launches.fetch_add(1, Ordering::SeqCst);
    // Metadata only: exercise home projection without launching a provider CLI.
    let launch = crate::provider::LaunchProviderRequest::new(
        "worker-session",
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id("worker-agent")
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "worker-native-run",
        &launch,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    let response =
        crate::transport::relay_peer::RelayPeerResponse::LeasedNativeProviderRunLaunched {
            provider_run: run,
        };
    crate::transport::relay_crypto::encrypt_payload_for_peer(
        &worker.relay_private_key,
        home_public_key,
        &serde_json::to_vec(&response).unwrap(),
    )
    .unwrap()
}

async fn revoked_native_launch(connected: bool) {
    let worktree = crate::test_support::TestWorktree::new("access-native-discovery");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = crate::config::DaemonConfig::for_tests();
    let relay_url = format!("ws://{}", listener.local_addr().unwrap());
    config.relay_url = Some(relay_url.clone());
    config.relay_token = Some("native-discovery-fixture".into());
    let home_public_key = config.relay_public_key.clone();
    let worker = crate::config::DaemonConfig::for_tests();
    let worker_id = format!("native-authority-worker-{:016x}", rand::random::<u64>());
    let mut daemon = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut daemon)
        .create_session(worktree.session_request())
        .unwrap();
    daemon
        .agents_mut()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: worker_id.clone(),
                worker_machine_id: "fixture-machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased-agent".into(),
                active_worker_provider_run_id: Some("worker-existing-run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    let app = Arc::new(Mutex::new(daemon));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(app, 32);
    let state = router.runtime_state();
    let grant = state.insert_access_grant_for_test(session.id());
    let launches = Arc::new(AtomicUsize::new(0));
    let discovery_started = Arc::new(Notify::new());
    let release_discovery = Arc::new(Notify::new());
    let metadata_server = tokio::spawn({
        let worker = worker.clone();
        let worker_id = worker_id.clone();
        let home_public_key = home_public_key.clone();
        let launches = launches.clone();
        let discovery_started = discovery_started.clone();
        let release_discovery = release_discovery.clone();
        async move {
            let mut first_discovery = true;
            loop {
                let mut socket = accept_async(listener.accept().await.unwrap().0)
                    .await
                    .unwrap();
                match receive(&mut socket).await {
                    RelayEnvelope::ClientMetadataRequest { request_id, .. } => {
                        if first_discovery {
                            first_discovery = false;
                            discovery_started.notify_one();
                            release_discovery.notified().await;
                        }
                        let presence = serde_json::from_value(serde_json::json!({
                            "kernel_id":worker_id, "machine_id":"fixture-machine", "public_key":worker.relay_public_key
                        })).unwrap();
                        send(
                            &mut socket,
                            RelayEnvelope::ClientMetadataResponse {
                                request_id,
                                machines: None,
                                kernels: None,
                                kernel: Some(presence),
                                error: None,
                            },
                        )
                        .await;
                    }
                    RelayEnvelope::DaemonRegister { .. } => {
                        let RelayEnvelope::DaemonPeerRequest {
                            request_id,
                            encrypted_request,
                            ..
                        } = receive(&mut socket).await
                        else {
                            panic!("expected native launch request")
                        };
                        let response =
                            worker_reply(&worker, &home_public_key, &encrypted_request, &launches);
                        send(
                            &mut socket,
                            RelayEnvelope::DaemonPeerResponse {
                                request_id,
                                from_daemon_id: worker_id.clone(),
                                encrypted_response: Some(response),
                                error: None,
                            },
                        )
                        .await;
                    }
                    _ => panic!("unexpected fixture envelope"),
                }
                assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
                let _ = socket.close(None).await;
            }
        }
    });
    let connected_worker = if connected {
        let relay_state = state.owned.relay_state.clone();
        let (sender, mut priority_rx, _event_rx) =
            crate::transport::relay_client::RelayOutgoingSender::channel(4);
        relay_state
            .write()
            .await
            .test_set_connected_sender(sender, relay_url);
        // Deliberately leave the worker key uncached so this path must discover it.
        Some(tokio::spawn({
            let launches = launches.clone();
            let worker_id = worker_id.clone();
            async move {
                while let Some(envelope) = priority_rx.recv().await {
                    let RelayEnvelope::DaemonPeerRequest {
                        request_id,
                        encrypted_request,
                        ..
                    } = envelope
                    else {
                        panic!("expected connected native launch")
                    };
                    let response =
                        worker_reply(&worker, &home_public_key, &encrypted_request, &launches);
                    crate::transport::relay_client::resolve_pending_peer_response_for_test(
                        &relay_state,
                        request_id,
                        worker_id.clone(),
                        response,
                    )
                    .await;
                }
            }
        }))
    } else {
        None
    };
    let request = crate::local::LaunchProviderRunRequest {
        session_id: session.id().into(),
        agent_id: Some(agent.id().into()),
        adapter_key: "claude".into(),
        provider: "claude".into(),
        account_profile: "default".into(),
        model: "sonnet".into(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: true,
    };
    let pending = tokio::spawn({
        let state = state.clone();
        let request = request.clone();
        let grant = grant.clone();
        async move {
            state
                .launch_remote_native_provider_run_with_grant(
                    &request,
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    Some(&grant),
                )
                .await
        }
    });
    timeout(Duration::from_secs(5), discovery_started.notified())
        .await
        .unwrap();
    state
        .revoke_kernel_access(None, Some(&grant), "explicit_revoke")
        .unwrap();
    release_discovery.notify_one();
    let external_result = timeout(Duration::from_secs(5), pending)
        .await
        .unwrap()
        .unwrap();
    let external_launches = launches.load(Ordering::SeqCst);
    let terminal_result = timeout(
        Duration::from_secs(5),
        state.launch_remote_native_provider_run(&request, crate::session::DEFAULT_LOCAL_USER_ID),
    )
    .await
    .unwrap();
    metadata_server.abort();
    let _ = metadata_server.await;
    if let Some(task) = connected_worker {
        task.abort();
        let _ = task.await;
    }
    assert_eq!(
        external_launches, 0,
        "revoked external native launch reached the worker"
    );
    assert!(external_result
        .unwrap_err()
        .to_string()
        .contains("grant revoked or expired"));
    assert!(matches!(
        terminal_result.unwrap(),
        Some(LocalDaemonResponse::ProviderRunLaunched { .. })
    ));
    assert_eq!(
        launches.load(Ordering::SeqCst),
        1,
        "terminal launch should reach the worker"
    );
}
