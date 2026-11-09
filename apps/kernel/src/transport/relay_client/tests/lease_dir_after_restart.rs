//! MP-08 / MP-09 / MP-10 / MP-11: a remote child keeps its working directory
//! when its binding is renewed after the worker kernel restarted.
//! Real local relay and worker kernel with the supplementary managed-dev-stub provider.
use super::support::*;

#[test]
fn a_rebound_remote_child_keeps_its_working_directory_after_the_worker_restarts() {
    run_async_with_large_test_stack("lease-dir-after-restart", rebind_keeps_dir_async);
}

async fn rebind_keeps_dir_async() {
    let _relay_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    let server = RelayServer::new(RelayConfig {
        host: "127.0.0.1".into(),
        port: 0,
        shared_token: Some("lease-dir-test".into()),
    });
    let listener = server.bind_listener().await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = server.registry();
    let (stop_tx, stop_rx) = oneshot::channel();
    let relay_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = stop_rx.await;
            })
            .await
            .unwrap();
    });
    let mut config = DaemonConfig::for_tests();
    config.daemon_id = "lease-dir-worker".into();
    config.host_machine_id = "lease-dir-machine".into();
    config.relay_url = Some(format!("ws://{addr}"));
    config.relay_token = Some("lease-dir-test".into());
    config.accept_remote_leases = true;
    let worker = Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap()));
    let worker_state = worker.lock().await.relay_client_state();
    let (worker_stop_tx, worker_stop_rx) = watch::channel(false);
    let worker_task = tokio::spawn(run_daemon_relay_connector(
        worker.clone(),
        worker_state,
        worker_stop_rx,
    ));
    wait_for_daemon_registration(registry, &config.daemon_id).await;
    let presence = relay_discovery::list_live_kernels_for_machine(&config, &config.host_machine_id)
        .await
        .unwrap()
        .into_iter()
        .find(|k| k.kernel_id == config.daemon_id)
        .unwrap();
    let mut home_config = DaemonConfig::for_tests();
    home_config.relay_url = config.relay_url.clone();
    home_config.relay_token = config.relay_token.clone();
    let mut home = DaemonApp::bootstrap(home_config).unwrap();
    let room_dir = crate::test_support::TestWorktree::new("lease-dir-room");
    let child_dir = crate::test_support::TestWorktree::new("lease-dir-child");
    let child_dir = child_dir.path().display().to_string();
    let (_session, agent) = crate::app::KernelSessionService::new(&mut home)
        .create_session(room_dir.session_request().with_agent_defaults(
            crate::session::SessionAgentDefaults {
                provider: "managed-dev-stub".into(),
                model: Some("native-tui-idle".into()),
                ..Default::default()
            },
        ))
        .unwrap();
    // The child was spawned on the worker with `--dir`; the worker then restarted
    // and lost its live lease, so the home renews the binding.
    home.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: config.daemon_id.clone(),
                worker_machine_id: config.host_machine_id.clone(),
                execution_lease_id: "lease-before-restart".into(),
                leased_agent_id: "leased-before-restart".into(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
                worker_worktree_id: Some(child_dir.clone()),
            },
        )
        .unwrap();
    let rebound = home
        .refresh_remote_agent_binding_to_worker_kernel(agent.id(), &presence)
        .expect("the binding is renewed on the restarted worker");
    let directories =
        RemoteLeaseRuntime::new(&mut *worker.lock().await).leased_agent_working_directories();
    let _ = worker_stop_tx.send(true);
    let _ = worker_task.await;
    let _ = stop_tx.send(());
    let _ = relay_task.await;
    assert_eq!(
        directories.as_slice(),
        std::slice::from_ref(&child_dir),
        "the renewed child runs in its --dir, not in the worker's launch directory"
    );
    assert_eq!(
        rebound.remote_execution().unwrap().worker_worktree_id,
        Some(child_dir),
        "the renewed binding keeps the folder for the next renewal"
    );
}
