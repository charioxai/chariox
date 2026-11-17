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
