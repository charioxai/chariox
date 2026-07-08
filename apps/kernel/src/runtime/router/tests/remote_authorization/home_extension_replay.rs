use super::*;

#[tokio::test]
async fn home_extension_invocation_replay_is_audited() {
    let workspace = home_extension_script_workspace("replay");
    let (router, context, mut metadata, hinted_tool, agent_id) =
        home_extension_script_router(&workspace, "remote-extension-replay");
    metadata.idempotency_key = Some("idem-replay".to_string());
    let completed = router
        .runtime_state
        .dispatch_forwarded_home_extension_tool_call(
            context.clone(),
            metadata.clone(),
            hinted_tool.clone(),
            serde_json::json!({}),
        )
        .await
        .expect("initial idempotent invocation should complete");
    assert!(completed.ok);

    let replayed = router
        .runtime_state
        .dispatch_forwarded_home_extension_tool_call(
            context,
            metadata,
            hinted_tool,
            serde_json::json!({}),
        )
        .await
        .expect("idempotent replay should return cached result");
    assert_eq!(replayed.ok, completed.ok);
    assert_eq!(replayed.payload, completed.payload);

    let events = router
        .runtime_state
        .list_home_extension_audit_events(&agent_id, DEFAULT_LOCAL_USER_ID, 10)
        .expect("audit events should load");
    let event = events
        .iter()
        .find(|event| event.kind == "home_extension.invoke.replayed")
        .expect("idempotent replay should be audited");
    assert_eq!(
        event
            .payload
            .pointer("/status")
            .and_then(serde_json::Value::as_str),
        Some("replayed")
    );
    assert_eq!(
        event
            .payload
            .pointer("/invocation/idempotency_key")
            .and_then(serde_json::Value::as_str),
        Some("idem-replay")
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn home_extension_invocation_duplicate_rejection_is_audited() {
    let workspace = home_extension_script_workspace("duplicate");
    let (router, context, metadata, hinted_tool, agent_id) =
        home_extension_script_router(&workspace, "remote-extension-duplicate");
    let completed = router
        .runtime_state
        .dispatch_forwarded_home_extension_tool_call(
            context.clone(),
            metadata.clone(),
            hinted_tool.clone(),
            serde_json::json!({}),
        )
        .await
        .expect("initial non-idempotent invocation should complete");
    assert!(completed.ok);

    let denied = router
        .runtime_state
        .dispatch_forwarded_home_extension_tool_call(
            context,
            metadata,
            hinted_tool,
            serde_json::json!({}),
        )
        .await
        .expect_err("duplicate non-idempotent invocation should be denied");
    assert!(
        denied
            .to_string()
            .contains("duplicate non-idempotent home extension invocation"),
        "unexpected denial: {denied}"
    );

    let events = router
        .runtime_state
        .list_home_extension_audit_events(&agent_id, DEFAULT_LOCAL_USER_ID, 10)
        .expect("audit events should load");
    let event = events
        .iter()
        .find(|event| event.kind == "home_extension.invoke.denied")
        .expect("duplicate rejection should be audited");
    assert_eq!(
        event
            .payload
            .pointer("/status")
            .and_then(serde_json::Value::as_str),
        Some("denied")
    );
    assert!(event
        .payload
        .pointer("/error")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|error| error.contains("duplicate non-idempotent")));

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test(flavor = "multi_thread")]
async fn forwarded_home_script_invocation_uses_shared_timeout_policy() {
    let workspace = std::env::temp_dir().join(format!(
        "arroba-home-extension-script-timeout-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos()
    ));
    let script_dir = workspace.join(".arroba").join("scripts").join("home-slow");
    std::fs::create_dir_all(&script_dir).expect("script dir should be created");
    std::fs::write(
        script_dir.join("metadata.json"),
        r#"{
  "name": "home-slow",
  "runtime": "python",
  "entrypoint": "script.py",
  "description": "Slow home-owned test script",
  "input_schema": {"type": "object", "properties": {}},
  "definition_hash": "slow-test-hash",
  "timeout_sec": 1
}
"#,
    )
    .expect("script metadata should be written");
    std::fs::write(
        script_dir.join("script.py"),
        "import time\n\ndef run():\n    time.sleep(5)\n    return {\"done\": True}\n",
    )
    .expect("script should be written");
    let env_dir = workspace.join(".arroba").join("envs");
    std::fs::create_dir_all(&env_dir).expect("env dir should be created");
    std::fs::write(
        env_dir.join("test-env.json"),
        r#"{
  "name": "test-env",
  "runtime": {"type": "python", "python": "/usr/bin/python3"}
}
"#,
    )
    .expect("environment should be written");

    let config = DaemonConfig::for_tests();
    let home_kernel_id = config.daemon_id.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = spawn_test_agent(
        &mut app,
        &session_id,
        "remote-extension-timeout",
        "dev-stub",
    );
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("provider-run-1".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("agent should be remote-backed");
    let granted_agent = app
        .agents()
        .grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::script("home-slow", "test-env"),
        )
        .expect("script grant should be recorded");
    let manifest = app
        .remote_extension_manifest_for_agent(&granted_agent)
        .expect("home manifest should be rebuilt from current state");
    let hinted_tool = manifest
        .home_proxy_tool("home-slow")
        .expect("home script should be projected")
        .clone();
    assert_eq!(hinted_tool.timeout_sec, Some(1));

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let metadata = crate::extension::RemoteExtensionInvocationMetadata::new(
        "provider-run-1",
        "home-slow",
        None,
    );
    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id,
        home_session_id: session_id,
        home_agent_id: agent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-1".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };

    let started = std::time::Instant::now();
    let result = router
        .runtime_state
        .dispatch_forwarded_home_extension_tool_call(
            context,
            metadata,
            hinted_tool,
            serde_json::json!({}),
        )
        .await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "forwarded script call should not wait for the default script timeout"
    );
    match result {
        Ok(result) => {
            assert!(!result.ok, "timed out script should not be reported as ok");
            assert_eq!(
                result
                    .payload
                    .pointer("/error/kind")
                    .and_then(serde_json::Value::as_str),
                Some("timeout")
            );
        }
        Err(error) => assert!(
            error.to_string().contains("timed out after 1s"),
            "unexpected timeout error: {error}"
        ),
    }

    let _ = std::fs::remove_dir_all(&workspace);
}
