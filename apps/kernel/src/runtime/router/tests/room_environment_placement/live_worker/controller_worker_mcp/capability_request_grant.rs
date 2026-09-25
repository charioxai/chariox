use super::*;

#[test]
fn leased_request_extension_completes_without_waiting_for_relay_timeout() {
    run_test(check_leased_request_extension_completes_without_waiting_for_relay_timeout);
}

async fn check_leased_request_extension_completes_without_waiting_for_relay_timeout() {
    let mut fixture = LiveWorker::start().await;
    let (_worker_state, worker) = start_agent_worker(&mut fixture).await;
    let capability_root = fixture
        .home_state
        .root
        .join("request-extension-capabilities");
    let _capability_environment = HomePersistenceEnvironment::set(
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        capability_root.as_os_str(),
    );
    let check =
        std::panic::AssertUnwindSafe(async {
            wait_for_agent_worker(&fixture).await;
            let name = "leased-request-extension-fixture";
            let registry = crate::mcp::CharioxMcpRegistry::new(vec![
                crate::mcp::CharioxMcpRegistry::user_root().expect("isolated MCP registry"),
            ]);
            registry
                .install(&crate::mcp::CharioxMcpServerConfig::stdio(
                    name,
                    "/bin/echo",
                    vec!["unused-fixture".to_string()],
                ))
                .expect("install fixture MCP metadata in disposable capability root");
            let leased = launch_leased_room_provider(
                &fixture,
                &worker,
                "request a home extension during this leased turn",
            )
            .await;
            let token = worker
                .runtime_state
                .runtime_mcp_auth_token_for_provider_run(&leased.worker_provider_run_id)
                .expect("leased provider runtime tool token");
            let worker_timeout = worker.app.lock().await.config().relay_request_timeout_ms;
            let home_timeout = fixture
                .home
                .app
                .lock()
                .await
                .config()
                .relay_request_timeout_ms;
            let completion_budget = Duration::from_secs(5);
            assert!(worker_timeout > completion_budget.as_millis() as u64);
            assert!(home_timeout > completion_budget.as_millis() as u64);
            // Keep real relay timeouts unchanged. Await the entire operation even
            // on RED so no detached capability task retains the worker app lock
            // during teardown. The elapsed assertion rejects timeout-based recovery.
            let started = std::time::Instant::now();
            let result = worker
                .runtime_state
                .dispatch_authenticated_runtime_tool_call(
                    &token,
                    crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
                    json!({"kind": "mcp", "name": name}),
                )
                .await
                .expect("forwarded request_extension must finish successfully");
            assert!(started.elapsed() < completion_budget,
            "grant waited {:?}; home/worker relay timeouts are {home_timeout}/{worker_timeout} ms",
            started.elapsed());
            assert!(result.ok, "{:?}", result.payload);
            assert_eq!(result.payload["granted"], true);
            let app = fixture.home.app.lock().await;
            let agent = app.agents().get_agent(&leased.home_agent_id).unwrap();
            assert!(agent.mcp_grants().iter().any(|grant| grant == name));
            drop(app);
            let app = worker.app.lock().await;
            let run = app
                .providers()
                .get_run(&leased.worker_provider_run_id)
                .expect("grant preserves the active worker provider run");
            assert!(
                run.remote_extension_manifest().tools.iter().any(|tool| {
                    tool.kind == crate::extension::ExtensionKind::Mcp && tool.name == name
                }),
                "successful grant must reach the current worker manifest"
            );
        })
        .catch_unwind()
        .await;
    let cleanup = worker
        .app
        .lock()
        .await
        .teardown_provider_processes(Some("managed-dev-stub"), true);
    fixture.stop().await;
    cleanup.expect("stop provider after forwarded extension grant regression");
    if let Err(panic) = check {
        std::panic::resume_unwind(panic);
    }
}
