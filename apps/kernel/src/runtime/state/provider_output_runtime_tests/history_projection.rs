use super::*;

#[tokio::test]
async fn started_prompt_history_records_prompt_activation_timestamp() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-prompt-timestamp",
            "worktree-prompt-timestamp",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-prompt-timestamp",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let prompt = crate::session::PromptQueueItem::new(
        "prompt-accepted-before-dispatch",
        attachment.id(),
        agent.id(),
        "preserve my acceptance time",
        crate::session::PromptStatus::Running,
    );
    let accepted_at_ms = prompt.created_at_ms();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let recorded_at_ms = runtime
        .owned
        .record_started_user_prompt(session.id(), attachment.id(), &prompt)
        .expect("started prompt should be recorded");

    assert!(recorded_at_ms > accepted_at_ms);
    let entries = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .expect("canonical prompt history should load");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].timestamp_ms, recorded_at_ms);
}

#[tokio::test]
async fn session_lookup_snapshots_project_runtime_view_from_owned_state() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            crate::session::CreateSessionRequest::new(
                "workspace-session-lookup-projection",
                "worktree-session-lookup-projection",
            )
            .with_alias("lookup-projection"),
        )
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.sessions
        .set_active_provider_run(session.id(), None)
        .expect("test should clear stale stored active run");
    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "thread-session-lookup-projection",
        "turn-session-lookup-projection",
        agent.id(),
        "external prompt from prompt owner",
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

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let waiting_room_sequence = runtime.waiting_room_change_sequence();
    let listed = runtime.list_session_snapshots();
    assert_eq!(
        runtime.waiting_room_change_sequence(),
        waiting_room_sequence
    );
    let listed_session = listed
        .iter()
        .find(|listed| listed.id() == session.id())
        .expect("listed sessions should include the test session");
    assert_eq!(listed_session.agents().len(), 1);
    assert_eq!(listed_session.active_provider_run_id(), Some(run.id()));
    let listed_active_prompt = listed_session
        .active_prompt_for_agent(agent.id())
        .expect("listed session should project prompt owner active prompt");
    assert_eq!(
        listed_active_prompt.prompt_origin(),
        crate::session::PromptOrigin::External
    );
    assert_eq!(
        listed_active_prompt.prompt(),
        "external prompt from prompt owner"
    );

    let resolved = runtime
        .resolve_session_snapshot(crate::local::ResolveSessionRequest {
            session_ref: "lookup-projection".to_string(),
            workspace_id: Some("workspace-session-lookup-projection".to_string()),
        })
        .expect("session should resolve by alias");
    assert_eq!(resolved.id(), session.id());
    assert_eq!(resolved.agents().len(), 1);
    assert_eq!(resolved.active_provider_run_id(), Some(run.id()));
    let resolved_active_prompt = resolved
        .active_prompt_for_agent(agent.id())
        .expect("resolved session should project prompt owner active prompt");
    assert_eq!(
        resolved_active_prompt.prompt_origin(),
        crate::session::PromptOrigin::External
    );
    assert_eq!(
        resolved_active_prompt.prompt(),
        "external prompt from prompt owner"
    );
}

#[tokio::test]
async fn unchanged_session_lookup_does_not_wake_waiting_room_subscribers() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-idle-session-lookup",
            "worktree-idle-session-lookup",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    runtime
        .session_snapshot(&session_id)
        .await
        .expect("first lookup should warm the projection");
    let waiting_room_sequence = runtime.waiting_room_change_sequence();

    runtime
        .session_snapshot(&session_id)
        .await
        .expect("unchanged lookup should succeed");

    assert_eq!(
        runtime.waiting_room_change_sequence(),
        waiting_room_sequence,
        "read-only session lookup must not wake every waiting-room subscriber"
    );
}

#[tokio::test]
async fn owned_user_prompt_history_enqueues_archive_outbox_when_external_archive_enabled() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.user_config.history.archive.mode = crate::config::HistoryArchiveMode::External;
    config.user_config.history.archive.url = Some("http://127.0.0.1:9".to_string());
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-owned-archive",
            "worktree-owned-archive",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-owned-archive",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .append_user_prompt_history(
            session.id(),
            attachment.id(),
            agent.id(),
            "owned archive prompt",
            &[],
            crate::session::PromptOrigin::Arroba,
            Some("prompt-owned-archive"),
            None,
            None,
            None,
        )
        .expect("owned prompt history should append");

    let pending = runtime
        .owned
        .operational_history_store
        .load_pending_archive_events(10)
        .expect("pending archive events should load");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].event.session_id.as_deref(), Some(session.id()));
    assert_eq!(pending[0].event.agent_id.as_deref(), Some(agent.id()));
    assert_eq!(
        pending[0].event.content.as_deref().map(str::trim_end),
        Some("owned archive prompt")
    );
}

#[tokio::test]
async fn owned_user_prompt_history_persists_operational_when_legacy_append_fails() {
    let config = crate::config::DaemonConfig::for_tests();
    let legacy_history_root = config.session_history_root.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-owned-legacy-history-fail",
            "worktree-owned-legacy-history-fail",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-owned-legacy-history-fail",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let _ = fs::remove_dir_all(&legacy_history_root);
    fs::write(&legacy_history_root, b"not a directory")
        .expect("fixture should block legacy history writes");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .append_user_prompt_history(
            session.id(),
            attachment.id(),
            agent.id(),
            "owned reload me",
            &[],
            crate::session::PromptOrigin::Arroba,
            Some("prompt-owned-legacy-history-fail"),
            None,
            None,
            None,
        )
        .expect("owned prompt history should append");

    let entries = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .expect("canonical operational history should load");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text.trim_end(), "owned reload me");

    let _ = fs::remove_file(&legacy_history_root);
}

#[tokio::test]
async fn owned_user_prompt_history_preserves_external_prompt_origin() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-external-user-history",
            "worktree-external-user-history",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-external-user-history",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .append_user_prompt_history(
            session.id(),
            attachment.id(),
            agent.id(),
            "external reload me",
            &[],
            crate::session::PromptOrigin::External,
            Some("prompt-external-user-history"),
            None,
            None,
            None,
        )
        .expect("external prompt history should append");

    let entries = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .expect("canonical operational history should load");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text.trim_end(), "external reload me");
    assert_eq!(
        entries[0].prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
}

#[tokio::test]
async fn owned_external_observed_history_uses_provider_turn_id_for_operational_turn() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-owned-external-turn-id",
            "worktree-owned-external-turn-id",
        ))
        .expect("session should be created");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime.owned.append_history_entry(
        session.id(),
        crate::history::SessionHistoryEntry::external_provider_observed(
            session.id(),
            None,
            agent.id(),
            crate::history::SessionHistoryEntryKind::ProviderOutput,
            "external observed output",
            "codex",
            "thread-owned-external-turn-id",
            Some("provider-turn-owned-external".to_string()),
            Some(1_234),
        ),
    );

    let events = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .expect("canonical operational history should load");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].turn_id.as_deref(),
        Some("provider-turn-owned-external")
    );
    let reloaded_entry = events[0]
        .to_session_history_entry()
        .expect("history event should project back to session history");
    assert_eq!(
        reloaded_entry.prompt_origin,
        Some(crate::session::PromptOrigin::External)
    );
}

#[tokio::test]
async fn owned_runtime_notice_persists_operational_when_legacy_append_fails() {
    let config = crate::config::DaemonConfig::for_tests();
    let legacy_history_root = config.session_history_root.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-owned-notice-legacy-history-fail",
            "worktree-owned-notice-legacy-history-fail",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");

    let _ = fs::remove_dir_all(&legacy_history_root);
    fs::write(&legacy_history_root, b"not a directory")
        .expect("fixture should block legacy history writes");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime.owned.record_notice(
        session.id(),
        Some(run.id()),
        Vec::new(),
        "owned notice reload me",
    );

    let entries = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .expect("canonical operational history should load");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].kind,
        crate::history::SessionHistoryEntryKind::Notice
    );
    assert_eq!(entries[0].text.trim_end(), "owned notice reload me");

    let _ = fs::remove_file(&legacy_history_root);
}

#[tokio::test]
async fn rejected_owned_local_prompt_does_not_persist_history() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-owned-queue-overflow",
            "worktree-owned-queue-overflow",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-owned-queue-overflow",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    app.launch_provider(
        crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "default",
        )
        .with_agent_id(agent.id()),
    )
    .expect("provider run should launch");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let submit = |prompt_id: String, prompt: String| {
        runtime
            .owned
            .submit_local_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session.id().to_string(),
                prompt: crate::session::PromptQueueItem::new(
                    prompt_id,
                    attachment.id(),
                    agent.id(),
                    prompt,
                    crate::session::PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
    };

    submit(
        "prompt-overflow-active".to_string(),
        "active accepted prompt".to_string(),
    )
    .expect("active prompt should submit")
    .expect("local prompt should be handled");
    for index in 0..crate::runtime::prompt_state::PROMPT_QUEUE_LIMIT {
        submit(
            format!("prompt-overflow-queued-{index}"),
            format!("queued accepted prompt {index}"),
        )
        .expect("queued prompt should submit while under queue limit")
        .expect("local prompt should be handled");
    }

    let error = match submit(
        "prompt-overflow-rejected".to_string(),
        "rejected prompt must not be history".to_string(),
    ) {
        Ok(_) => panic!("queue overflow should reject prompt before history append"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("agent prompt queue overloaded"));

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session should snapshot");
    assert_eq!(
        session_state
            .queued_prompts_for_agent(agent.id())
            .map(|queued| queued.len()),
        Some(crate::runtime::prompt_state::PROMPT_QUEUE_LIMIT)
    );
    let history_entries = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .expect("history should load");
    assert!(
        history_entries
            .iter()
            .all(|entry| entry.text != "rejected prompt must not be history"),
        "rejected prompt should not be visible after reload"
    );
    assert!(
        history_entries
            .iter()
            .all(|entry| !entry.text.starts_with("queued accepted prompt ")),
        "queued-only prompts should become history when they start, not when they wait in queue"
    );
    assert_eq!(
        history_entries
            .iter()
            .filter(|entry| entry.kind == crate::history::SessionHistoryEntryKind::UserPrompt)
            .count(),
        1
    );
}
