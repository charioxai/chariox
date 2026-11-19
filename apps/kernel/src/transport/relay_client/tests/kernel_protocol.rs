use super::support::*;
use crate::local::{
    ApproveRemoteMachineRequest, ListRemoteMachineKernelsRequest, ListRemoteMachinesRequest,
    LocalDaemonRequest, LocalDaemonResponse, RemoteMachineKernel, RemoteMachineRecord,
    RemoteMachineTrustStatus, LOCAL_DAEMON_PROTOCOL_VERSION,
};

/// Registers like a kernel built before registrations carried their protocol: the field is
/// missing from the wire, not null.
async fn register_kernel_without_protocol(
    relay_url: &str,
    config: &DaemonConfig,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (mut socket, handshake) = connect_async(relay_url)
        .await
        .expect("the old kernel should connect to the relay");
    let register = crate::transport::relay_socket_connect::daemon_register(
        relay_url,
        &handshake,
        chariox_relay::protocol::DaemonRegistration {
            auth_token: "secret".to_string(),
            daemon_id: config.daemon_id.clone(),
            machine_id: config.host_machine_id.clone(),
            machine_alias: Some("old-machine".to_string()),
            os_name: Some("Linux".to_string()),
            kernel_started_at_ms: crate::session::unix_epoch_ms(),
            daemon_alias: None,
            kernel_alias: Some("old-kernel".to_string()),
            public_key: config.relay_public_key.clone(),
            capabilities: vec!["kernel_ws".to_string()],
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            accepting_remote_leases: true,
            leased_agent_count: 0,
            local_session_count: 0,
            local_protocol: None,
        },
        &config.relay_private_key,
    )
    .expect("the old kernel's registration should be proven");
    let wire = serde_json::to_value(&register).expect("registration should encode");
    assert!(
        wire.pointer("/registration/local_protocol").is_none(),
        "an old kernel's registration has no protocol field: {wire}"
    );
    socket
        .send(Message::Text(wire.to_string().into()))
        .await
        .expect("the old kernel's registration should send");
    socket
}

async fn listed_protocol(app: &Arc<Mutex<DaemonApp>>, machine_ref: &str) -> Option<u32> {
    let (config_projection, inventory) = {
        let app = app.lock().await;
        (
            app.config_projection_store(),
            app.remote_relay_inventory_projection_store(),
        )
    };
    let response =
        crate::runtime::remote_relay_inventory::execute_list_remote_machine_kernels_request(
            Arc::new(RwLock::new(RelayClientState::default())),
            config_projection,
            inventory,
            ListRemoteMachineKernelsRequest {
                machine_ref: machine_ref.to_string(),
            },
        )
        .await
        .expect("the machine's kernels should list");
    let LocalDaemonResponse::RemoteMachineKernelsListed { kernels, .. } = response else {
        panic!("expected the machine's kernels, got {response:?}");
    };
    let [kernel] = kernels.as_slice() else {
        panic!("expected one kernel on {machine_ref}, got {kernels:?}");
    };
    kernel.local_protocol
}

/// Approves a discovered machine the way a client does, through the kernel's command router.
async fn approve_machine(app: &Arc<Mutex<DaemonApp>>, machine_ref: &str) -> RemoteMachineRecord {
    let router = CommandRouter::with_interactive_capacity_from_app(Arc::clone(app), 1);
    let request = LocalDaemonRequest::ApproveRemoteMachine(ApproveRemoteMachineRequest {
        machine_ref: machine_ref.to_string(),
    });
    let command = crate::runtime::command::KernelCommand::from_local_request_with_caller(
        "approve-machine",
        crate::runtime::command::KernelCommandSource::LocalCli,
        router
            .local_command_caller(
                crate::runtime::command::KernelCommandSource::LocalCli,
                crate::local::KernelConnectionClass::Terminal,
            )
            .await,
        None,
        None,
        &request,
    );
    let response = router
        .dispatch(command, request)
        .await
        .expect("Approve should answer");
    let LocalDaemonResponse::RemoteMachineApproved { machine } = response else {
        panic!("expected the approved machine, got {response:?}");
    };
    machine
}

/// The kernels each listed machine names in the machine list clients follow.
async fn machine_list_kernels(
    app: &Arc<Mutex<DaemonApp>>,
) -> Vec<(String, Vec<RemoteMachineKernel>)> {
    let (config_projection, inventory) = {
        let app = app.lock().await;
        (
            app.config_projection_store(),
            app.remote_relay_inventory_projection_store(),
        )
    };
    let response = crate::runtime::remote_relay_inventory::execute_list_remote_machines_request(
        Arc::new(RwLock::new(RelayClientState::default())),
        config_projection,
        inventory,
        ListRemoteMachinesRequest,
    )
    .await
    .expect("the machines should list");
    let LocalDaemonResponse::RemoteMachinesListed { machines, .. } = response else {
        panic!("expected the machine list, got {response:?}");
    };
    machines
        .into_iter()
        .map(|machine| (machine.machine_id, machine.kernels))
        .collect()
}

// A client learns each remote kernel's protocol from the kernel's own relay registration;
// a kernel that predates the field lists no protocol, which clients show as unknown.
#[tokio::test(flavor = "multi_thread")]
async fn a_remote_kernel_lists_the_protocol_it_registered_with() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    let server = Arc::new(RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    }));
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };
    let relay_url = format!("ws://{addr}");
    let start_kernel = |daemon_id: &str, machine_id: &str| {
        let mut config = DaemonConfig::for_tests();
        config.daemon_id = daemon_id.to_string();
        config.host_machine_id = machine_id.to_string();
        config.relay_url = Some(relay_url.clone());
        config.relay_token = Some("secret".to_string());
        config.relay_heartbeat_ms = 50;
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app),
            Arc::new(RwLock::new(RelayClientState::default())),
            shutdown_rx,
        ));
        (config, app, shutdown_tx, task)
    };

    let (home, home_app, home_shutdown, home_task) = start_kernel("home-kernel", "home-machine");
    let (worker, _worker_app, worker_shutdown, worker_task) =
        start_kernel("worker-kernel", "worker-machine");
    let mut old = DaemonConfig::for_tests();
    old.daemon_id = "old-kernel".to_string();
    old.host_machine_id = "old-machine".to_string();
    let old_socket = register_kernel_without_protocol(&relay_url, &old).await;
    for daemon_id in [&home.daemon_id, &worker.daemon_id, &old.daemon_id] {
        wait_for_daemon_registration(registry.clone(), daemon_id).await;
    }
    refresh_remote_inventory_projection_for_app_with_relay_state(&home_app)
        .await
        .expect("the home kernel's inventory should refresh");

    assert_eq!(
        listed_protocol(&home_app, &worker.host_machine_id).await,
        Some(LOCAL_DAEMON_PROTOCOL_VERSION),
        "a current kernel lists the protocol it registered with"
    );
    assert_eq!(
        listed_protocol(&home_app, &old.host_machine_id).await,
        None,
        "a kernel that registered without a protocol lists none"
    );

    let mut machines = machine_list_kernels(&home_app).await;
    machines.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        machines,
        vec![
            (
                old.host_machine_id.clone(),
                vec![RemoteMachineKernel {
                    kernel_id: old.daemon_id.clone(),
                    local_protocol: None,
                }]
            ),
            (
                worker.host_machine_id.clone(),
                vec![RemoteMachineKernel {
                    kernel_id: worker.daemon_id.clone(),
                    local_protocol: Some(LOCAL_DAEMON_PROTOCOL_VERSION),
                }]
            ),
        ],
        "the machine list names each machine's kernels with their protocols"
    );

    // Approval changes trust only: the kernels the live pass found stay in the answer and the list.
    let worker_kernels = vec![RemoteMachineKernel {
        kernel_id: worker.daemon_id.clone(),
        local_protocol: Some(LOCAL_DAEMON_PROTOCOL_VERSION),
    }];
    let approved = approve_machine(&home_app, &worker.host_machine_id).await;
    assert_eq!(
        (&approved.trust_status, approved.pending, &approved.kernels),
        (&RemoteMachineTrustStatus::Approved, false, &worker_kernels),
        "the approval answer keeps the machine's kernels: {approved:?}"
    );
    let mut machines_after = machine_list_kernels(&home_app).await;
    machines_after.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        machines_after, machines,
        "the machine list keeps every kernel and protocol after an approval"
    );

    drop(old_socket);
    for (shutdown, task) in [(home_shutdown, home_task), (worker_shutdown, worker_task)] {
        let _ = shutdown.send(true);
        task.await.expect("connector should join");
    }
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}

// A kernel no client follows keeps the machine list it made when it connected. A client that
// makes it the source of its list later (its previous source dropped) is answered with what the
// relay lists then, not with that stale list, where machines that joined since are missing.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_list_asked_for_after_it_went_stale_is_the_live_one() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    let server = Arc::new(RelayServer::new(RelayConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        shared_token: Some("secret".to_string()),
    }));
    let listener = server
        .bind_listener()
        .await
        .expect("relay listener should bind");
    let addr = listener.local_addr().expect("listener should have addr");
    let registry = server.registry();
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();
    let server_task = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            server
                .run_listener_until(listener, async {
                    let _ = server_shutdown_rx.await;
                })
                .await
                .expect("relay server should run");
        })
    };
    let relay_url = format!("ws://{addr}");
    let start_kernel = |daemon_id: &str, machine_id: &str| {
        let mut config = DaemonConfig::for_tests();
        config.daemon_id = daemon_id.to_string();
        config.host_machine_id = machine_id.to_string();
        config.relay_url = Some(relay_url.clone());
        config.relay_token = Some("secret".to_string());
        config.relay_heartbeat_ms = 50;
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
        ));
        let relay_state = Arc::new(RwLock::new(RelayClientState::default()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(run_daemon_relay_connector(
            Arc::clone(&app),
            Arc::clone(&relay_state),
            shutdown_rx,
        ));
        (config, app, relay_state, shutdown_tx, task)
    };
    let (home, home_app, home_relay, home_shutdown, home_task) =
        start_kernel("home-kernel", "home-machine");
    wait_for_daemon_registration(registry.clone(), &home.daemon_id).await;
    let (config_projection, inventory) = {
        let app = home_app.lock().await;
        (
            app.config_projection_store(),
            app.remote_relay_inventory_projection_store(),
        )
    };
    // The pass the home kernel makes when it connects lists no other machine yet.
    timeout(Duration::from_secs(5), async {
        while !inventory.is_fresh(crate::session::unix_epoch_ms(), 60_000) {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the home kernel's connect pass should publish");
    assert!(inventory.inventory().machines.is_empty());

    let (worker, _worker_app, _worker_relay, worker_shutdown, worker_task) =
        start_kernel("worker-kernel", "worker-machine");
    wait_for_daemon_registration(registry.clone(), &worker.daemon_id).await;
    // No client follows the home kernel's list, so nothing renews it: it goes stale.
    sleep(Duration::from_millis(1_200)).await;

    let response = crate::runtime::remote_relay_inventory::execute_list_remote_machines_request(
        Arc::clone(&home_relay),
        config_projection,
        inventory,
        ListRemoteMachinesRequest,
    )
    .await
    .expect("the machines should list");
    let LocalDaemonResponse::RemoteMachinesListed { machines, .. } = response else {
        panic!("expected the machine list, got {response:?}");
    };
    assert_eq!(
        machines
            .iter()
            .map(|machine| machine.machine_id.as_str())
            .collect::<Vec<_>>(),
        vec![worker.host_machine_id.as_str()],
        "the answer is what the relay lists now"
    );

    for (shutdown, task) in [(home_shutdown, home_task), (worker_shutdown, worker_task)] {
        let _ = shutdown.send(true);
        task.await.expect("connector should join");
    }
    let _ = server_shutdown_tx.send(());
    server_task.await.expect("server task should join");
}
