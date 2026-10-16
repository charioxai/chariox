use super::*;

#[test]
fn leased_mcp_reregistration_refreshes_already_granted_definition() {
    run_test(check_leased_mcp_reregistration);
}

async fn check_leased_mcp_reregistration() {
    check_capability_response_refresh().await;
}

async fn check_capability_response_refresh() {
    let mut fixture = LiveWorker::start().await;
    let (_worker_state, worker) = start_agent_worker(&mut fixture).await;
    let capability_root = fixture
        .home_state
        .root
        .join("response-refresh-capabilities");
    let _capability_environment = HomePersistenceEnvironment::set(
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        capability_root.as_os_str(),
    );
    let check = std::panic::AssertUnwindSafe(async {
        wait_for_agent_worker(&fixture).await;
        let name = "leased-response-refresh-fixture";
        let mut config = crate::mcp::CharioxMcpServerConfig::stdio(
            name,
            "/bin/echo",
            vec!["original".to_string()],
        );
        config.tool_timeout_sec = Some(13);
        crate::mcp::CharioxMcpRegistry::new(vec![
            crate::mcp::CharioxMcpRegistry::user_root().expect("isolated MCP registry")
        ])
        .install(&config)
        .expect("install MCP fixture");
        let leased = launch_leased_room_provider(
            &fixture,
            &worker,
            "exercise forwarded capability response refresh",
        )
        .await;
        let token = worker
            .runtime_state
            .runtime_mcp_auth_token_for_provider_run(&leased.worker_provider_run_id)
            .unwrap();
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
        let started = std::time::Instant::now();
        let granted = worker
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &token,
                crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
                json!({"kind": "mcp", "name": name}),
            )
            .await
            .expect("forwarded grant completes");
        // Await completion before asserting so a failed regression leaves no
        // detached request holding locks during fixture teardown.
        assert!(
            started.elapsed() < completion_budget,
            "grant waited {:?}, relay timeouts {home_timeout}/{worker_timeout} ms",
            started.elapsed()
        );
        assert!(granted.ok, "{:?}", granted.payload);
        assert_eq!(granted.payload["granted"], true);
        {
            let before = worker
                .app
                .lock()
                .await
                .providers()
                .get_run(&leased.worker_provider_run_id)
                .unwrap();
            let before_tool = before
                .remote_extension_manifest()
                .tools
                .iter()
                .find(|tool| tool.name == name)
                .unwrap();
            assert_eq!(
                before_tool.version_hash.as_deref(),
                Some(config.definition_hash().unwrap().as_str())
            );
            config.tool_timeout_sec = Some(29);
            let expected_hash = config.definition_hash().unwrap();
            let registered = worker
                .runtime_state
                .dispatch_authenticated_runtime_tool_call(
                    &token,
                    crate::transport::runtime_tools::REGISTER_MCP_TOOL,
                    json!({"config": config}),
                )
                .await
                .expect("re-register existing grant without requesting another grant");
            assert!(registered.ok, "{:?}", registered.payload);
            let after = worker
                .app
                .lock()
                .await
                .providers()
                .get_run(&leased.worker_provider_run_id)
                .unwrap();
            let after_tool = after
                .remote_extension_manifest()
                .tools
                .iter()
                .find(|tool| tool.name == name)
                .unwrap();
            assert_eq!(
                after_tool.version_hash.as_deref(),
                Some(expected_hash.as_str())
            );
            assert_eq!(after_tool.timeout_sec, Some(29));
            assert_ne!(before_tool.version_hash, after_tool.version_hash);
            let events = fixture
                .home
                .runtime_state
                .list_home_extension_audit_events(&leased.home_agent_id, DEFAULT_LOCAL_USER_ID, 100)
                .expect("registration audit");
            // No grant push occurs here. The response manifest itself carries
            // the updated definition, without a compare-and-set refresh.
            let registrations = events
                .iter()
                .filter(|event| {
                    event.kind == "extension.registration.created"
                        && event.payload["kind"] == "mcp"
                        && event.payload["name"] == name
                })
                .count();
            assert_eq!(registrations, 1, "registration executes exactly once");
        }
        let events = fixture
            .home
            .runtime_state
            .list_home_extension_audit_events(&leased.home_agent_id, DEFAULT_LOCAL_USER_ID, 100)
            .expect("grant audit");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "home_extension.grant.created"
                    && event.payload["grant"]["kind"] == "mcp"
                    && event.payload["grant"]["name"] == name)
                .count(),
            1,
            "refresh must not replay the original grant"
        );
        let app = worker.app.lock().await;
        assert!(
            app.providers()
                .get_run(&leased.worker_provider_run_id)
                .is_ok(),
            "forwarding preserves the same provider run"
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
    cleanup.expect("stop provider after capability response refresh regression");
    if let Err(panic) = check {
        std::panic::resume_unwind(panic);
    }
}

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
