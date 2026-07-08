use super::*;

#[test]
fn local_request_api_sets_workspace_live_sync_mode_through_dedicated_request() {
    run_workspace_capability_test(
        "local_request_api_sets_workspace_live_sync_mode_through_dedicated_request",
        local_request_api_sets_workspace_live_sync_mode_through_dedicated_request_inner,
    );
}

fn local_request_api_sets_workspace_live_sync_mode_through_dedicated_request_inner() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "/tmp/arroba-worktree-sync-mode"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let (updated, effects) = match harness
        .dispatch(LocalDaemonRequest::SetWorkspaceLiveSyncMode(
            SetWorkspaceLiveSyncModeRequest {
                session_id: session.id().to_string(),
                mode: crate::config::WorkspaceLiveSyncMode::Tracked,
            },
        ))
        .expect("workspace live sync mode update should succeed")
    {
        LocalDaemonResponse::WorkspaceLiveSyncModeUpdated { session, effects } => {
            (session, effects)
        }
        _ => panic!("unexpected local response"),
    };

    assert_eq!(
        updated.workspace_live_sync_mode(),
        Some(crate::config::WorkspaceLiveSyncMode::Tracked)
    );
    let reload_effect = effects
        .iter()
        .find(|effect| effect.kind == "provider_reload")
        .expect("session live sync mode update should report provider reload effect");
    assert_eq!(reload_effect.path, "session.workspace_live_sync_mode");
    assert_eq!(
        reload_effect
            .provider_reload
            .as_ref()
            .expect("provider reload summary should be present")
            .reloaded,
        0
    );

    let events = match harness
        .dispatch(LocalDaemonRequest::QueryRecall(QueryRecallRequest {
            session_id: Some(session.id().to_string()),
            kind: Some("workspace_live_sync_mode_changed".to_string()),
            limit: Some(5),
            ..Default::default()
        }))
        .expect("workspace live sync audit query should succeed")
    {
        LocalDaemonResponse::RecallEvents { events, .. } => events,
        _ => panic!("unexpected local response"),
    };
    let event = events
        .iter()
        .find(|event| event.kind == crate::history::HistoryEventKind::WorkspaceLiveSyncModeChanged)
        .expect("workspace live sync mode change should be recorded");
    assert_eq!(event.session_id.as_deref(), Some(session.id()));
    assert_eq!(event.workspace_id.as_deref(), Some("workspace-1"));
    assert_eq!(
        event.metadata["caller_user_id"],
        serde_json::json!(crate::session::DEFAULT_LOCAL_USER_ID)
    );
    assert_eq!(event.metadata["previous_mode"], serde_json::Value::Null);
    assert_eq!(event.metadata["mode"], serde_json::json!("tracked"));
    assert_eq!(
        event.metadata["scope"],
        serde_json::json!("selected_workspace_worktree")
    );
    assert_eq!(
        event.metadata["other_repositories"],
        serde_json::json!("unrestricted")
    );
}

#[test]
fn local_request_api_accepts_managed_workspace_live_sync_config_policy() {
    run_workspace_capability_test(
        "local_request_api_accepts_managed_workspace_live_sync_config_policy",
        local_request_api_accepts_managed_workspace_live_sync_config_policy_inner,
    );
}

fn local_request_api_accepts_managed_workspace_live_sync_config_policy_inner() {
    let mut config = DaemonConfig::for_tests();
    config.user_config_path = std::env::temp_dir().join(format!(
        "arroba-tests/user-config-workspace-live-sync-{}.toml",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos()
    ));
    let harness = LocalRouterTestHarness::with_config(config);

    let updated = match harness
        .dispatch(LocalDaemonRequest::SetUserConfigValue(
            SetUserConfigValueRequest {
                path: "providers.workspace_live_sync".to_string(),
                value: "managed".to_string(),
            },
        ))
        .expect("managed workspace live sync policy should update config")
    {
        LocalDaemonResponse::UserConfigUpdated { config, .. } => config,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(
        updated.providers.workspace_live_sync.mode,
        crate::config::WorkspaceLiveSyncMode::Managed
    );
}

#[test]
fn local_request_api_reports_workspace_live_sync_ignore_rules() {
    run_workspace_capability_test(
        "local_request_api_reports_workspace_live_sync_ignore_rules",
        local_request_api_reports_workspace_live_sync_ignore_rules_inner,
    );
}

fn local_request_api_reports_workspace_live_sync_ignore_rules_inner() {
    let harness = LocalRouterTestHarness::new();
    let worktree = std::env::temp_dir().join(format!(
        "arroba-live-sync-ignore-status-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&worktree).expect("worktree should be created");
    std::fs::write(
        worktree.join(".gitignore"),
        "ignored/\n*.secret\n# comment\n!keep\n",
    )
    .expect("gitignore should be written");
    let worktree_id = worktree.to_string_lossy().to_string();

    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", &worktree_id),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    let status = match harness
        .dispatch(LocalDaemonRequest::GetWorkspaceLiveSyncStatus(
            GetWorkspaceLiveSyncStatusRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("workspace live sync status should succeed")
    {
        LocalDaemonResponse::WorkspaceLiveSyncStatus { status } => status,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(status.ignore.ignore_file.as_deref(), Some(".arrobaignore"));
    assert_eq!(status.ignore.rules, vec!["ignored/**", "*.secret"]);
    assert_eq!(
        std::fs::read_to_string(worktree.join(".arrobaignore"))
            .expect(".arrobaignore should initialize from .gitignore"),
        "ignored/\n*.secret\n# comment\n!keep\n"
    );
    std::fs::remove_dir_all(worktree).expect("worktree should clean up");
}
