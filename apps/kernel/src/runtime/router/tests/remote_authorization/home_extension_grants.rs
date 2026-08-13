use super::*;

#[test]
fn home_owner_controls_extension_grants_for_collaborator_remote_agent() {
    run_remote_authorization_large_stack_test(
        "home-owner-controls-extension-grants",
        home_owner_controls_extension_grants_for_collaborator_remote_agent_inner,
    );
}

async fn home_owner_controls_extension_grants_for_collaborator_remote_agent_inner() {
    let workspace = std::env::temp_dir().join(format!(
        "chariox-home-extension-authority-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos()
    ));
    let script_dir = workspace.join(".chariox").join("scripts").join("home-only");
    std::fs::create_dir_all(&script_dir).expect("script dir should be created");
    std::fs::write(
        script_dir.join("metadata.json"),
        r#"{
  "name": "home-only",
  "runtime": "python",
  "entrypoint": "script.py",
  "description": "Home-owned test script",
  "input_schema": {"type": "object", "properties": {}},
  "definition_hash": "test-hash"
}
"#,
    )
    .expect("script metadata should be written");
    std::fs::write(script_dir.join("script.py"), "def run():\n    return {}\n")
        .expect("script should be written");
    let env_dir = workspace.join(".chariox").join("envs");
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

    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_id,
            "invite-extension-peer".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Full,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("user should join session");
    let peer_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("peer-remote")
                .with_owner_user_id("user-2"),
        )
        .expect("peer agent should be created");
    let provider_run = launch_test_provider(
        &mut app,
        &session_id,
        peer_agent.id(),
        "dev-stub",
        "dev-stub",
        "authority-model",
    );
    let runtime_auth_token = provider_run
        .runtime_mcp_auth_token()
        .expect("provider run should expose runtime MCP auth token")
        .to_string();
    app.agents()
        .bind_remote_execution(
            peer_agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("agent should be remote-backed");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let runtime_request = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &runtime_auth_token,
            crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
            serde_json::json!({
                "kind": "script",
                "name": "home-only",
                "environment": "test-env",
            }),
        )
        .await
        .expect("runtime request_extension should return a policy result");
    assert!(!runtime_request.ok);
    assert_eq!(
        runtime_request
            .payload
            .get("authority")
            .and_then(serde_json::Value::as_str),
        Some("home")
    );

    let grant = LocalDaemonRequest::GrantAgentExtension(crate::local::GrantAgentExtensionRequest {
        workspace_id: Some(workspace.to_string_lossy().to_string()),
        agent_ref: peer_agent.id().to_string(),
        kind: crate::local::ExtensionKind::Script,
        name: "home-only".to_string(),
        environment: Some("test-env".to_string()),
        credential: Some("home-secret-token".to_string()),
        max_safety: None,
    });

    assert_ownership_denied(
        router
            .dispatch(
                remote_command_for_request(&grant, Some("user-2")),
                grant.clone(),
            )
            .await
            .expect_err("peer owner should not grant home-owned remote extension"),
        "user-2",
        DEFAULT_LOCAL_USER_ID,
    );
    match router
        .dispatch(
            remote_command_for_request(&grant, Some(DEFAULT_LOCAL_USER_ID)),
            grant,
        )
        .await
        .expect("home owner should grant home-owned remote extension")
    {
        LocalDaemonResponse::AgentExtensionGranted { agent } => {
            assert!(agent.has_extension_grant(crate::extension::ExtensionKind::Script, "home-only"));
        }
        other => panic!("unexpected grant response: {other:?}"),
    }
    let grant_events = router
        .runtime_state
        .list_home_extension_audit_events(peer_agent.id(), DEFAULT_LOCAL_USER_ID, 20)
        .expect("grant audit events should load");
    let grant_event = grant_events
        .iter()
        .find(|event| {
            event.kind == "home_extension.grant.created"
                && event
                    .payload
                    .pointer("/grant/kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("script")
        })
        .expect("home extension grant should be audited");
    assert_eq!(
        grant_event
            .payload
            .pointer("/home_user_id")
            .and_then(serde_json::Value::as_str),
        Some(DEFAULT_LOCAL_USER_ID)
    );
    assert_eq!(
        grant_event
            .payload
            .pointer("/caller_user_id")
            .and_then(serde_json::Value::as_str),
        Some(DEFAULT_LOCAL_USER_ID)
    );
    assert_eq!(
        grant_event
            .payload
            .pointer("/agent_owner_user_id")
            .and_then(serde_json::Value::as_str),
        Some("user-2")
    );
    assert_eq!(
        grant_event
            .payload
            .pointer("/lease_id")
            .and_then(serde_json::Value::as_str),
        Some("lease-1")
    );
    assert_eq!(
        grant_event
            .payload
            .pointer("/grant/credential_present")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert!(
        !grant_event
            .payload
            .to_string()
            .contains("home-secret-token"),
        "audit event must not serialize credential material"
    );

    let revoke =
        LocalDaemonRequest::RevokeAgentExtension(crate::local::RevokeAgentExtensionRequest {
            agent_ref: peer_agent.id().to_string(),
            kind: crate::local::ExtensionKind::Script,
            name: "home-only".to_string(),
        });
    assert_ownership_denied(
        router
            .dispatch(
                remote_command_for_request(&revoke, Some("user-2")),
                revoke.clone(),
            )
            .await
            .expect_err("peer owner should not revoke home-owned remote extension"),
        "user-2",
        DEFAULT_LOCAL_USER_ID,
    );
    match router
        .dispatch(
            remote_command_for_request(&revoke, Some(DEFAULT_LOCAL_USER_ID)),
            revoke,
        )
        .await
        .expect("home owner should revoke home-owned remote extension")
    {
        LocalDaemonResponse::AgentExtensionRevoked { agent } => {
            assert!(
                !agent.has_extension_grant(crate::extension::ExtensionKind::Script, "home-only")
            );
        }
        other => panic!("unexpected revoke response: {other:?}"),
    }
    let revoke_events = router
        .runtime_state
        .list_home_extension_audit_events(peer_agent.id(), DEFAULT_LOCAL_USER_ID, 20)
        .expect("revoke audit events should load");
    let revoke_event = revoke_events
        .iter()
        .find(|event| {
            event.kind == "home_extension.grant.revoked"
                && event
                    .payload
                    .pointer("/grant/kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("script")
        })
        .expect("home extension revoke should be audited");
    assert_eq!(
        revoke_event
            .payload
            .pointer("/caller_user_id")
            .and_then(serde_json::Value::as_str),
        Some(DEFAULT_LOCAL_USER_ID)
    );
    assert_eq!(
        revoke_event
            .payload
            .pointer("/agent_owner_user_id")
            .and_then(serde_json::Value::as_str),
        Some("user-2")
    );

    let _ = std::fs::remove_dir_all(&workspace);
}
