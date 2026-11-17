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

async fn wait_for_remembered(app: &Arc<Mutex<DaemonApp>>) {
    for _ in 0..200 {
        let (reconnecting, offline) = remembered_inventory(app).await;
        if reconnecting == ["machine-recent"] && offline == ["machine-old"] {
            return;
        }
        sleep(Duration::from_millis(50)).await;
    }
    let (reconnecting, offline) = remembered_inventory(app).await;
    panic!("remembered machines were not projected: reconnecting {reconnecting:?}, offline {offline:?}");
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
    // The relay runs on a runtime of its own, so dropping that runtime cuts every socket at once.
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
