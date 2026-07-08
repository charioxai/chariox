use std::time::Duration;

use crate::support::kernel_websocket::*;
use arroba_kernel::local::{
    LaunchProviderRunRequest, ListProviderProcessesRequest, LocalDaemonRequest,
};
use arroba_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use arroba_kernel::session::CreateSessionRequest;
use arroba_kernel::{DaemonApp, DaemonConfig};
use tokio::sync::oneshot;
use tokio::time::sleep;

#[test]
fn kernel_websocket_provider_process_gc_reaps_idle_managed_process() {
    crate::run_kernel_websocket_runtime_test(async {
        let mut config = DaemonConfig::for_tests();
        let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
        config.kernel_websocket_port = kernel_websocket_port;
        config.runtime_mcp_port = unused_tcp_port();
        config.provider_process_idle_ttl_ms = 1_000;
        config.provider_process_orphan_ttl_ms = u64::MAX;
        let app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            run_kernel_websocket_server_on_listener(
                std::sync::Arc::new(tokio::sync::Mutex::new(app)),
                kernel_websocket_listener,
                async {
                    let _ = shutdown_rx.await;
                },
            )
            .await
        });

        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;

        let create_response = send_request(
            &mut socket,
            "gc-create-session",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-provider-gc",
                "worktree-provider-gc",
            )),
        )
        .await;
        let session = &response_variant(&create_response, "SessionCreated")["session"];
        let session_id = session["id"]
            .as_str()
            .expect("session id should be present")
            .to_string();
        let agent_id = session["agents"][0]["id"]
            .as_str()
            .expect("agent id should be present")
            .to_string();

        let provider_response = send_request(
            &mut socket,
            "gc-launch-provider",
            LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            }),
        )
        .await;
        let provider_run_id = provider_run_id_from_launch_response(&provider_response);
        wait_for_provider_run_state(&mut socket, &provider_run_id, "Running").await;

        let listed_response = send_request(
            &mut socket,
            "gc-list-provider-processes-before",
            LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest {
                provider: None,
            }),
        )
        .await;
        let processes = response_variant(&listed_response, "ProviderProcessesListed")["processes"]
            .as_array()
            .expect("provider processes should be present");
        assert_eq!(processes.len(), 1);
        let pid = processes[0]["pid"]
            .as_u64()
            .expect("managed provider process pid should be present") as u32;

        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let mut list_attempt = 0_u32;
        loop {
            list_attempt += 1;
            let listed_response = send_request(
                &mut socket,
                &format!("gc-list-provider-processes-after-{list_attempt}"),
                LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest {
                    provider: None,
                }),
            )
            .await;
            let processes =
                response_variant(&listed_response, "ProviderProcessesListed")["processes"]
                    .as_array()
                    .expect("provider processes should be present");
            if processes.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "provider process gc did not reap idle process: {processes:?}"
            );
            sleep(Duration::from_millis(200)).await;
        }
        assert!(!arroba_kernel::runtime::process_health::process_running(
            pid
        ));

        drop(socket);
        shutdown_tx.send(()).expect("shutdown should send");
        server
            .await
            .expect("kernel websocket task should join")
            .expect("kernel websocket server should shut down cleanly");
    });
}
