#![allow(unused_imports)]
use super::support::*;
use crate::config::PersistedMachineRegistration;
use crate::local::RemoteMachinePresence;

const MINUTE_MS: u64 = 60_000;

/// Remembers one machine seen 2 minutes ago (reconnecting) and one seen 11 minutes ago (offline).
fn remember_two_machines() {
    let now_ms = crate::session::unix_epoch_ms();
    DaemonConfig::update_machine_registry(|registry| {
        for (machine_id, age_minutes) in [("machine-recent", 2), ("machine-old", 11)] {
            registry.push(PersistedMachineRegistration {
                machine_id: machine_id.to_string(),
                approved: true,
                last_seen_ms: now_ms - age_minutes * MINUTE_MS,
                display_name: Some(format!("{machine_id} name")),
                ..PersistedMachineRegistration::default()
            });
        }
    })
    .expect("remembered machines should persist");
}

async fn remembered_inventory(app: &Arc<Mutex<DaemonApp>>) -> (Vec<String>, Vec<String>) {
    let inventory = app
        .lock()
        .await
        .remote_relay_inventory_projection_store()
        .inventory();
    (
        inventory
            .machines
            .iter()
            .filter(|machine| machine.presence == RemoteMachinePresence::Reconnecting)
            .map(|machine| machine.machine_id.clone())
            .collect(),
        inventory
            .offline_machines
            .iter()
            .map(|machine| machine.machine_id.clone())
            .collect(),
    )
}

async fn wait_for_remembered_within(app: &Arc<Mutex<DaemonApp>>, window: Duration) {
    let deadline = tokio::time::Instant::now() + window;
    while tokio::time::Instant::now() < deadline {
        let (reconnecting, offline) = remembered_inventory(app).await;
        if reconnecting == ["machine-recent"] && offline == ["machine-old"] {
            return;
        }
        sleep(Duration::from_millis(50)).await;
    }
    let (reconnecting, offline) = remembered_inventory(app).await;
    panic!("remembered machines were not projected: reconnecting {reconnecting:?}, offline {offline:?}");
}

async fn wait_for_remembered(app: &Arc<Mutex<DaemonApp>>) {
    wait_for_remembered_within(app, Duration::from_secs(10)).await;
}

/// Starts the connector for `config`, expects the remembered machines within `window`, stops it.
async fn remembered_machines_listed_within(config: DaemonConfig, window: Duration) {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("daemon should bootstrap"),
    ));
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app),
        Arc::clone(&state),
        shutdown_rx,
    ));

    wait_for_remembered_within(&app, window).await;

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
}

type RelayRegistry = Arc<RwLock<chariox_relay::server::RelayRegistry>>;

/// Runs a relay on a runtime of its own, so stopping it cuts every socket at once.
fn spawn_relay() -> (
    std::net::SocketAddr,
    RelayRegistry,
    std::sync::mpsc::Sender<()>,
    std::thread::JoinHandle<()>,
) {
    let (relay_up_tx, relay_up_rx) = std::sync::mpsc::channel();
    let (relay_stop_tx, relay_stop_rx) = std::sync::mpsc::channel::<()>();
    let relay_thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("relay runtime");
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = runtime
            .block_on(server.bind_listener())
            .expect("relay listener should bind");
        let addr = listener.local_addr().expect("listener should have addr");
        let server = Arc::new(RelayServer::new(RelayConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            shared_token: Some("secret".to_string()),
        }));
        relay_up_tx
            .send((addr, server.registry()))
            .expect("test should wait for the relay");
        let running = Arc::clone(&server);
        runtime.spawn(async move {
            running
                .run_listener_until(listener, std::future::pending::<()>())
                .await
                .expect("relay server should run");
        });
        let _ = relay_stop_rx.recv();
        runtime.shutdown_background();
    });
    let (addr, registry) = relay_up_rx.recv().expect("relay should start");
    (addr, registry, relay_stop_tx, relay_thread)
}

// A kernel that restarts while the relay is down still lists what it remembers.
#[tokio::test(flavor = "multi_thread")]
async fn a_kernel_restarted_with_the_relay_down_lists_remembered_machines() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    remember_two_machines();
    let closed_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("port should bind");
        listener.local_addr().expect("listener addr").port()
    };

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://127.0.0.1:{closed_port}"));
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("daemon should bootstrap"),
    ));
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app),
        Arc::clone(&state),
        shutdown_rx,
    ));

    wait_for_remembered(&app).await;

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
}

// Losing the relay keeps the remembered machines listed instead of emptying the lists.
#[tokio::test(flavor = "multi_thread")]
async fn losing_the_relay_keeps_remembered_machines_listed() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    remember_two_machines();
    let (addr, registry, relay_stop_tx, relay_thread) = spawn_relay();

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://{}:{}", addr.ip(), addr.port()));
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app),
        Arc::clone(&state),
        shutdown_rx,
    ));
    wait_for_daemon_registration(registry, &config.daemon_id).await;
    refresh_remote_inventory_projection_for_app_with_relay_state(&app)
        .await
        .expect("inventory should refresh while the relay is up");
    wait_for_remembered(&app).await;

    relay_stop_tx
        .send(())
        .expect("relay thread should be waiting");
    relay_thread.join().expect("relay thread should join");
    for _ in 0..100 {
        if !state.read().await.connected {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(!state.read().await.connected, "the relay is gone");
    sleep(Duration::from_millis(500)).await;
    wait_for_remembered(&app).await;

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
}

// A relay whose handshake never answers keeps the remembered machines listed from the start,
// not only after the connect timeout.
#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_relay_handshake_lists_remembered_machines() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    remember_two_machines();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("port should bind");
    let port = listener.local_addr().expect("listener addr").port();
    let stalled = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(format!("ws://127.0.0.1:{port}"));
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    remembered_machines_listed_within(config, RELAY_CONNECT_TIMEOUT / 2).await;

    stalled.abort();
}

// A Cloud token that has expired and cannot be renewed keeps the remembered machines listed.
#[tokio::test(flavor = "multi_thread")]
async fn an_unrenewable_cloud_token_lists_remembered_machines() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    remember_two_machines();
    let closed_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("port should bind");
        listener.local_addr().expect("listener addr").port()
    };

    let mut config = DaemonConfig::for_tests();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        api_url: format!("http://127.0.0.1:{closed_port}"),
        email: "user@example.test".to_string(),
        account_id: "account-1".to_string(),
        user_id: "user-1".to_string(),
        account_slug: "account".to_string(),
        realm_id: "realm-1".to_string(),
        relay_url: format!("ws://127.0.0.1:{closed_port}"),
        issuer_id: "issuer-1".to_string(),
        machine_credential: Some("machine-credential".to_string()),
        token_expires_at_ms: Some(1),
        ..crate::config::PersistedCloudRelayProfile::default()
    });
    config.relay_heartbeat_ms = 50;
    remembered_machines_listed_within(config, Duration::from_secs(4)).await;
}

// A kernel with no relay configured lists what it remembers.
#[tokio::test(flavor = "multi_thread")]
async fn a_kernel_without_a_relay_lists_remembered_machines() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    remember_two_machines();

    remembered_machines_listed_within(DaemonConfig::for_tests(), Duration::from_secs(4)).await;
}

/// What the healthy primary connector projected: one online machine with one kernel.
async fn seed_healthy_primary_inventory(app: &Arc<Mutex<DaemonApp>>) {
    let projection = app.lock().await.remote_relay_inventory_projection_store();
    projection.update(
        crate::local::RemoteMachineInventory {
            machines: vec![crate::local::RemoteMachineRecord {
                machine_id: "machine-live".to_string(),
                machine_alias: None,
                display_name: "machine-live name".to_string(),
                trust_status: crate::local::RemoteMachineTrustStatus::Approved,
                presence: RemoteMachinePresence::Online,
                pending: false,
                kernel_count: 1,
                available_providers: Vec::new(),
                provider_accounts: Vec::new(),
                kernels: Vec::new(),
            }],
            offline_machines: Vec::new(),
        },
        vec![chariox_relay::protocol::RelayKernelPresence {
            kernel_id: "kernel-live".to_string(),
            machine_id: "machine-live".to_string(),
            machine_alias: None,
            relay_alias: None,
            kernel_alias: None,
            available_providers: Vec::new(),
            provider_accounts: Vec::new(),
            capabilities: Vec::new(),
            accepting_remote_leases: true,
            leased_agent_count: 0,
            local_session_count: 0,
            public_key: "live-public-key".to_string(),
            local_protocol: None,
        }],
    );
}

async fn assert_primary_inventory_intact(app: &Arc<Mutex<DaemonApp>>) {
    let projection = app.lock().await.remote_relay_inventory_projection_store();
    let inventory = projection.inventory();
    let (_, kernels) = projection.snapshot();
    assert_eq!(
        inventory
            .machines
            .iter()
            .map(|machine| (machine.machine_id.as_str(), machine.presence))
            .collect::<Vec<_>>(),
        [("machine-live", RemoteMachinePresence::Online)],
        "a private relay must not replace the healthy primary inventory"
    );
    assert!(
        inventory.offline_machines.is_empty(),
        "an offline machine is never listed"
    );
    assert_eq!(
        kernels
            .iter()
            .map(|kernel| kernel.kernel_id.as_str())
            .collect::<Vec<_>>(),
        ["kernel-live"],
        "a private relay must not clear the primary's kernels"
    );
}

// A private relay that is unreachable starts and retries without touching the healthy primary
// inventory that its connector shares.
#[tokio::test(flavor = "multi_thread")]
async fn a_private_relay_that_cannot_connect_leaves_the_primary_inventory_alone() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    remember_two_machines();
    let closed_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("port should bind");
        listener.local_addr().expect("listener addr").port()
    };
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should bootstrap"),
    ));
    seed_healthy_primary_inventory(&app).await;
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app),
        16,
    ));
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector_with_router_and_static_relay(
        router,
        state,
        shutdown_rx,
        format!("ws://127.0.0.1:{closed_port}"),
        "secret".to_string(),
    ));

    // Long enough for the first pass and two retries.
    sleep(Duration::from_millis(2500)).await;
    assert_primary_inventory_intact(&app).await;

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
}

// A private relay whose connection ends does not hand the primary inventory to the remembered list.
#[tokio::test(flavor = "multi_thread")]
async fn a_private_relay_that_drops_leaves_the_primary_inventory_alone() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    remember_two_machines();
    let (addr, registry, relay_stop_tx, relay_thread) = spawn_relay();
    let config = DaemonConfig::for_tests();
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    seed_healthy_primary_inventory(&app).await;
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app),
        16,
    ));
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector_with_router_and_static_relay(
        router,
        Arc::clone(&state),
        shutdown_rx,
        format!("ws://{}:{}", addr.ip(), addr.port()),
        "secret".to_string(),
    ));
    wait_for_daemon_registration(registry, &config.daemon_id).await;
    assert_primary_inventory_intact(&app).await;

    relay_stop_tx
        .send(())
        .expect("relay thread should be waiting");
    relay_thread.join().expect("relay thread should join");
    for _ in 0..100 {
        if !state.read().await.connected {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(!state.read().await.connected, "the relay is gone");
    sleep(Duration::from_millis(500)).await;
    assert_primary_inventory_intact(&app).await;

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
}

// Presence state table, row "Remove during a disconnected pass": a Remove that lands after a pass
// read the registry and before it wrote the projection is not undone by that pass. The test stops
// the pass at its projection write and lets the Remove go as far as it can meanwhile.
#[tokio::test(flavor = "multi_thread")]
async fn a_remove_during_a_disconnected_pass_is_not_undone() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    remember_two_machines();
    let projection = crate::runtime::projection::RemoteRelayInventoryProjectionStore::default();
    let mut held = projection.hold();
    let pass = {
        let projection = projection.clone();
        std::thread::spawn(move || {
            super::super::remote_inventory::retain_remembered_machines("host-machine", &projection)
        })
    };
    std::thread::sleep(Duration::from_millis(200));
    let (removed_tx, removed_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ =
            removed_tx.send(DaemonConfig::forget_remote_machine("machine-old", |_| {}).map(|_| ()));
    });
    match removed_rx.recv_timeout(Duration::from_millis(300)) {
        Ok(removed) => {
            removed.expect("Remove should persist");
            held.remove_machine("machine-old");
            drop(held);
            pass.join()
                .expect("pass thread")
                .expect("pass should project");
        }
        Err(_) => {
            drop(held);
            pass.join()
                .expect("pass thread")
                .expect("pass should project");
            removed_rx
                .recv()
                .expect("Remove thread")
                .expect("Remove should persist");
            projection.remove_machine("machine-old");
        }
    }
    let offline: Vec<String> = projection
        .inventory()
        .offline_machines
        .into_iter()
        .map(|machine| machine.machine_id)
        .collect();
    assert!(
        offline.is_empty(),
        "a removed machine came back from a pass that started before the Remove: {offline:?}"
    );
}

/// A kernel of `machine_id` registered over a socket the test holds: it answers nothing, so a
/// peer probe of it waits until it times out.
async fn register_silent_kernel(
    relay_url: &str,
    daemon_id: &str,
    machine_id: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut config = DaemonConfig::for_tests();
    config.daemon_id = daemon_id.to_string();
    config.host_machine_id = machine_id.to_string();
    let (mut socket, handshake) = connect_async(relay_url)
        .await
        .expect("the kernel should connect to the relay");
    let register = crate::transport::relay_socket_connect::daemon_register(
        relay_url,
        &handshake,
        chariox_relay::protocol::DaemonRegistration {
            auth_token: "secret".to_string(),
            daemon_id: config.daemon_id.clone(),
            machine_id: config.host_machine_id.clone(),
            machine_alias: Some(format!("{machine_id} name")),
            os_name: Some("Linux".to_string()),
            kernel_started_at_ms: crate::session::unix_epoch_ms(),
            daemon_alias: None,
            kernel_alias: None,
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
    .expect("the registration should be proven");
    socket
        .send(Message::Text(
            serde_json::to_string(&register)
                .expect("registration should encode")
                .into(),
        ))
        .await
        .expect("the registration should send");
    socket
}

fn approve(machine_id: &str) {
    DaemonConfig::approve_remote_machine(machine_id).expect("approval should persist");
}

fn listed(
    app_projection: &crate::runtime::projection::RemoteRelayInventoryProjectionStore,
    machine_id: &str,
) -> Option<crate::local::RemoteMachineRecord> {
    app_projection
        .inventory()
        .machines
        .into_iter()
        .find(|machine| machine.machine_id == machine_id)
}

// Presence state table, row "disconnect after an idle while": with no client following the list,
// no pass writes a sighting, so the written one ages. A machine shown Online when the relay drops
// was seen gone only then: it is Reconnecting, never straight Offline.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_shown_online_is_reconnecting_after_a_disconnect_however_old_its_written_sighting(
) {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    approve("worker-machine");
    let (addr, registry, relay_stop_tx, relay_thread) = spawn_relay();
    let relay_url = format!("ws://{}:{}", addr.ip(), addr.port());
    let _worker = register_silent_kernel(&relay_url, "worker-kernel", "worker-machine").await;
    wait_for_daemon_registration(registry.clone(), "worker-kernel").await;

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(relay_url);
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    let projection = app.lock().await.remote_relay_inventory_projection_store();
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector(
        Arc::clone(&app),
        Arc::clone(&state),
        shutdown_rx,
    ));
    wait_for_daemon_registration(registry, &config.daemon_id).await;
    refresh_remote_inventory_projection_for_app_with_relay_state(&app)
        .await
        .expect("inventory should refresh while the relay is up");
    assert_eq!(
        listed(&projection, "worker-machine").map(|machine| machine.presence),
        Some(RemoteMachinePresence::Online)
    );
    // Eleven idle minutes: the last written sighting is that old, the machine still shown Online.
    let eleven_minutes_ago = crate::session::unix_epoch_ms() - 11 * MINUTE_MS;
    DaemonConfig::update_machine_registry(|registry| {
        for entry in registry.iter_mut() {
            entry.last_seen_ms = eleven_minutes_ago;
        }
    })
    .expect("the aged sighting should persist");

    relay_stop_tx
        .send(())
        .expect("relay thread should be waiting");
    relay_thread.join().expect("relay thread should join");
    for _ in 0..100 {
        if !state.read().await.connected {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    sleep(Duration::from_millis(300)).await;
    let inventory = projection.inventory();
    assert_eq!(
        listed(&projection, "worker-machine").map(|machine| machine.presence),
        Some(RemoteMachinePresence::Reconnecting),
        "the machine went straight to {:?}",
        inventory.offline_machines
    );

    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
}

// Presence state table, row "live refresh in flight + Remove": a live pass that read the registry
// before a Remove does not publish the approval the Remove took away. The pass is held at its
// probe of the machine's kernel, which never answers, while the owner removes the machine.
#[tokio::test(flavor = "multi_thread")]
async fn a_live_pass_overtaken_by_remove_does_not_publish_the_old_approval() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    approve("worker-machine");
    let (addr, registry, relay_stop_tx, relay_thread) = spawn_relay();
    let relay_url = format!("ws://{}:{}", addr.ip(), addr.port());
    let mut worker = register_silent_kernel(&relay_url, "worker-kernel", "worker-machine").await;
    wait_for_daemon_registration(registry, "worker-kernel").await;

    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(relay_url);
    config.relay_token = Some("secret".to_string());
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("daemon should bootstrap"),
    ));
    let projection = app.lock().await.remote_relay_inventory_projection_store();
    refresh_remote_inventory_projection_for_app_with_relay_state(&app)
        .await
        .expect("the first pass lists the machine");
    assert_eq!(
        listed(&projection, "worker-machine").map(|machine| machine.pending),
        Some(false),
        "approved before the Remove"
    );

    // The kernel is known now, so the next pass probes it and waits there.
    let pass = {
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            refresh_remote_inventory_projection_for_app_with_relay_state(&app).await
        })
    };
    timeout(Duration::from_secs(5), async {
        loop {
            let Some(Ok(Message::Text(text))) = worker.next().await else {
                panic!("the kernel's socket closed before the probe");
            };
            if text.contains("daemon_incoming_peer_request") {
                break;
            }
        }
    })
    .await
    .expect("the pass should probe the machine's kernel");

    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app),
        1,
    ));
    let request =
        LocalDaemonRequest::ForgetRemoteMachine(crate::local::ForgetRemoteMachineRequest {
            machine_ref: "worker-machine".into(),
        });
    let command = crate::runtime::command::KernelCommand::from_local_request_with_caller(
        "forget-machine",
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
    let removed = router
        .dispatch(command, request)
        .await
        .expect("Remove should answer");
    assert!(
        matches!(removed, LocalDaemonResponse::RemoteMachineForgotten { .. }),
        "{removed:?}"
    );

    pass.await
        .expect("pass task")
        .expect("the pass should end with the machine list");
    let machine = listed(&projection, "worker-machine")
        .expect("the machine is still connected, so it is listed again");
    assert!(
        machine.pending,
        "a pass that started before the Remove published the old approval: {machine:?}"
    );

    relay_stop_tx
        .send(())
        .expect("relay thread should be waiting");
    relay_thread.join().expect("relay thread should join");
}

// Presence state table, row "subscriber demand, local": a client following the Room list over the
// kernel's own socket keeps the machine list fresh, as a relay client does. A machine that
// connects later appears without anyone asking for the list.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_list_followed_only_by_a_local_client_stays_fresh() {
    let _relay_test_guard = relay_client_test_guard().await;
    let _home = RelayTestHome::new();
    let (addr, registry, relay_stop_tx, relay_thread) = spawn_relay();
    let relay_url = format!("ws://{}:{}", addr.ip(), addr.port());
    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some(relay_url.clone());
    config.relay_token = Some("secret".to_string());
    config.relay_heartbeat_ms = 50;
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon should bootstrap"),
    ));
    let projection = app.lock().await.remote_relay_inventory_projection_store();
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app),
        4,
    ));

    let (client_io, server_io) = tokio::io::duplex(1 << 20);
    let local_server = {
        let router = Arc::clone(&router);
        tokio::spawn(async move {
            let socket = tokio_tungstenite::accept_async(server_io)
                .await
                .expect("the local socket should open");
            crate::runtime_transport::serve_admitted_test_socket(router, socket).await
        })
    };
    let (mut local_client, _) = tokio_tungstenite::client_async("ws://localhost/", client_io)
        .await
        .expect("the local client should connect");
    local_client
        .send(Message::Text(
            serde_json::json!({
                "type": "subscribe", "request_id": "room-list", "session_id": "__room_list__",
                "attachment_id": "__room_list__", "subscription_scope": "room_list",
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("the Room list subscription should send");
    let reader = tokio::spawn(async move { while local_client.next().await.is_some() {} });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let connector_task = tokio::spawn(run_daemon_relay_connector_with_router(
        Arc::clone(&router),
        Arc::new(RwLock::new(RelayClientState::default())),
        shutdown_rx,
    ));
    wait_for_daemon_registration(registry.clone(), &config.daemon_id).await;
    sleep(Duration::from_millis(500)).await;
    let _late = register_silent_kernel(&relay_url, "late-kernel", "late-machine").await;
    wait_for_daemon_registration(registry, "late-kernel").await;

    let mut seen = false;
    for _ in 0..300 {
        if listed(&projection, "late-machine").is_some() {
            seen = true;
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        seen,
        "a machine that connected later never appeared for the local client"
    );

    reader.abort();
    local_server.abort();
    let _ = shutdown_tx.send(true);
    connector_task.await.expect("connector task should join");
    relay_stop_tx
        .send(())
        .expect("relay thread should be waiting");
    relay_thread.join().expect("relay thread should join");
}
