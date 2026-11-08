//! `agent spawn --room --machine <m>`: a machine of the owner names the kernel it runs, through
//! the relay's own discovery.
use super::support::*;

#[test]
fn a_machine_name_resolves_to_the_kernel_it_runs_and_an_unknown_name_is_unreachable() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack("peer-machine-name", || async {
        let _relay_test_guard = relay_client_test_guard().await;
        let _test_home = RelayTestHome::new();
        let server = RelayServer::new(RelayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            shared_token: Some("secret".to_string()),
        });
        let listener = server.bind_listener().await.expect("relay listener binds");
        let addr = listener.local_addr().expect("listener has an address");
        let server = Arc::new(server);
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
                    .expect("relay server runs");
            })
        };
        let relay_url = format!("ws://{}:{}", addr.ip(), addr.port());
        let mut connectors = Vec::new();
        let mut kernel = |id: &str, machine_id: &str, machine: Option<&str>| {
            let mut config = DaemonConfig::for_tests();
            config.daemon_id = id.to_string();
            config.daemon_alias = Some(id.trim_start_matches("daemon-").to_string());
            config.host_machine_id = machine_id.to_string();
            config.host_machine_alias = machine.map(str::to_string);
            config.relay_url = Some(relay_url.clone());
            config.relay_token = Some("secret".to_string());
            config.relay_heartbeat_ms = 50;
            let app = Arc::new(Mutex::new(
                DaemonApp::bootstrap(config).expect("kernel bootstraps"),
            ));
            let state = app.try_lock().unwrap().relay_client_state();
            let (shutdown_tx, shutdown_rx) = watch::channel(false);
            let connector = tokio::spawn(run_daemon_relay_connector(
                Arc::clone(&app),
                state,
                shutdown_rx,
            ));
            connectors.push((shutdown_tx, connector));
            app
        };
        let home = kernel("daemon-home", "machine-home", None);
        kernel("daemon-worker", "machine-worker", Some("builder-west"));
        kernel("daemon-twin-a", "machine-twins", Some("builder-twins"));
        kernel("daemon-twin-b", "machine-twins", Some("builder-twins"));
        for id in [
            "daemon-home",
            "daemon-worker",
            "daemon-twin-a",
            "daemon-twin-b",
        ] {
            wait_for_daemon_registration(registry.clone(), id).await;
        }
        let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(home, 4)
            .runtime_state();

        let resolved = runtime
            .resolve_peer_kernel("builder-west")
            .await
            .expect("a machine of the owner resolves");
        assert_eq!(resolved.as_deref(), Some("daemon-worker"));

        for (name, reason) in [
            ("no-such-machine", "kernel_unreachable"),
            ("builder-twins", "kernel_ambiguous"),
        ] {
            let refusal = runtime
                .resolve_peer_kernel(name)
                .await
                .expect_err("not one kernel")
                .to_string();
            assert!(refusal.contains(reason), "{name}: {refusal}");
            for transport in ["relay", "metadata", "not found", "get_live_kernel"] {
                assert!(
                    !refusal.contains(transport),
                    "{name}: the reason, not the transport's words: {refusal}"
                );
            }
        }

        for (shutdown, connector) in connectors {
            let _ = shutdown.send(true);
            connector.await.expect("connector joins");
        }
        let _ = server_shutdown_tx.send(());
        server_task.await.expect("relay server joins");
    });
}
