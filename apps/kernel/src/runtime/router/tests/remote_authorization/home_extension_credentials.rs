use super::*;

#[tokio::test]
async fn forwarded_home_credential_secret_rejects_stale_worker_provider_run() {
    let config = DaemonConfig::for_tests();
    let home_kernel_id = config.daemon_id.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-home-credential-stale",
            "worktree-home-credential-stale",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = spawn_test_agent(&mut app, &session_id, "remote-credential-stale", "dev-stub");
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("provider-run-current".to_string()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
                worker_worktree_id: None,
            },
        )
        .expect("agent should be remote-backed");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4)
        .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
            "worker-kernel",
        ));
    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id,
        home_session_id: session_id,
        home_agent_id: agent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-stale".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };

    let denied = router
        .runtime_state
        .resolve_forwarded_home_credential_secret(
            context,
            "gmail-password".to_string(),
            crate::transport::relay_peer::RemoteCredentialSecretInjection::Browser {
                target_url: "https://accounts.google.com/signin".to_string(),
            },
        )
        .await
        .expect_err("stale worker provider run should be denied");
    assert!(
        denied
            .to_string()
            .contains("worker provider run does not match active remote agent binding"),
        "unexpected denial: {denied}"
    );
}

#[tokio::test]
async fn forwarded_home_credential_secret_adopts_first_worker_provider_run_for_active_prompt() {
    let (router, context, _agent_id) =
        remote_home_invocation_router_with_active_prompt("remote-credential-first-provider-run");
    let mut stale_context = context.clone();

    let error = router
        .runtime_state
        .resolve_forwarded_home_credential_secret(
            context,
            "not-registered".to_string(),
            crate::transport::relay_peer::RemoteCredentialSecretInjection::Browser {
                target_url: "https://accounts.google.com/signin".to_string(),
            },
        )
        .await
        .expect_err("test credential lookup should fail after authorization succeeds");
    assert!(
        error
            .to_string()
            .contains("unknown credential `not-registered`"),
        "authorization should pass and fail later at credential lookup: {error}"
    );
    stale_context.worker_provider_run_id = "provider-run-stale".to_string();
    let stale = router
        .runtime_state
        .resolve_forwarded_home_credential_secret(
            stale_context,
            "not-registered".to_string(),
            crate::transport::relay_peer::RemoteCredentialSecretInjection::Browser {
                target_url: "https://accounts.google.com/signin".to_string(),
            },
        )
        .await
        .expect_err("second provider run should be rejected after adoption");
    assert!(
        stale
            .to_string()
            .contains("worker provider run does not match active remote agent binding"),
        "unexpected stale denial: {stale}"
    );
}

#[tokio::test]
async fn home_extension_invocation_cancellation_is_authorized_and_audited() {
    let config = DaemonConfig::for_tests();
    let home_kernel_id = config.daemon_id.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-home-extension-cancel",
            "worktree-home-extension-cancel",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = spawn_test_agent(&mut app, &session_id, "remote-extension-cancel", "dev-stub");
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
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
                worker_worktree_id: None,
            },
        )
        .expect("agent should be remote-backed");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4)
        .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
            "worker-kernel",
        ));
    let metadata = crate::extension::RemoteExtensionInvocationMetadata::new(
        "provider-run-1",
        "home-only",
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

    let cancelled = router
        .runtime_state
        .cancel_forwarded_home_extension_invocation(context, metadata)
        .await
        .expect("valid cancellation context should be accepted");
    assert!(!cancelled, "test cancellation was not in flight");

    let events = router
        .runtime_state
        .list_home_extension_audit_events(agent.id(), DEFAULT_LOCAL_USER_ID, 10)
        .expect("audit events should load");
    let event = events
        .iter()
        .find(|event| event.kind == "home_extension.invoke.cancelled")
        .expect("cancellation should be audited");
    assert_eq!(
        event
            .payload
            .pointer("/status")
            .and_then(serde_json::Value::as_str),
        Some("not_in_flight")
    );
    assert_eq!(
        event
            .payload
            .pointer("/home_user_id")
            .and_then(serde_json::Value::as_str),
        Some(DEFAULT_LOCAL_USER_ID)
    );
    assert_eq!(
        event
            .payload
            .pointer("/caller_user_id")
            .and_then(serde_json::Value::as_str),
        Some(DEFAULT_LOCAL_USER_ID)
    );
    assert_eq!(
        event
            .payload
            .pointer("/lease_id")
            .and_then(serde_json::Value::as_str),
        Some("lease-1")
    );
    assert!(
        event
            .payload
            .pointer("/duration_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "cancellation audit should include duration"
    );
}

#[tokio::test]
async fn home_extension_invocation_adopts_first_worker_provider_run_for_active_prompt() {
    let (router, context, _agent_id) =
        remote_home_invocation_router_with_active_prompt("remote-extension-first-provider-run");
    let mut stale_context = context.clone();
    let metadata = crate::extension::RemoteExtensionInvocationMetadata::new(
        "provider-run-first",
        "home-only",
        None,
    );

    let cancelled = router
        .runtime_state
        .cancel_forwarded_home_extension_invocation(context, metadata)
        .await
        .expect("valid first provider run context should be accepted");
    assert!(!cancelled, "test cancellation was not in flight");
    stale_context.worker_provider_run_id = "provider-run-stale".to_string();
    let stale_metadata = crate::extension::RemoteExtensionInvocationMetadata::new(
        "provider-run-stale",
        "home-only",
        None,
    );
    let stale = router
        .runtime_state
        .cancel_forwarded_home_extension_invocation(stale_context, stale_metadata)
        .await
        .expect_err("second provider run should be rejected after adoption");
    assert!(
        stale
            .to_string()
            .contains("worker provider run does not match active remote agent binding"),
        "unexpected stale denial: {stale}"
    );
}

#[tokio::test]
async fn home_extension_invocation_rejects_wrong_worker_provider_run() {
    let config = DaemonConfig::for_tests();
    let home_kernel_id = config.daemon_id.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-home-extension-provider-run",
            "worktree-home-extension-provider-run",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = spawn_test_agent(
        &mut app,
        &session_id,
        "remote-extension-provider-run",
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
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
                worker_worktree_id: None,
            },
        )
        .expect("agent should be remote-backed");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4)
        .with_relay_peer_authority(crate::runtime::relay_peer_authority::test_peer_authority(
            "worker-kernel",
        ));
    let metadata = crate::extension::RemoteExtensionInvocationMetadata::new(
        "provider-run-2",
        "home-only",
        None,
    );
    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id,
        home_session_id: session_id,
        home_agent_id: agent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-2".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };

    let denied = router
        .runtime_state
        .cancel_forwarded_home_extension_invocation(context, metadata)
        .await
        .expect_err("wrong worker provider run should be denied");
    assert!(
        denied
            .to_string()
            .contains("worker provider run does not match active remote agent binding"),
        "unexpected denial: {denied}"
    );
}

/// A worker's request to the home kernel is refused on its handle and host before the home
/// Vault is asked to unlock: a refusal must not raise an unlock card for the owner.
#[tokio::test]
async fn a_forwarded_request_the_handle_refuses_does_not_ask_the_locked_home_vault_to_unlock() {
    let _environment = crate::env_lock::lock();
    let private = crate::test_support::TestWorktree::new("forwarded-vault-refusal");
    std::env::set_var("CHARIOX_HOME", private.path().join("home"));
    let vault = private.path().join("vault.json");
    crate::secret::create_chariox_encrypted_vault_for_test(&vault, "refusal passphrase").unwrap();
    crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .upsert(crate::config::UserCredentialConfig {
            id: "bound_key".into(),
            description: None,
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: "bound_key".into(),
            },
            allowed_hosts: vec!["api.example.com".into()],
            allowed_uses: vec![crate::config::UserCredentialUse::Http],
            injection: crate::config::UserCredentialInjectionConfig::Header {
                name: "authorization".into(),
                value: "Bearer ${secret}".into(),
            },
            metadata: None,
        })
        .unwrap();
    let mut config = DaemonConfig::for_tests();
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path = vault.display().to_string();
    let (router, context, _agent_id) = remote_home_invocation_router_with_active_prompt_and_config(
        "forwarded-vault-refusal",
        config,
    );
    let session_id = context.home_session_id.clone();

    for arguments in [
        serde_json::json!({"credential_id": "bound_key", "url": "http://127.0.0.1:19/echo"}),
        serde_json::json!({"credential_id": "no_such_key", "url": "https://api.example.com/"}),
    ] {
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            router
                .runtime_state
                .dispatch_forwarded_home_credential_tool_call(
                    context.clone(),
                    crate::transport::runtime_tools::HTTP_REQUEST_WITH_CREDENTIAL_TOOL.to_string(),
                    arguments.clone(),
                ),
        )
        .await
        .unwrap_or_else(|_| {
            panic!("{arguments} waited for the Vault unlock instead of being refused")
        });
        assert!(outcome.is_err(), "{arguments} should be refused");
        let session = router
            .runtime_state
            .session_snapshot(&session_id)
            .await
            .unwrap();
        assert!(
            session.active_interactions().is_empty(),
            "{arguments} asked for the unlock before refusing"
        );
    }
}
