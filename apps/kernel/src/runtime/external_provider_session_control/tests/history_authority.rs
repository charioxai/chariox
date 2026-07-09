use super::*;

#[test]
fn observed_external_history_appends_without_creating_active_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "codex:thread-observed".to_string(),
        "codex".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );

    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-1".to_string()),
                    role: ObservedExternalProviderTurnRole::User,
                    text: "external prompt".to_string(),
                    observed_at_ms: Some(42),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-1".to_string()),
                    role: ObservedExternalProviderTurnRole::Assistant,
                    text: "external response".to_string(),
                    observed_at_ms: Some(84),
                },
            ],
        },
    )
    .expect("observed history should append");

    assert_eq!(outcome.changed_count, 2);
    assert!(app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("active prompt should load")
        .is_none());
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(session.id(), agent.id())
            .expect("queued count should load"),
        0
    );
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| {
        entry.external_provider.as_deref() == Some("codex")
            && entry.external_provider_session_id.as_deref() == Some("thread-observed")
    }));
}

#[test]
fn observed_external_history_batch_emits_one_refresh_after_all_entries_persist() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let import = ExternalProviderImportMetadata::observed_history(
        "opencode:thread-observed".to_string(),
        "opencode".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );

    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("opencode-user-2".to_string()),
                    role: ObservedExternalProviderTurnRole::User,
                    text: "external prompt".to_string(),
                    observed_at_ms: Some(100),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("opencode-reasoning-2".to_string()),
                    role: ObservedExternalProviderTurnRole::Reasoning,
                    text: "external reasoning after prompt".to_string(),
                    observed_at_ms: Some(101),
                },
            ],
        },
    )
    .expect("observed history should append");

    assert_eq!(outcome.changed_count, 2);
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert!(entries
        .iter()
        .any(|entry| entry.text == "external reasoning after prompt"));

    let records = app
        .terminal_stream_store()
        .drain_output_records(session.id(), attachment.id());
    assert_eq!(
        records.len(),
        1,
        "multi-entry external imports should trigger one history recovery after the batch"
    );
    assert_eq!(
        records[0].kind,
        crate::terminal::TerminalOutputKind::ProviderStatus
    );
    assert_eq!(
        String::from_utf8_lossy(&records[0].bytes),
        EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS
    );
    assert_eq!(
        records[0]
            .external_observation_metadata
            .as_ref()
            .and_then(|metadata| metadata.external_provider_turn_id.as_deref()),
        Some("opencode-user-2")
    );
}

#[test]
fn observed_live_external_user_turn_starts_active_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "opencode:thread-live".to_string(),
        "opencode".to_string(),
        "thread-live".to_string(),
    )
    .with_cursor(ExternalProviderObservedCursor::new(
        Some("assistant-1".to_string()),
        Some(84),
        Some(crate::history::external_provider_observed_merge_key(
            "opencode",
            "thread-live",
            "assistant-1",
        )),
    ));
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("opencode-user-2".to_string()),
                role: ObservedExternalProviderTurnRole::User,
                text: "external live prompt".to_string(),
                observed_at_ms: Some(100),
            }],
        },
    )
    .expect("observed history should append");

    let active = app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("live external prompt should become active");
    assert_eq!(active.prompt(), "external live prompt");
    assert_eq!(
        active.prompt_origin(),
        crate::session::PromptOrigin::External
    );
    assert_eq!(active.external_provider(), Some("opencode"));
    assert_eq!(active.external_provider_session_id(), Some("thread-live"));
    assert_eq!(active.external_provider_turn_id(), Some("opencode-user-2"));
}

#[test]
fn observed_live_external_completion_clears_active_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "opencode:thread-live".to_string(),
        "opencode".to_string(),
        "thread-live".to_string(),
    )
    .with_cursor(ExternalProviderObservedCursor::new(
        Some("opencode-user-2".to_string()),
        Some(100),
        Some(crate::history::external_provider_observed_merge_key(
            "opencode",
            "thread-live",
            "opencode-user-2",
        )),
    ));
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let active_prompt = PromptQueueItem::external_observed_running(
        "opencode",
        "thread-live",
        "opencode-user-2",
        agent.id(),
        "external live prompt",
    );
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(active_prompt))
        .expect("external active prompt should sync");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("message-status-opencode-completion".to_string()),
                role: ObservedExternalProviderTurnRole::Status,
                text: "opencode message completed\n{\"finish\":\"stop\"}".to_string(),
                observed_at_ms: Some(120),
            }],
        },
    )
    .expect("observed completion should append");

    assert!(app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("active prompt should load")
        .is_none());
}

#[test]
fn observed_live_external_completion_advances_queued_arroba_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let import = ExternalProviderImportMetadata::observed_history(
        "codex:thread-live".to_string(),
        "codex".to_string(),
        "thread-live".to_string(),
    )
    .with_cursor(ExternalProviderObservedCursor::new(
        Some("codex-user-2".to_string()),
        Some(100),
        Some(crate::history::external_provider_observed_merge_key(
            "codex",
            "thread-live",
            "codex-user-2",
        )),
    ));
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let run = test_codex_run(session.id(), agent.id(), "run-live", "thread-live");
    app.providers_mut().insert_run_for_test(run.clone());
    let active_prompt = PromptQueueItem::external_observed_running(
        "codex",
        "thread-live",
        "codex-user-2",
        agent.id(),
        "external live prompt",
    );
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(active_prompt))
        .expect("external active prompt should sync");
    let queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued kernel prompt after external turn",
        PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("second prompt should queue behind external prompt")
    else {
        panic!("second prompt should queue");
    };
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        Some(run.id().to_string()),
        import,
    );

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("task-complete-1".to_string()),
                role: ObservedExternalProviderTurnRole::Status,
                text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
                observed_at_ms: Some(120),
            }],
        },
    )
    .expect("observed completion should append");

    let active = app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("queued prompt should advance after external completion");
    assert_eq!(active.prompt(), "queued kernel prompt after external turn");
    assert_eq!(active.prompt_origin(), crate::session::PromptOrigin::Arroba);
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(session.id(), agent.id())
            .expect("queued count should load"),
        0
    );
}

#[test]
fn observed_external_completion_does_not_dispatch_queued_arroba_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let active_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "kernel owned prompt",
        PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), active_prompt, false)
        .expect("prompt should start")
    else {
        panic!("first prompt should start");
    };
    let queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued kernel prompt",
        PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("second prompt should queue")
    else {
        panic!("second prompt should queue");
    };
    let import = ExternalProviderImportMetadata::observed_history(
        "codex:thread-observed".to_string(),
        "codex".to_string(),
        "thread-observed".to_string(),
    );
    persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
        .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import,
    );

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("task-complete-1".to_string()),
                role: ObservedExternalProviderTurnRole::Status,
                text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
                observed_at_ms: Some(84),
            }],
        },
    )
    .expect("observed completion should append");

    let active = app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("kernel-owned active prompt must remain active");
    assert_eq!(active.prompt(), "kernel owned prompt");
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(session.id(), agent.id())
            .expect("queued count should load"),
        1
    );
}
