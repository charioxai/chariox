//! A maintenance pass that waits on a slow service must not make the pump spin on
//! the exit of an access-grant holder it has already seen.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dead_holder_does_not_spin_the_pump_while_maintenance_waits() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mcp_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(daemon_config_for_runtime_mcp_listener(&mcp_listener)).unwrap(),
    ));
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app),
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    let state = router.runtime_state().clone();
    // The first maintenance pass sweeps access grants, then waits for the app lock.
    let slow_service = app.lock().await;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(run_kernel_websocket_server_with_bound_listeners(
        router,
        listener,
        adopt_std_listener(mcp_listener, "test MCP").unwrap(),
        KernelLocalAuth::Unconfigured,
        async {
            let _ = shutdown_rx.await;
        },
    ));
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut holder = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    let identity = crate::runtime::kernel_access::process::inspect(holder.id())
        .unwrap()
        .0;
    holder.kill().unwrap();
    holder.wait().unwrap();
    let grant = state.insert_access_grant_held_by_for_test(identity);

    let before = state.transport_pump_iterations_for_test();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let spun = state.transport_pump_iterations_for_test() - before;
    assert!(
        spun < 20,
        "the pump looped {spun} times in 500 ms on one dead holder while maintenance waited"
    );

    drop(slow_service);
    timeout(Duration::from_secs(5), async {
        while state.access_grant_ids_for_test().contains(&grant) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the next maintenance pass sweeps the dead holder's grant");
    let _ = shutdown_tx.send(());
    let _ = server.await;
}
