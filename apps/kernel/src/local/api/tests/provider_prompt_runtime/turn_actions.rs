use super::*;

#[test]
fn completed_native_tui_turn_projects_undo_action_for_tracked_session() {
    let root = temp_git_repo("native-tui-turn-actions");
    let proof_path = root.join("web-terminal-undo-proof.txt");
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "turn-actions-client".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected local response: {other:?}"),
    };
    let provider_run = harness.with_app_mut(|app| {
        app.launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "managed-dev-stub",
                "dev-stub",
                "default",
                "native-tui-idle",
            )
            .with_variant(Some("default".to_string()))
            .with_agent_id(agent.id())
            .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked)
            .with_client_interface(crate::provider::ProviderClientInterface::NativeTui),
        )
        .expect("provider launch should succeed")
    });
    assert!(
        provider_run.tracks_workspace_live_sync(),
        "tracked session should launch a tracked provider run"
    );
    let prompt = match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent.id().to_string()),
            prompt: "turn actions prompt".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should start")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            ..
        } => prompt,
        other => panic!("unexpected prompt submit response: {other:?}"),
    };
    fs::write(&proof_path, "created by native tui turn\n").expect("proof file should be written");
    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("prompt completion should succeed")
    {
        LocalDaemonResponse::PromptCompleted { completion } => {
            assert_eq!(completion.completed.id(), prompt.id());
            assert_eq!(completion.completed.prompt(), prompt.prompt());
            assert_eq!(completion.completed.pending_prompt_id(), None);
        }
        other => panic!("unexpected completion response: {other:?}"),
    }

    let agent_activity = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { agent_activity, .. } => agent_activity,
        other => panic!("unexpected session state response: {other:?}"),
    };
    let completed_turn = agent_activity
        .get(agent.id())
        .and_then(|activity| activity.last_completed_turn.as_ref())
        .expect("completed tracked turn should project a turn action");
    assert_eq!(completed_turn.agent_id, agent.id());
    assert_eq!(completed_turn.prompt_id, prompt.id());
    assert!(completed_turn.undo_available);
    assert_eq!(
        completed_turn.changed_paths,
        vec!["web-terminal-undo-proof.txt".to_string()]
    );
    let event_projection = harness.with_app_mut(|app| {
        crate::runtime::projection::SessionSnapshotProjection::from_daemon_app(app, session.id(), 0)
            .expect("subscription projection should load")
    });
    let event_completed_turn = event_projection
        .agent_activity
        .get(agent.id())
        .and_then(|activity| activity.last_completed_turn.as_ref())
        .expect("subscription projection should include completed tracked turn action");
    assert_eq!(event_completed_turn.prompt_id, prompt.id());
    assert!(event_completed_turn.undo_available);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn queued_native_tui_turn_projects_undo_action_after_provider_launch() {
    run_with_large_test_stack(
        "queued-native-tui-turn-actions",
        queued_native_tui_turn_projects_undo_action_after_provider_launch_inner,
    );
}

fn queued_native_tui_turn_projects_undo_action_after_provider_launch_inner() {
    let root = temp_git_repo("queued-native-tui-turn-actions");
    let proof_path = root.join("web-terminal-undo-proof.txt");
    let mut config = DaemonConfig::for_tests();
    config.provider_runtime_init_delay_ms = 50;
    let harness = LocalRouterTestHarness::with_config(config);
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "queued-turn-actions-client".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected local response: {other:?}"),
    };
    let provider_run = match harness
        .dispatch(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: Some(agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: "dev-stub".to_string(),
                account_profile: "default".to_string(),
                model: "default".to_string(),
                variant: Some("default".to_string()),
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: true,
            },
        ))
        .expect("provider launch should be accepted")
    {
        LocalDaemonResponse::ProviderRunLaunched { provider_run }
        | LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => provider_run,
        other => panic!("unexpected provider launch response: {other:?}"),
    };

    let prompt = match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent.id().to_string()),
            prompt: "queued turn actions prompt".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should queue while native TUI provider starts")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Queued { prompt },
            ..
        } => prompt,
        other => panic!("unexpected prompt submit response: {other:?}"),
    };

    let active_session = harness.wait_for_session_where(
        session.id(),
        "queued native TUI prompt should become active after provider launch",
        |session| {
            session
                .active_prompt_for_agent(agent.id())
                .is_some_and(|active| {
                    active.prompt() == prompt.prompt()
                        && active.pending_prompt_id().is_none()
                        && active.id() != prompt.id()
                })
        },
    );
    let promoted_prompt_id = active_session
        .active_prompt_for_agent(agent.id())
        .expect("queued prompt should be active")
        .id()
        .to_string();
    fs::write(&proof_path, "created by queued native tui turn\n")
        .expect("proof file should be written");
    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("queued prompt completion should succeed")
    {
        LocalDaemonResponse::PromptCompleted { completion } => {
            assert_eq!(completion.completed.id(), promoted_prompt_id);
            assert_eq!(completion.completed.prompt(), prompt.prompt());
            assert_eq!(completion.completed.pending_prompt_id(), None);
        }
        other => panic!("unexpected completion response: {other:?}"),
    }

    let agent_activity = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { agent_activity, .. } => agent_activity,
        other => panic!("unexpected session state response: {other:?}"),
    };
    let completed_turn = agent_activity
        .get(agent.id())
        .and_then(|activity| activity.last_completed_turn.as_ref())
        .expect("completed queued tracked turn should project a turn action");
    assert_eq!(completed_turn.agent_id, agent.id());
    assert_eq!(completed_turn.prompt_id, promoted_prompt_id);
    assert_eq!(completed_turn.provider_run_id, provider_run.id());
    assert!(completed_turn.undo_available);
    assert_eq!(
        completed_turn.changed_paths,
        vec!["web-terminal-undo-proof.txt".to_string()]
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn undo_turn_request_restores_workspace_states_and_preserves_head() {
    run_with_large_test_stack(
        "turn-undo-restores-workspace",
        undo_turn_request_restores_workspace_states_and_preserves_head_inner,
    );
}

fn undo_turn_request_restores_workspace_states_and_preserves_head_inner() {
    let root = temp_git_repo("turn-undo-restores-workspace");
    fs::create_dir_all(root.join("src")).expect("src directory should be created");
    fs::write(root.join("src/existing.txt"), "existing before\n")
        .expect("existing file should be written");
    fs::write(root.join("src/delete.txt"), "delete before\n")
        .expect("deleted file seed should be written");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "seed undo files"]);
    fs::write(root.join("src/dirty.txt"), "dirty before turn\n")
        .expect("pre-existing dirty file should be written");

    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    let before = crate::git_observer::capture_turn_snapshot(crate::git_observer::GitTurnContext {
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        provider: "dev-stub".to_string(),
        model: "default".to_string(),
        provider_run_id: "provider-run-undo".to_string(),
        provider_session_id: None,
        prompt_id: "prompt-undo".to_string(),
        turn_id: "turn-undo".to_string(),
        source_attachment_id: Some("attachment-undo".to_string()),
        prompt_origin: Some(crate::session::PromptOrigin::Chariox),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        started_at_ms: Some(crate::session::unix_epoch_ms()),
        worktree_path: root.clone(),
        workspace_live_sync_tracked: true,
        machine_id: None,
        prompt_summary: "make committed and dirty changes".to_string(),
    })
    .expect("pre-turn snapshot should capture");

    fs::write(root.join("src/existing.txt"), "existing after\n")
        .expect("existing file should change");
    fs::write(root.join("src/added.txt"), "added after\n").expect("added file should be written");
    fs::remove_file(root.join("src/delete.txt")).expect("file should be deleted");
    fs::write(root.join("src/dirty.txt"), "dirty after turn\n").expect("dirty file should change");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "agent committed turn"]);
    let head_after_turn = git_output(&root, &["rev-parse", "HEAD"]);
    let after = crate::git_observer::capture_turn_snapshot(crate::git_observer::GitTurnContext {
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        provider: "dev-stub".to_string(),
        model: "default".to_string(),
        provider_run_id: "provider-run-undo".to_string(),
        provider_session_id: None,
        prompt_id: "prompt-undo".to_string(),
        turn_id: "turn-undo".to_string(),
        source_attachment_id: before.source_attachment_id.clone(),
        prompt_origin: before.prompt_origin,
        external_provider: before.external_provider.clone(),
        external_provider_session_id: before.external_provider_session_id.clone(),
        external_provider_turn_id: before.external_provider_turn_id.clone(),
        started_at_ms: before.started_at_ms,
        worktree_path: root.clone(),
        workspace_live_sync_tracked: true,
        machine_id: None,
        prompt_summary: "make committed and dirty changes".to_string(),
    })
    .expect("post-turn snapshot should capture");
    let change =
        crate::git_observer::tracked_workspace_live_sync_change_after_turn(&before, &after)
            .expect("committed turn should produce reversible changes");
    harness.with_app_mut(|app| {
        app.completed_git_turn_snapshot_store().record(
            crate::git_observer::CompletedGitTurnSnapshot::new(
                before,
                after,
                Some(change),
                crate::session::unix_epoch_ms(),
            ),
        );
    });

    let completed_turn = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { agent_activity, .. } => agent_activity
            .get(agent.id())
            .and_then(|activity| activity.last_completed_turn.clone())
            .expect("completed turn should project"),
        other => panic!("unexpected session state response: {other:?}"),
    };
    assert!(completed_turn.undo_available);
    assert_eq!(
        completed_turn.changed_paths,
        vec![
            "src/added.txt".to_string(),
            "src/delete.txt".to_string(),
            "src/dirty.txt".to_string(),
            "src/existing.txt".to_string(),
        ]
    );

    let undo = match harness
        .dispatch(LocalDaemonRequest::UndoTurn(
            crate::local::UndoTurnRequest {
                session_id: session.id().to_string(),
                agent_ref: None,
                turn_ref: None,
            },
        ))
        .expect("focused agent undo should succeed")
    {
        LocalDaemonResponse::TurnUndone { result } => result,
        other => panic!("unexpected undo response: {other:?}"),
    };

    assert_eq!(undo.agent_id, agent.id());
    assert_eq!(undo.turn_id, completed_turn.turn_id);
    assert_eq!(git_output(&root, &["rev-parse", "HEAD"]), head_after_turn);
    assert_eq!(
        fs::read_to_string(root.join("src/existing.txt")).expect("existing file should read"),
        "existing before\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("src/delete.txt")).expect("deleted file should be restored"),
        "delete before\n"
    );
    assert!(!root.join("src/added.txt").exists());
    assert_eq!(
        fs::read_to_string(root.join("src/dirty.txt")).expect("dirty file should read"),
        "dirty before turn\n"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn undo_turn_request_allows_noop_turns_without_workspace_changes() {
    run_with_large_test_stack(
        "turn-undo-noop",
        undo_turn_request_allows_noop_turns_without_workspace_changes_inner,
    );
}

fn undo_turn_request_allows_noop_turns_without_workspace_changes_inner() {
    let root = temp_git_repo("turn-undo-noop");
    fs::write(root.join("README.md"), "seed\n").expect("seed file should be written");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "seed noop undo repo"]);

    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    let before = crate::git_observer::capture_turn_snapshot(crate::git_observer::GitTurnContext {
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        provider: "dev-stub".to_string(),
        model: "default".to_string(),
        provider_run_id: "provider-run-noop".to_string(),
        provider_session_id: None,
        prompt_id: "prompt-noop".to_string(),
        turn_id: "turn-noop".to_string(),
        source_attachment_id: Some("attachment-noop".to_string()),
        prompt_origin: Some(crate::session::PromptOrigin::Chariox),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        started_at_ms: Some(crate::session::unix_epoch_ms()),
        worktree_path: root.clone(),
        workspace_live_sync_tracked: false,
        machine_id: None,
        prompt_summary: "inspect without editing".to_string(),
    })
    .expect("pre-turn snapshot should capture");
    let after = crate::git_observer::capture_turn_snapshot(crate::git_observer::GitTurnContext {
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        provider: "dev-stub".to_string(),
        model: "default".to_string(),
        provider_run_id: "provider-run-noop".to_string(),
        provider_session_id: None,
        prompt_id: "prompt-noop".to_string(),
        turn_id: "turn-noop".to_string(),
        source_attachment_id: before.source_attachment_id.clone(),
        prompt_origin: before.prompt_origin,
        external_provider: before.external_provider.clone(),
        external_provider_session_id: before.external_provider_session_id.clone(),
        external_provider_turn_id: before.external_provider_turn_id.clone(),
        started_at_ms: before.started_at_ms,
        worktree_path: root.clone(),
        workspace_live_sync_tracked: false,
        machine_id: None,
        prompt_summary: "inspect without editing".to_string(),
    })
    .expect("post-turn snapshot should capture");
    harness.with_app_mut(|app| {
        app.completed_git_turn_snapshot_store().record(
            crate::git_observer::CompletedGitTurnSnapshot::new(
                before,
                after,
                None,
                crate::session::unix_epoch_ms(),
            ),
        );
    });

    let completed_turn = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { agent_activity, .. } => agent_activity
            .get(agent.id())
            .and_then(|activity| activity.last_completed_turn.clone())
            .expect("completed turn should project"),
        other => panic!("unexpected session state response: {other:?}"),
    };
    assert!(
        completed_turn.undo_available,
        "latest turn undo should be available even with no changed paths"
    );
    assert!(completed_turn.changed_paths.is_empty());
    assert_eq!(completed_turn.undo_unavailable_reason, None);

    let undo = match harness
        .dispatch(LocalDaemonRequest::UndoTurn(
            crate::local::UndoTurnRequest {
                session_id: session.id().to_string(),
                agent_ref: None,
                turn_ref: None,
            },
        ))
        .expect("noop undo should succeed")
    {
        LocalDaemonResponse::TurnUndone { result } => result,
        other => panic!("unexpected undo response: {other:?}"),
    };

    assert_eq!(undo.agent_id, agent.id());
    assert_eq!(undo.turn_id, completed_turn.turn_id);
    assert!(undo.reverted_paths.is_empty());
    assert!(undo.path_results.is_empty());

    let completed_turn = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { agent_activity, .. } => agent_activity
            .get(agent.id())
            .and_then(|activity| activity.last_completed_turn.clone())
            .expect("completed turn should still project"),
        other => panic!("unexpected session state response: {other:?}"),
    };
    assert!(!completed_turn.undo_available);
    assert_eq!(
        completed_turn.undo_unavailable_reason.as_deref(),
        Some("turn already undone")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn undo_turn_request_conflict_fails_without_partial_writes() {
    run_with_large_test_stack(
        "turn-undo-conflict",
        undo_turn_request_conflict_fails_without_partial_writes_inner,
    );
}

fn undo_turn_request_conflict_fails_without_partial_writes_inner() {
    let root = temp_git_repo("turn-undo-conflict");
    fs::create_dir_all(root.join("src")).expect("src directory should be created");
    fs::write(root.join("src/conflict.txt"), "before\n").expect("conflict seed should write");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "seed conflict file"]);

    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "turn-undo-conflict-client".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected local response: {other:?}"),
    };
    let _provider_run = harness.with_app_mut(|app| {
        app.launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "managed-dev-stub",
                "dev-stub",
                "default",
                "native-tui-idle",
            )
            .with_agent_id(agent.id())
            .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked)
            .with_client_interface(crate::provider::ProviderClientInterface::NativeTui),
        )
        .expect("provider launch should succeed")
    });

    match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent.id().to_string()),
            prompt: "make conflicting changes".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should start")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { .. },
            ..
        } => {}
        other => panic!("unexpected prompt submit response: {other:?}"),
    };
    fs::write(root.join("src/conflict.txt"), "after\n").expect("conflict file should change");
    fs::write(root.join("src/added.txt"), "added\n").expect("added file should write");
    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("prompt completion should succeed")
    {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        other => panic!("unexpected completion response: {other:?}"),
    }
    fs::write(root.join("src/conflict.txt"), "post-turn user edit\n")
        .expect("post-turn conflict should write");

    let error = harness
        .dispatch(LocalDaemonRequest::UndoTurn(
            crate::local::UndoTurnRequest {
                session_id: session.id().to_string(),
                agent_ref: Some(agent.id().to_string()),
                turn_ref: None,
            },
        ))
        .expect_err("post-turn edit should block undo");
    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "turn undo");
            assert!(message.contains("workspace changed after the turn"));
            assert!(message.contains("src/conflict.txt"));
        }
        other => panic!("unexpected error: {other}"),
    }
    assert_eq!(
        fs::read_to_string(root.join("src/conflict.txt")).expect("conflict file should read"),
        "post-turn user edit\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("src/added.txt")).expect("added file should remain"),
        "added\n"
    );

    let completed_turn = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { agent_activity, .. } => agent_activity
            .get(agent.id())
            .and_then(|activity| activity.last_completed_turn.clone())
            .expect("completed turn should still project"),
        other => panic!("unexpected session state response: {other:?}"),
    };
    assert!(
        completed_turn.undo_available,
        "failed undo must not mark the turn undone"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn turn_actions_without_agent_ref_require_focused_agent() {
    let harness = LocalRouterTestHarness::new();
    let (session, _agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-no-focus", "worktree-no-focus"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    harness.with_app_mut(|app| {
        app.sessions_mut()
            .set_focused_agent(session.id(), None)
            .expect("focus should clear");
    });

    for request in [
        LocalDaemonRequest::UndoTurn(crate::local::UndoTurnRequest {
            session_id: session.id().to_string(),
            agent_ref: None,
            turn_ref: None,
        }),
        LocalDaemonRequest::ForkAgent(crate::local::ForkAgentRequest {
            session_id: session.id().to_string(),
            source_agent_ref: None,
            alias: None,
        }),
    ] {
        let error = harness
            .dispatch(request)
            .expect_err("omitted agent ref should require focus");
        match error {
            DaemonError::LocalTransport { message, .. } => {
                assert!(message.contains("agent reference is required because no agent is focused"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}

#[test]
fn fork_agent_clones_config_and_launches_provider() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(fork_agent_clones_config_and_launches_provider_inner)
        .expect("fork test thread should spawn")
        .join()
        .expect("fork test thread should complete");
}

fn fork_agent_clones_config_and_launches_provider_inner() {
    let root = temp_git_repo("agent-fork-handoff");
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy()),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    let source = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("source".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("model-source".to_string()),
            effort: Some("high".to_string()),
            execution_mode: Some(AgentExecutionMode::Plan),
            permission_level: Some(AgentPermissionLevel::Required),
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("source agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        other => panic!("unexpected spawn response: {other:?}"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "fork-handoff-client".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected local response: {other:?}"),
    };
    let source_run = harness.with_app_mut(|app| {
        app.providers_mut()
            .launch_run_detached(
                crate::provider::LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "model-source",
                )
                .with_agent_id(source.id())
                .with_variant(Some("high".to_string()))
                .with_execution_mode(AgentExecutionMode::Plan)
                .with_permission_level(AgentPermissionLevel::Required),
            )
            .expect("source provider run should be available for fork setup")
    });

    match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(source.id().to_string()),
            prompt: "remember source context".to_string(),
            attachments: Vec::new(),
        }))
        .expect("source prompt should submit")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { .. },
            ..
        } => {}
        other => panic!("unexpected source prompt response: {other:?}"),
    }
    match harness
        .dispatch(LocalDaemonRequest::AppendNativeProviderOutput(
            AppendNativeProviderOutputRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                provider_run_id: source_run.id().to_string(),
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: None,
                text: "source answer for fork handoff\n".to_string(),
            },
        ))
        .expect("source output should append")
    {
        LocalDaemonResponse::TerminalOutput { .. } => {}
        other => panic!("unexpected source output response: {other:?}"),
    }
    let (forked, forked_run) = match harness
        .dispatch(LocalDaemonRequest::ForkAgent(
            crate::local::ForkAgentRequest {
                session_id: session.id().to_string(),
                source_agent_ref: Some(source.id().to_string()),
                alias: Some("forked".to_string()),
            },
        ))
        .expect("agent fork should succeed")
    {
        LocalDaemonResponse::AgentForked {
            source_agent_id,
            agent,
            provider_run,
            session: forked_session,
        } => {
            assert_eq!(source_agent_id, source.id());
            assert!(forked_session
                .agents()
                .iter()
                .any(|session_agent| session_agent.id() == agent.id()));
            (agent, provider_run)
        }
        other => panic!("unexpected fork response: {other:?}"),
    };
    assert_eq!(forked.alias(), Some("forked"));
    assert_eq!(forked.provider(), "dev-stub");
    assert_eq!(forked.model(), Some("model-source"));
    assert_eq!(forked.effort(), Some("high"));
    assert_eq!(
        forked.execution_mode_override(),
        Some(AgentExecutionMode::Plan)
    );
    assert_eq!(
        forked.permission_level_override(),
        Some(AgentPermissionLevel::Required)
    );
    assert_eq!(forked_run.agent_instance_id(), Some(forked.id()));
    assert_eq!(forked_run.provider(), source_run.provider());
    assert_eq!(forked_run.model(), source_run.model());
    assert_eq!(forked_run.variant(), source_run.variant());

    let _ = fs::remove_dir_all(root);
}
