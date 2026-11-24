//! A slice start rebinds every attached agent to the restarted worker, but relaunches a provider
//! only for a native terminal that was running. Real local relay and worker kernel.
use super::*;

#[test]
fn a_restarted_slice_leaves_a_chariox_agent_provider_to_its_next_prompt() {
    crate::test_support::isolated_env_test!();
    std::thread::Builder::new()
        .name("slice-relaunch-interface".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(32 * 1024 * 1024)
                .build()
                .unwrap()
                .block_on(chariox_agent_is_rebound_without_a_native_terminal())
        })
        .unwrap()
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

async fn chariox_agent_is_rebound_without_a_native_terminal() {
    let server = chariox_relay::RelayServer::new(chariox_relay::RelayConfig {
        host: "127.0.0.1".into(),
        port: 0,
        shared_token: Some("slice-relaunch-test".into()),
    });
    let listener = server.bind_listener().await.unwrap();
    let addr = listener.local_addr().unwrap();
    let registry = server.registry();
    let (stop_relay, relay_stopped) = tokio::sync::oneshot::channel::<()>();
    let relay_task = tokio::spawn(async move {
        server
            .run_listener_until(listener, async {
                let _ = relay_stopped.await;
            })
            .await
            .unwrap();
    });
    let mut worker_config = crate::config::DaemonConfig::for_tests();
    worker_config.daemon_id = "slice-relaunch-worker".into();
    worker_config.host_machine_id = "slice-relaunch-machine".into();
    worker_config.relay_url = Some(format!("ws://{addr}"));
    worker_config.relay_token = Some("slice-relaunch-test".into());
    worker_config.accept_remote_leases = true;
    let worker = Arc::new(Mutex::new(
        DaemonApp::bootstrap(worker_config.clone()).unwrap(),
    ));
    let worker_relay_state = worker.lock().await.relay_client_state();
    let (stop_worker, worker_stopped) = tokio::sync::watch::channel(false);
    let worker_task = tokio::spawn(crate::transport::relay_client::run_daemon_relay_connector(
        worker.clone(),
        worker_relay_state,
        worker_stopped,
    ));
    for _ in 0..200 {
        if registry
            .read()
            .await
            .daemon(&worker_config.daemon_id)
            .is_some()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let worker_presence = crate::transport::relay_discovery::list_live_kernels_for_machine(
        &worker_config,
        &worker_config.host_machine_id,
    )
    .await
    .unwrap()
    .into_iter()
    .find(|kernel| kernel.kernel_id == worker_config.daemon_id)
    .expect("the restarted slice worker is live on the relay");

    let mut home_config = crate::config::DaemonConfig::for_tests();
    home_config.relay_url = worker_config.relay_url.clone();
    home_config.relay_token = worker_config.relay_token.clone();
    let mut home = DaemonApp::bootstrap(home_config).unwrap();
    let room = crate::test_support::TestWorktree::new("slice-relaunch-room");
    let slice_workspace = crate::test_support::TestWorktree::new("slice-relaunch-workspace");
    let (session, agent) = crate::app::KernelSessionService::new(&mut home)
        .create_session(room.session_request().with_agent_defaults(
            crate::session::SessionAgentDefaults {
                provider: "managed-dev-stub".into(),
                model: Some("native-tui-idle".into()),
                ..Default::default()
            },
        ))
        .unwrap();
    let (session_id, agent_id) = (session.id().to_string(), agent.id().to_string());
    let slice = home
        .slices()
        .create(
            "home-kernel",
            "home-machine",
            crate::slice::CreateSliceInput {
                source_slice_ref: None,
                name: "slice-relaunch".into(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".into(),
                display_mode: crate::slice::SliceDisplayMode::Headless,
                display_backend: Default::default(),
                workspace_id: None,
                worktree_id: None,
                workspace_mount: None,
                development: None,
                worker_kernel_ref: Some(worker_config.daemon_id.clone()),
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 1,
            },
        )
        .unwrap();
    let workspace = slice_workspace.path().display().to_string();
    home.slices()
        .set_development_publication(
            &slice.id,
            crate::slice::SliceDevelopmentPublication {
                publication_id: "publication-1".into(),
                destination_root: workspace.clone(),
                primary_repository_path: workspace,
                repository_paths: Vec::new(),
            },
            2,
        )
        .unwrap();
    home.slices()
        .set_worker_presence(
            &slice.id,
            Some(worker_config.daemon_id.clone()),
            Some(worker_config.host_machine_id.clone()),
            vec!["managed-dev-stub".into()],
            3,
        )
        .unwrap();
    let slice = home
        .slices()
        .attach_agent(&slice.id, &session_id, &agent_id, 4)
        .unwrap();
    // The worker restarted with the slice, so the lease recorded before it is gone.
    home.agents_mut()
        .bind_remote_execution(
            &agent_id,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: worker_config.daemon_id.clone(),
                worker_machine_id: worker_config.host_machine_id.clone(),
                execution_lease_id: "lease-before-restart".into(),
                leased_agent_id: "leased-before-restart".into(),
                active_worker_provider_run_id: Some("worker-run-before-restart".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
                worker_worktree_id: None,
            },
        )
        .unwrap();
    // The agent's prompts ran through Chariox, not in a native terminal.
    let projected_run_id = crate::provider::projected_leased_provider_run_id(
        "leased-before-restart",
        "worker-run-before-restart",
    );
    let mut projected = crate::provider::RuntimeProviderRun::new(
        &projected_run_id,
        &crate::provider::LaunchProviderRequest::new(
            &session_id,
            "managed-dev-stub",
            "managed-dev-stub",
            "default",
            "native-tui-idle",
        )
        .with_agent_id(&agent_id),
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "projected-chariox-run".into(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    projected.mark_running();
    home.provider_run_projection_store().update(projected);
    home.sessions_mut()
        .set_active_provider_run(&session_id, Some(projected_run_id))
        .unwrap();
    let home = Arc::new(Mutex::new(home));
    let runtime = super::tests::owned_runtime_state(&home).await;

    let manifests = runtime
        .slice_agent_relaunch_manifests(&slice, "slice.state.save")
        .unwrap();
    runtime
        .park_slice_agent_provider_runs(&manifests)
        .await
        .unwrap();
    let operation = runtime
        .begin_slice_operation(&slice.id, "slice.start")
        .unwrap();
    let relaunched = runtime
        .rebind_and_relaunch_slice_agents(manifests, &worker_presence, &operation)
        .await;
    drop(operation);
    let worker_runs = worker.lock().await.providers().list_runs();
    let rebound = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
    let _ = stop_worker.send(true);
    let _ = worker_task.await;
    let _ = stop_relay.send(());
    let _ = relay_task.await;

    relaunched.expect("the restarted slice rebinds its agent");
    assert_ne!(
        rebound.remote_execution().unwrap().execution_lease_id,
        "lease-before-restart",
        "the agent is bound to a lease on the restarted worker"
    );
    assert!(
        worker_runs.is_empty(),
        "a Chariox agent gets no native terminal that no client drives; its next prompt starts the provider: {:?}",
        worker_runs
            .iter()
            .map(|run| (run.id(), run.client_interface()))
            .collect::<Vec<_>>()
    );
}
