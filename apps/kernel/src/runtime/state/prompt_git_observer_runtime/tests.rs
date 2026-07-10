use super::*;
use crate::workspace_live_sync_journal::workspace_live_sync_notice_messages;
use std::sync::Arc;
use tokio::sync::Mutex;

async fn owned_runtime_state(app: &Arc<Mutex<crate::DaemonApp>>) -> KernelRuntimeState {
    let (
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        history_store,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    ) = {
        let app_locked = app.lock().await;
        (
            app_locked.config_projection_store(),
            app_locked.session_state_store(),
            app_locked.agents().clone(),
            app_locked.attachments().clone(),
            app_locked.providers().clone(),
            app_locked.provider_process_tracking_store(),
            app_locked.slices(),
            app_locked.session_state_projection_store(),
            app_locked.provider_run_projection_store(),
            app_locked.history_store(),
            app_locked.operational_history_store(),
            app_locked.durable_state_store(),
            app_locked.prompt_state_owner(),
            app_locked.active_turn_store(),
            app_locked.prompt_activity_store(),
            app_locked.prompt_workspace_claim_store(),
            app_locked.structured_output_record_store(),
            app_locked.terminal_stream_store(),
            app_locked.workflow_design_event_store(),
            app_locked.metaagent_event_store(),
            app_locked.workspace_coordinator(),
        )
    };
    KernelRuntimeState::new_with_owned_state(
        Arc::clone(app),
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        history_store,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    )
}

#[test]
fn workspace_live_sync_summary_names_targets_paths_and_next_action() {
    let messages = workspace_live_sync_notice_messages(
        &change("managed_workspace_live_sync"),
        &[target_result(vec![
            path_result(
                "src/lib.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied,
                "applied cleanly",
            ),
            path_result(
                "src/main.rs",
                crate::git_observer::WorkspaceLiveSyncApplyStatus::Rebased,
                "rebased over non-overlapping target change",
            ),
        ])],
    );

    assert_eq!(messages.len(), 1);
    let summary = &messages[0];
    assert!(summary.contains("Workspace live sync managed summary"));
    assert!(summary.contains("source agent `agent-1`"));
    assert!(
        summary.contains("target user `user-2` worktree `/tmp/target` path `src/lib.rs` applied")
    );
    assert!(
        summary.contains("target user `user-2` worktree `/tmp/target` path `src/main.rs` rebased")
    );
    assert!(summary.contains("Next action: none."));
}

#[test]
fn workspace_live_sync_conflict_notice_names_source_target_path_and_action() {
    let messages = workspace_live_sync_notice_messages(
        &change("tracked_workspace_live_sync"),
        &[target_result(vec![path_result(
            "src/lib.rs",
            crate::git_observer::WorkspaceLiveSyncApplyStatus::SkippedConflict,
            "overlapping edits",
        )])],
    );

    assert_eq!(messages.len(), 2);
    let conflict = &messages[0];
    assert!(conflict.contains("Workspace live sync conflict"));
    assert!(conflict.contains("source agent `agent-1`"));
    assert!(conflict.contains("changed `src/lib.rs`"));
    assert!(conflict.contains("target user `user-2` worktree `/tmp/target`"));
    assert!(conflict.contains("overlapping edits"));
    assert!(conflict.contains("Next action: assign a resolver agent"));
    assert!(messages[1].contains("conflicts=1"));
    assert!(messages[1].contains("Next action: review the listed conflict/failure notices"));
}

#[test]
fn workspace_live_sync_failed_io_notice_names_source_target_path_and_action() {
    let messages = workspace_live_sync_notice_messages(
        &change("tracked_workspace_live_sync"),
        &[target_result(vec![path_result(
            "src/lib.rs",
            crate::git_observer::WorkspaceLiveSyncApplyStatus::FailedIo,
            "permission denied",
        )])],
    );

    assert_eq!(messages.len(), 2);
    let failure = &messages[0];
    assert!(failure.contains("Workspace live sync failed"));
    assert!(failure.contains("source agent `agent-1`"));
    assert!(failure.contains("changed `src/lib.rs`"));
    assert!(failure.contains("target user `user-2` worktree `/tmp/target`"));
    assert!(failure.contains("permission denied"));
    assert!(failure.contains("Next action: verify the target worktree is attached and writable"));
    assert!(messages[1].contains("failed_io=1"));
}

#[tokio::test]
async fn workspace_live_sync_status_reads_durable_only_results_by_session_and_kind() {
    let app = Arc::new(Mutex::new(
        crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed"),
    ));
    let runtime = owned_runtime_state(&app).await;
    let session_id = "session-durable-only";
    let mut persisted = target_result(vec![path_result(
        "src/lib.rs",
        crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied,
        "applied remotely",
    )]);
    persisted.session_id = session_id.to_string();

    runtime
        .owned
        .durable_state_store
        .append_event(
            "session.updated",
            Some(session_id.to_string()),
            serde_json::json!({ "unrelated": "x".repeat(1_000_000) }),
        )
        .expect("unrelated event should append");
    runtime
        .owned
        .durable_state_store
        .append_event(
            "workspace_live_sync.target_results_recorded",
            Some(session_id.to_string()),
            serde_json::json!({ "target_results": [persisted] }),
        )
        .expect("target result event should append");
    runtime
        .owned
        .durable_state_store
        .append_event(
            "workspace_live_sync.target_results_recorded",
            Some("other-session".to_string()),
            serde_json::json!({ "target_results": [] }),
        )
        .expect("other session event should append");

    let results = runtime.workspace_live_sync_target_results(session_id);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].session_id, session_id);
    assert_eq!(results[0].path_results[0].message, "applied remotely");
}

#[test]
fn workspace_live_sync_source_attachment_skip_matches_root_and_origin_machine_or_kernel() {
    let source = attachment("/tmp/source", "kernel-source");
    let same_path_same_machine = crate::session::WorkspaceLinkAttachment::new(
        "link-1",
        "local",
        "machine-source",
        "kernel-remote",
        "/tmp/source/",
        Some("main".to_string()),
        Some("fingerprint".to_string()),
    );
    let same_path_remote_machine = crate::session::WorkspaceLinkAttachment::new(
        "link-1",
        "local",
        "machine-remote",
        "kernel-remote",
        "/tmp/source/",
        Some("main".to_string()),
        Some("fingerprint".to_string()),
    );
    let same_kernel_other_path = attachment("/tmp/target", "kernel-source");
    let source_root = crate::session::normalize_workspace_link_repo_root("/tmp/source/");

    assert!(workspace_live_sync_should_skip_source_attachment(
        &source,
        &source_root,
        "kernel-source",
        "machine-source"
    ));
    assert!(workspace_live_sync_should_skip_source_attachment(
        &same_path_same_machine,
        &source_root,
        "kernel-source",
        "machine-source"
    ));
    assert!(!workspace_live_sync_should_skip_source_attachment(
        &same_path_remote_machine,
        &source_root,
        "kernel-source",
        "machine-source"
    ));
    assert!(!workspace_live_sync_should_skip_source_attachment(
        &same_kernel_other_path,
        &source_root,
        "kernel-source",
        "machine-source"
    ));
}

#[test]
fn forwarded_workspace_live_sync_apply_requires_matching_link_attachment() {
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext {
        home_session_id: "session-1".to_string(),
        link_id: "link-1".to_string(),
        link_name: "pair".to_string(),
        source_agent_id: "agent-1".to_string(),
        source_worktree_path: "/tmp/source".to_string(),
        target_user_id: "user-2".to_string(),
        target_machine_id: "machine-2".to_string(),
        target_kernel_id: "kernel-target".to_string(),
        target_repo_root: "/tmp/target/".to_string(),
    };
    let normalized_target_root = crate::session::normalize_workspace_link_repo_root("/tmp/target/");
    let linked_target = crate::session::WorkspaceLinkAttachment::new(
        "link-1",
        "user-2",
        "machine-2",
        "kernel-target",
        "/tmp/target",
        Some("main".to_string()),
        None,
    );
    let wrong_link_same_root = crate::session::WorkspaceLinkAttachment::new(
        "link-2",
        "user-2",
        "machine-2",
        "kernel-target",
        "/tmp/target",
        Some("main".to_string()),
        None,
    );
    let wrong_kernel_same_link = crate::session::WorkspaceLinkAttachment::new(
        "link-1",
        "user-2",
        "machine-2",
        "other-kernel",
        "/tmp/target",
        Some("main".to_string()),
        None,
    );

    assert!(forwarded_workspace_live_sync_attachment_matches_context(
        &linked_target,
        &context,
        "kernel-target",
        &normalized_target_root,
    ));
    assert!(!forwarded_workspace_live_sync_attachment_matches_context(
        &wrong_link_same_root,
        &context,
        "kernel-target",
        &normalized_target_root,
    ));
    assert!(!forwarded_workspace_live_sync_attachment_matches_context(
        &wrong_kernel_same_link,
        &context,
        "kernel-target",
        &normalized_target_root,
    ));
}

#[tokio::test]
async fn workspace_live_sync_fans_out_between_second_user_attached_forks() {
    let root = std::env::temp_dir().join(format!(
        "arroba-workspace-live-sync-collab-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let source = root.join("source");
    let target = root.join("target-fork");
    init_repo_with_file(&source, "src/lib.rs", "seed\n");
    init_repo_with_file(&target, "src/lib.rs", "seed\n");

    let mut app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            source.to_string_lossy(),
            source.to_string_lossy(),
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "terminal-client-1",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("terminal attachment should attach");
    let session_id = session.id().to_string();
    let attachment_id = attachment.id().to_string();
    let daemon_id = app.config_projection_store().snapshot().daemon_id;
    let machine_id = app.config_projection_store().snapshot().host_machine_id;
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let (_, invite) = runtime
        .create_session_invite(
            &session_id,
            "invite-user-2".to_string(),
            "local".to_string(),
            None,
            None,
            crate::session::CollaborationLevel::Full,
        )
        .expect("invite should be created");
    runtime
        .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("user-2 should join");
    let (_, link) = runtime
        .create_workspace_link(&session_id, "shared-fork".to_string(), "local".to_string())
        .expect("workspace link should be created");
    let source_branch = crate::git_observer::workspace_live_sync_git_branch(&source);
    let target_branch = crate::git_observer::workspace_live_sync_git_branch(&target);
    let source_fingerprint = crate::git_observer::workspace_live_sync_repo_fingerprint(&source);
    let target_fingerprint = crate::git_observer::workspace_live_sync_repo_fingerprint(&target);
    runtime
        .attach_workspace_link(
            &session_id,
            link.link_id(),
            "local".to_string(),
            machine_id.clone(),
            daemon_id.clone(),
            source.to_string_lossy().to_string(),
            source_branch,
            source_fingerprint,
        )
        .expect("source worktree should attach");
    runtime
        .attach_workspace_link(
            &session_id,
            link.link_id(),
            "user-2".to_string(),
            machine_id,
            daemon_id,
            target.to_string_lossy().to_string(),
            target_branch,
            target_fingerprint,
        )
        .expect("second user target worktree should attach");

    std::fs::write(source.join("src/lib.rs"), "seed\nagent change\n")
        .expect("source should change");
    let change = workspace_live_sync_text_change(
        &session_id,
        "agent-source",
        "provider-run-1",
        "prompt-1",
        &source,
        "seed\n",
        "seed\nagent change\n",
    );

    runtime
        .record_and_fanout_workspace_live_sync_change(change, None, None)
        .await;

    let notices = runtime
        .drain_notice_records(&session_id, &attachment_id)
        .await;
    assert!(
        notices.iter().any(|notice| {
            notice.session_id == session_id
                && notice.provider_run_id.as_deref() == Some("provider-run-1")
                && notice
                    .message
                    .contains("Workspace live sync tracked turn summary")
                && notice.message.contains("source agent `agent-source`")
                && notice.message.contains("target user `user-2`")
                && notice.message.contains("Next action: none")
        }),
        "workspace live sync fanout should deliver a runtime notice: {notices:?}"
    );

    assert_eq!(
        std::fs::read_to_string(target.join("src/lib.rs")).expect("target file should be readable"),
        "seed\nagent change\n"
    );
    let results = runtime.workspace_live_sync_target_results(&session_id);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].target_user_id, "user-2");
    assert_eq!(results[0].source_agent_id, "agent-source");
    assert_eq!(results[0].path_results[0].path, "src/lib.rs");
    assert_eq!(
        results[0].path_results[0].status,
        crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied
    );

    std::fs::write(
        target.join("src/lib.rs"),
        "seed\nagent change\npeer change\n",
    )
    .expect("target should change");
    let peer_change = workspace_live_sync_text_change(
        &session_id,
        "agent-peer",
        "provider-run-2",
        "prompt-2",
        &target,
        "seed\nagent change\n",
        "seed\nagent change\npeer change\n",
    );

    runtime
        .record_and_fanout_workspace_live_sync_change(peer_change, None, None)
        .await;

    assert_eq!(
        std::fs::read_to_string(source.join("src/lib.rs")).expect("source file should be readable"),
        "seed\nagent change\npeer change\n"
    );
    let results = runtime.workspace_live_sync_target_results(&session_id);
    assert_eq!(results.len(), 2);
    let reverse_result = results
        .iter()
        .find(|result| result.source_agent_id == "agent-peer")
        .expect("peer change should produce a reverse target result");
    assert_eq!(reverse_result.target_user_id, "local");
    assert_eq!(reverse_result.path_results[0].path, "src/lib.rs");
    assert_eq!(
        reverse_result.path_results[0].status,
        crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn forwarded_workspace_live_sync_apply_notifies_target_session() {
    let root = std::env::temp_dir().join(format!(
        "arroba-forwarded-workspace-live-sync-notice-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    let target = root.join("target");
    init_repo_with_file(&target, "src/lib.rs", "seed\n");

    let mut config = crate::config::DaemonConfig::for_tests();
    config.daemon_id = "target-kernel".to_string();
    let mut app = crate::DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            target.to_string_lossy(),
            target.to_string_lossy(),
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "target-terminal-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("terminal attachment should attach");
    let target_session_id = session.id().to_string();
    let target_attachment_id = attachment.id().to_string();
    let machine_id = app.config_projection_store().snapshot().host_machine_id;
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let (_, link) = runtime
        .create_workspace_link(
            &target_session_id,
            "shared-remote".to_string(),
            "target-user".to_string(),
        )
        .expect("workspace link should be created");
    runtime
        .attach_workspace_link(
            &target_session_id,
            link.link_id(),
            "target-user".to_string(),
            machine_id,
            "target-kernel".to_string(),
            target.to_string_lossy().to_string(),
            crate::git_observer::workspace_live_sync_git_branch(&target),
            crate::git_observer::workspace_live_sync_repo_fingerprint(&target),
        )
        .expect("target worktree should attach");

    let change = workspace_live_sync_text_change(
        "home-session",
        "source-agent",
        "source-provider-run",
        "source-prompt",
        std::path::Path::new("/tmp/source-worktree"),
        "seed\n",
        "seed\nfrom source\n",
    );
    let result = runtime.apply_forwarded_workspace_live_sync_change(
        crate::transport::relay_peer::RemoteWorkspaceLiveSyncApplyContext {
            home_session_id: "home-session".to_string(),
            link_id: link.link_id().to_string(),
            link_name: link.name().to_string(),
            source_agent_id: "source-agent".to_string(),
            source_worktree_path: "/tmp/source-worktree".to_string(),
            target_user_id: "target-user".to_string(),
            target_machine_id: "target-machine".to_string(),
            target_kernel_id: "target-kernel".to_string(),
            target_repo_root: target.to_string_lossy().to_string(),
        },
        change,
    );

    assert_eq!(
        result.path_results[0].status,
        crate::git_observer::WorkspaceLiveSyncApplyStatus::Applied
    );
    assert_eq!(
        std::fs::read_to_string(target.join("src/lib.rs")).expect("target file should be readable"),
        "seed\nfrom source\n"
    );
    let notices = runtime
        .drain_notice_records(&target_session_id, &target_attachment_id)
        .await;
    assert!(
        notices.iter().any(|notice| {
            notice.session_id == target_session_id
                && notice.provider_run_id.is_none()
                && notice
                    .message
                    .contains("Workspace live sync tracked turn summary")
                && notice.message.contains("source agent `source-agent`")
                && notice.message.contains("target user `target-user`")
                && notice.message.contains("Next action: none")
        }),
        "forwarded workspace live sync apply should notify target session: {notices:?}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn pending_git_snapshot_finalizes_completed_turn_projection_after_provider_completion() {
    let root = std::env::temp_dir().join(format!(
        "arroba-git-turn-finalizer-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    init_repo_with_file(&root, "src/lib.rs", "seed\n");

    let mut app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            root.to_string_lossy(),
            root.to_string_lossy(),
        ))
        .expect("session should be created");
    let provider_run = app
        .providers
        .start_run_provider_only(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id())
            .with_working_directory(root.clone()),
        )
        .expect("provider should start")
        .into_run();
    app.update_provider_run_projection(provider_run.clone());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let before = crate::git_observer::capture_turn_snapshot(crate::git_observer::GitTurnContext {
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        provider: provider_run.provider().to_string(),
        model: provider_run.model().to_string(),
        provider_run_id: provider_run.id().to_string(),
        provider_session_id: None,
        prompt_id: "prompt-1".to_string(),
        turn_id: "prompt-1".to_string(),
        source_attachment_id: Some("attachment-1".to_string()),
        prompt_origin: Some(crate::session::PromptOrigin::Arroba),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        started_at_ms: Some(crate::session::unix_epoch_ms()),
        worktree_path: root.clone(),
        workspace_live_sync_tracked: true,
        machine_id: None,
        prompt_summary: "edit src/lib.rs".to_string(),
    })
    .expect("pre-turn snapshot should be captured");
    runtime.owned.git_turn_snapshots.insert(before);

    std::fs::write(root.join("src/lib.rs"), "seed\nagent change\n").expect("source should change");

    runtime
        .observe_git_after_provider_activity_if_pending(provider_run.id())
        .await;

    assert_eq!(
        runtime
            .owned
            .git_turn_snapshots
            .get_for_provider_run(provider_run.id()),
        None,
        "successful finalization should consume the pending snapshot"
    );
    let projection = runtime
        .owned
        .completed_git_turn_snapshots
        .latest_projection_for_agent(session.id(), agent.id())
        .expect("completed turn projection should be recorded");
    assert_eq!(projection.agent_id, agent.id());
    assert_eq!(projection.provider_run_id, provider_run.id());
    assert_eq!(projection.prompt_id, "prompt-1");
    assert!(projection.undo_available);
    assert_eq!(projection.changed_paths, vec!["src/lib.rs".to_string()]);

    let activity = runtime.agent_activity_for_session(
        &runtime
            .owned
            .session_snapshot(session.id())
            .expect("session snapshot should exist"),
    );
    assert_eq!(
        activity
            .get(agent.id())
            .and_then(|agent| agent.last_completed_turn.as_ref())
            .map(|turn| turn.turn_id.as_str()),
        Some("prompt-1"),
        "session activity should expose the completed turn"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn pending_git_snapshot_waits_for_prompt_owner_when_session_mirror_is_stale() {
    let root = std::env::temp_dir().join(format!(
        "arroba-git-turn-owner-active-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    init_repo_with_file(&root, "src/lib.rs", "seed\n");

    let mut app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            root.to_string_lossy(),
            root.to_string_lossy(),
        ))
        .expect("session should be created");
    let provider_run = app
        .providers
        .start_run_provider_only(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id())
            .with_working_directory(root.clone()),
        )
        .expect("provider should start")
        .into_run();
    app.update_provider_run_projection(provider_run.clone());
    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "codex-thread-git-owner-active",
        "codex-turn-git-owner-active",
        agent.id(),
        "external prompt still running",
    );
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(external_prompt))
        .expect("external active prompt should sync");
    app.sessions_mut()
        .mirror_agent_prompt_state(
            session.id(),
            agent.id(),
            None,
            std::collections::VecDeque::new(),
        )
        .expect("test drift should clear stale session prompt mirror");
    assert!(
        app.sessions()
            .get_session(session.id())
            .expect("session should load")
            .active_prompt_for_agent(agent.id())
            .is_none(),
        "session mirror should not expose the active prompt"
    );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let before = crate::git_observer::capture_turn_snapshot(crate::git_observer::GitTurnContext {
        session_id: session.id().to_string(),
        agent_id: agent.id().to_string(),
        provider: provider_run.provider().to_string(),
        model: provider_run.model().to_string(),
        provider_run_id: provider_run.id().to_string(),
        provider_session_id: None,
        prompt_id: "prompt-owner-active".to_string(),
        turn_id: "prompt-owner-active".to_string(),
        source_attachment_id: Some("external:codex".to_string()),
        prompt_origin: Some(crate::session::PromptOrigin::External),
        external_provider: Some("codex".to_string()),
        external_provider_session_id: Some("codex-thread-git-owner-active".to_string()),
        external_provider_turn_id: Some("codex-turn-git-owner-active".to_string()),
        started_at_ms: Some(crate::session::unix_epoch_ms()),
        worktree_path: root.clone(),
        workspace_live_sync_tracked: true,
        machine_id: None,
        prompt_summary: "external prompt still running".to_string(),
    })
    .expect("pre-turn snapshot should be captured");
    runtime.owned.git_turn_snapshots.insert(before);
    std::fs::write(root.join("src/lib.rs"), "seed\nagent change\n").expect("source should change");

    runtime
        .observe_git_after_provider_activity_if_pending(provider_run.id())
        .await;

    assert!(
        runtime
            .owned
            .git_turn_snapshots
            .get_for_provider_run(provider_run.id())
            .is_some(),
        "pending git snapshot must not finalize while prompt owner still has an active prompt"
    );

    let _ = std::fs::remove_dir_all(root);
}

fn workspace_live_sync_text_change(
    session_id: &str,
    agent_id: &str,
    provider_run_id: &str,
    prompt_id: &str,
    worktree: &std::path::Path,
    before: &str,
    after: &str,
) -> crate::git_observer::WorkspaceLiveSyncChange {
    use base64::Engine as _;

    crate::git_observer::WorkspaceLiveSyncChange {
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        provider_run_id: provider_run_id.to_string(),
        prompt_id: prompt_id.to_string(),
        repo_root: worktree.to_string_lossy().to_string(),
        worktree_path: worktree.to_string_lossy().to_string(),
        branch: crate::git_observer::workspace_live_sync_git_branch(worktree),
        changed_paths: vec!["src/lib.rs".to_string()],
        file_changes: vec![crate::git_observer::WorkspaceLiveSyncFileChange {
            path: "src/lib.rs".to_string(),
            previous_path: None,
            kind: crate::git_observer::WorkspaceLiveSyncFileChangeKind::Modified,
            before_content_base64: Some(base64::engine::general_purpose::STANDARD.encode(before)),
            after_content_base64: Some(base64::engine::general_purpose::STANDARD.encode(after)),
            binary: false,
        }],
        status_fingerprint: "tracked_workspace_live_sync".to_string(),
    }
}

fn change(status_fingerprint: &str) -> crate::git_observer::WorkspaceLiveSyncChange {
    crate::git_observer::WorkspaceLiveSyncChange {
        session_id: "session-1".to_string(),
        agent_id: "agent-1".to_string(),
        provider_run_id: "run-1".to_string(),
        prompt_id: "prompt-1".to_string(),
        repo_root: "/tmp/source".to_string(),
        worktree_path: "/tmp/source".to_string(),
        branch: Some("main".to_string()),
        changed_paths: vec!["src/lib.rs".to_string(), "src/main.rs".to_string()],
        file_changes: Vec::new(),
        status_fingerprint: status_fingerprint.to_string(),
    }
}

fn target_result(
    path_results: Vec<crate::git_observer::WorkspaceLiveSyncPathApplyResult>,
) -> crate::git_observer::WorkspaceLiveSyncTargetResult {
    crate::git_observer::WorkspaceLiveSyncTargetResult {
        session_id: "session-1".to_string(),
        link_id: "link-1".to_string(),
        link_name: "pair".to_string(),
        source_agent_id: "agent-1".to_string(),
        source_worktree_path: "/tmp/source".to_string(),
        target_user_id: "user-2".to_string(),
        target_machine_id: "machine-2".to_string(),
        target_kernel_id: "kernel-2".to_string(),
        target_repo_root: "/tmp/target".to_string(),
        path_results,
    }
}

fn attachment(repo_root: &str, kernel_id: &str) -> crate::session::WorkspaceLinkAttachment {
    crate::session::WorkspaceLinkAttachment::new(
        "link-1",
        "user-1",
        "machine-1",
        kernel_id,
        repo_root,
        Some("main".to_string()),
        Some("repo-fingerprint".to_string()),
    )
}

fn path_result(
    path: &str,
    status: crate::git_observer::WorkspaceLiveSyncApplyStatus,
    message: &str,
) -> crate::git_observer::WorkspaceLiveSyncPathApplyResult {
    crate::git_observer::WorkspaceLiveSyncPathApplyResult {
        path: path.to_string(),
        status,
        message: message.to_string(),
    }
}

fn init_repo_with_file(root: &std::path::Path, relative_path: &str, content: &str) {
    std::fs::create_dir_all(root.join(std::path::Path::new(relative_path).parent().unwrap()))
        .expect("fixture directory should be created");
    run_git(root, &["init", "-b", "main"]);
    run_git(
        root,
        &["config", "user.email", "workspace-live-sync@example.com"],
    );
    run_git(root, &["config", "user.name", "Workspace Live Sync"]);
    std::fs::write(root.join(relative_path), content).expect("fixture file should be written");
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "seed"]);
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    std::fs::create_dir_all(cwd).expect("git cwd should exist");
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
