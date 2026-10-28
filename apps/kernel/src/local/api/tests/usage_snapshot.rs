use super::*;

#[test]
fn usage_snapshot_reads_accounts_without_private_profile_fields() {
    crate::test_support::isolated_env_test!();
    let harness = LocalRouterTestHarness::new();
    let profile = harness.with_app_mut(|app| {
        app.provider_account_profile_registry()
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                "Usage account",
            )
            .unwrap()
    });
    let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "GetProviderUsageSnapshot": {}
    }))
    .expect("public usage snapshot request must be supported");
    let response = harness.dispatch(request).expect("usage read succeeds");
    let value = serde_json::to_value(response).unwrap();
    let accounts = value["ProviderUsageSnapshot"]["accounts"]
        .as_array()
        .unwrap();
    let account = accounts
        .iter()
        .find(|a| a["profile_id"] == profile.profile_id)
        .unwrap();
    assert_eq!(account["label"], "Usage account");
    assert_eq!(account["usage"]["availability"], "unavailable");
    assert!(account["usage"].get("meters").is_none());
    assert_eq!(
        account
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["label", "profile_id", "provider", "usage"]
    );
}

#[test]
fn usage_snapshot_scopes_agent_context_and_route_to_the_requested_agent() {
    crate::test_support::isolated_env_test!();
    let worktree = crate::test_support::TestWorktree::new("usage-snapshot");
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, agent, .. } => (session, agent),
        _ => panic!("expected session"),
    };
    let request: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "GetAgentUsageSnapshot": { "session_id": session.id(), "agent_id": agent.id() }
    }))
    .expect("agent usage snapshot request must be supported");
    let response = serde_json::to_value(harness.dispatch(request).unwrap()).unwrap();
    let snapshot = &response["AgentUsageSnapshot"]["snapshot"];
    assert_eq!(snapshot["agent_id"], agent.id());
    assert_eq!(snapshot["session_id"], session.id());
    assert!(
        snapshot.get("context").is_none(),
        "no context estimate before a provider reports it"
    );
    assert_eq!(snapshot["route"]["target"], session.host_daemon_id());
    assert!(snapshot["route"].get("via").is_none());
}

#[test]
fn usage_snapshot_preserves_observed_windows_credits_and_account_ownership() {
    crate::test_support::isolated_env_test!();
    let harness = LocalRouterTestHarness::new();
    let now = crate::session::unix_epoch_ms();
    let expected = serde_json::json!({
        "profile_id": "assigned-by-registry", "provider": "codex", "availability": "available",
        "source": "codex.account.rateLimits", "observed_at_ms": now,
        "meters": [
            {"meter_id":"primary", "label":"5-hour", "kind":"rolling_limit", "scope":"account", "used_percent":0.0, "window_duration_minutes":300, "resets_at_ms":now+1000, "state":"healthy", "source":"codex.account.rateLimits", "observed_at_ms":now},
            {"meter_id":"secondary", "label":"Weekly", "kind":"rolling_limit", "scope":"account", "used_percent":42.0, "window_duration_minutes":10080, "resets_at_ms":now+2000, "state":"healthy", "source":"codex.account.rateLimits", "observed_at_ms":now},
            {"meter_id":"credits", "label":"Credits", "kind":"credit_balance", "scope":"account", "remaining":0.0, "unit":"credits", "state":"exhausted", "source":"codex.account.rateLimits", "observed_at_ms":now}
        ]
    });
    let profile = harness.with_app_mut(|app| {
        let registry = app.provider_account_profile_registry();
        let owned = registry
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                "Owned usage",
            )
            .unwrap();
        registry
            .create_managed("foreign-owner", "codex", "Private account")
            .unwrap();
        registry
            .update_usage(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                &owned.profile_id,
                serde_json::from_value(expected.clone()).unwrap(),
            )
            .unwrap()
    });
    let response = harness
        .dispatch(LocalDaemonRequest::GetProviderUsageSnapshot(
            crate::local::GetProviderUsageSnapshotRequest {},
        ))
        .unwrap();
    let LocalDaemonResponse::ProviderUsageSnapshot { accounts } = response else {
        panic!("expected usage snapshot")
    };
    assert!(accounts
        .iter()
        .all(|account| account.label != "Private account"));
    let account = accounts
        .iter()
        .find(|account| account.profile_id == profile.profile_id)
        .unwrap();
    let mut expected = expected;
    expected["profile_id"] = serde_json::json!(profile.profile_id);
    assert_eq!(serde_json::to_value(&account.usage).unwrap(), expected);
}

#[test]
fn usage_snapshot_uses_agent_projection_and_never_serializes_remote_credentials() {
    crate::test_support::isolated_env_test!();
    let worktree = crate::test_support::TestWorktree::new("usage-snapshot-agent");
    let mut config = DaemonConfig::for_tests();
    config.daemon_alias = Some("PR reviewer".into());
    let harness = LocalRouterTestHarness::with_config(config);
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, agent, .. } => (session, agent),
        _ => panic!("expected session"),
    };
    harness.with_app_mut(|app| {
        let mut run = RuntimeProviderRun::from_control_capability_inference(
            "leased:usage-run",
            session.id().into(),
            Some(agent.id().into()),
            "codex".into(),
        );
        run.set_usage(crate::provider::ProviderRunTokenUsage {
            context_tokens: Some(500),
            context_window: Some(2000),
            ..Default::default()
        });
        app.provider_run_projection_store().update(run);
        app.agents()
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "review-worker".into(),
                    worker_machine_id: "review-machine".into(),
                    execution_lease_id: "private-lease".into(),
                    leased_agent_id: "private-worker-agent".into(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: Some("fixture-private-value".into()),
                    relay_peer_protocol_version: Some(195),
                },
            )
            .unwrap();
    });
    let request = || {
        LocalDaemonRequest::GetAgentUsageSnapshot(crate::local::GetAgentUsageSnapshotRequest {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
        })
    };
    let response = serde_json::to_value(harness.dispatch(request()).unwrap()).unwrap();
    let snapshot = &response["AgentUsageSnapshot"]["snapshot"];
    assert_eq!(snapshot["context"]["used_tokens"], 500);
    assert_eq!(snapshot["context"]["limit_tokens"], 2000);
    assert_eq!(snapshot["route"]["target"], "review-worker");
    assert_eq!(snapshot["route"]["via"], "PR reviewer");
    assert!(!response.to_string().contains("fixture-private-value"));
    assert_eq!(snapshot["route"].as_object().unwrap().len(), 4);
    let wrong =
        LocalDaemonRequest::GetAgentUsageSnapshot(crate::local::GetAgentUsageSnapshotRequest {
            session_id: session.id().into(),
            agent_id: "unknown-agent".into(),
        });
    assert!(harness.dispatch(wrong).is_err());
}

#[test]
fn usage_snapshot_answers_from_the_projection_while_the_provider_service_is_busy() {
    crate::test_support::isolated_env_test!();
    let worktree = crate::test_support::TestWorktree::new("usage-snapshot-busy-provider");
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree.session_request(),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, agent, .. } => (session, agent),
        _ => panic!("expected session"),
    };
    let providers = harness.with_app_mut(|app| {
        let mut run = RuntimeProviderRun::from_control_capability_inference(
            "leased:busy-provider-run",
            session.id().into(),
            Some(agent.id().into()),
            "codex".into(),
        );
        run.set_usage(crate::provider::ProviderRunTokenUsage {
            context_tokens: Some(1200),
            context_window: Some(4000),
            ..Default::default()
        });
        app.provider_run_projection_store().update(run);
        app.providers().clone()
    });
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let holder = {
        let release = std::sync::Arc::clone(&release);
        std::thread::spawn(move || {
            let _unrelated_provider_operation = providers.read();
            locked_tx.send(()).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while !release.load(std::sync::atomic::Ordering::SeqCst)
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        })
    };
    locked_rx.recv().unwrap();
    let started = std::time::Instant::now();
    let response = harness.dispatch(LocalDaemonRequest::GetAgentUsageSnapshot(
        crate::local::GetAgentUsageSnapshotRequest {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
        },
    ));
    let answered_in = started.elapsed();
    release.store(true, std::sync::atomic::Ordering::SeqCst);
    holder.join().unwrap();
    assert!(
        answered_in < std::time::Duration::from_secs(1),
        "usage read waited {answered_in:?} for an unrelated provider operation"
    );
    let response = serde_json::to_value(response.unwrap()).unwrap();
    let snapshot = &response["AgentUsageSnapshot"]["snapshot"];
    assert_eq!(snapshot["agent_id"], agent.id());
    assert_eq!(snapshot["context"]["used_tokens"], 1200);
    assert_eq!(snapshot["context"]["limit_tokens"], 4000);
}

#[test]
fn usage_snapshot_projects_claude_identity_without_private_profile_fields() {
    crate::test_support::isolated_env_test!();
    let harness = LocalRouterTestHarness::new();
    let profile = harness.with_app_mut(|app| {
        let registry = app.provider_account_profile_registry();
        let profile = registry
            .create_managed(crate::session::DEFAULT_LOCAL_USER_ID, "claude", "Work")
            .unwrap();
        registry
            .update_observation(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "claude",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                Some("work@example.test".into()),
                Some("pro".into()),
                None,
                None,
            )
            .unwrap()
    });
    let response = harness
        .dispatch(LocalDaemonRequest::GetProviderUsageSnapshot(
            crate::local::GetProviderUsageSnapshotRequest {},
        ))
        .unwrap();
    let value = serde_json::to_value(response).unwrap();
    let account = value["ProviderUsageSnapshot"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|account| account["profile_id"] == profile.profile_id)
        .unwrap();
    assert_eq!(account["identity_summary"], "work@example.test");
    assert_eq!(
        account
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "identity_summary",
            "label",
            "profile_id",
            "provider",
            "usage"
        ]
    );
}
