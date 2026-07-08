use super::*;

#[test]
fn append_observed_external_turns_consumes_arroba_owned_prompt_matches_once() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    app.append_history_entry(
        session.id(),
        SessionHistoryEntry::user_prompt(
            session.id(),
            "attachment-1",
            agent.id(),
            "repeatable prompt",
        ),
    );
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
                    provider_turn_id: Some("user-owned".to_string()),
                    role: ObservedExternalProviderTurnRole::User,
                    text: "repeatable prompt".to_string(),
                    observed_at_ms: Some(42),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-owned".to_string()),
                    role: ObservedExternalProviderTurnRole::Assistant,
                    text: "provider reply to arroba owned prompt".to_string(),
                    observed_at_ms: Some(84),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-external".to_string()),
                    role: ObservedExternalProviderTurnRole::User,
                    text: "repeatable prompt".to_string(),
                    observed_at_ms: Some(126),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-external".to_string()),
                    role: ObservedExternalProviderTurnRole::Assistant,
                    text: "provider reply to external repeated prompt".to_string(),
                    observed_at_ms: Some(168),
                },
            ],
        },
    )
    .expect("later repeated prompt should be observed as external");

    assert_eq!(outcome.changed_count, 2);
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].text, "repeatable prompt");
    assert_eq!(entries[1].text, "repeatable prompt");
    assert!(entries[1].is_external_provider_observed());
    assert_eq!(
        entries[1].merge_key.as_deref(),
        Some("external:codex:thread-observed:user-external")
    );
    assert_eq!(
        entries[2].text,
        "provider reply to external repeated prompt"
    );
    assert_eq!(
        entries[2].external_provider_turn_id.as_deref(),
        Some("user-external")
    );
    let active_prompt = app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("latest repeated external prompt should be active");
    assert_eq!(active_prompt.prompt_origin(), PromptOrigin::External);
    assert_eq!(active_prompt.prompt(), "repeatable prompt");
}

#[test]
fn append_observed_external_turns_skips_active_arroba_prompt_echoes() {
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
    app.prompt_owner_activate_prompt(
        session.id(),
        PromptQueueItem::new(
            "arroba-active-prompt",
            "attachment-1",
            agent.id(),
            "arroba-owned active prompt",
            PromptStatus::Running,
        ),
    )
    .expect("active Arroba prompt should mirror");

    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: attached_external_observer_target_from_import(
                session.id().to_string(),
                agent.id().to_string(),
                None,
                import,
            ),
            turns: vec![
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-owned-active".to_string()),
                    role: ObservedExternalProviderTurnRole::User,
                    text: "arroba-owned active prompt".to_string(),
                    observed_at_ms: Some(42),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("tool-owned-active".to_string()),
                    role: ObservedExternalProviderTurnRole::Tool,
                    text: "{\"tool\":\"bash\",\"status\":\"completed\"}".to_string(),
                    observed_at_ms: Some(84),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-owned-active".to_string()),
                    role: ObservedExternalProviderTurnRole::Assistant,
                    text: "provider reply to arroba-owned active prompt".to_string(),
                    observed_at_ms: Some(126),
                },
            ],
        },
    )
    .expect("observed Arroba-owned active provider turn should be skipped");

    assert_eq!(outcome.changed_count, 0);
    assert!(!outcome.external_active_prompt_settled);
    assert!(
        app.load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load")
            .is_empty()
    );
    let active_prompt = app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("Arroba active prompt should remain active");
    assert_eq!(active_prompt.prompt_origin(), PromptOrigin::Arroba);
    assert_eq!(active_prompt.prompt(), "arroba-owned active prompt");
}

#[test]
fn append_observed_external_turns_replaces_changed_duplicate_merge_key() {
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
        import.clone(),
    );
    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("assistant-1".to_string()),
                role: ObservedExternalProviderTurnRole::Assistant,
                text: "partial external reply".to_string(),
                observed_at_ms: Some(42),
            }],
        },
    )
    .expect("initial observed assistant turn should append");

    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("assistant-1".to_string()),
                role: ObservedExternalProviderTurnRole::Assistant,
                text: "complete external reply".to_string(),
                observed_at_ms: Some(84),
            }],
        },
    )
    .expect("changed observed assistant duplicate should replace prior content");

    assert_eq!(outcome.changed_count, 1);
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text, "complete external reply");
    assert_eq!(entries[0].observed_at_ms, Some(84));
    assert!(entries[0].is_external_provider_observed());
    assert_eq!(
        entries[0].merge_key.as_deref(),
        Some("external:codex:thread-observed:assistant-1")
    );
    assert_eq!(entries[0].external_provider.as_deref(), Some("codex"));
    assert_eq!(
        entries[0].external_provider_session_id.as_deref(),
        Some("thread-observed")
    );
    assert_eq!(
        entries[0].external_provider_turn_id.as_deref(),
        Some("assistant-1")
    );
}

#[test]
fn append_observed_external_turns_uses_latest_duplicate_merge_key_per_poll() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "claude:thread-observed".to_string(),
        "claude".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let target = attached_external_observer_target_from_import(
        session.id().to_string(),
        agent.id().to_string(),
        None,
        import.clone(),
    );
    let turns = vec![
        ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1".to_string()),
            role: ObservedExternalProviderTurnRole::Assistant,
            text: "partial external reply".to_string(),
            observed_at_ms: Some(42),
        },
        ObservedExternalProviderTurn {
            provider_turn_id: Some("assistant-1".to_string()),
            role: ObservedExternalProviderTurnRole::Assistant,
            text: "complete external reply".to_string(),
            observed_at_ms: Some(84),
        },
    ];

    let initial = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: turns.clone(),
        },
    )
    .expect("latest duplicate observed assistant turn should append");

    assert_eq!(initial.changed_count, 1);
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text, "complete external reply");
    assert_eq!(entries[0].observed_at_ms, Some(84));

    let stable = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead { target, turns },
    )
    .expect("same duplicate snapshot should not churn history");

    assert_eq!(stable.changed_count, 0);
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text, "complete external reply");
}

#[test]
fn append_observed_external_turns_ignores_provider_run_id_only_changes() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "claude:thread-observed".to_string(),
        "claude".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let turn = ObservedExternalProviderTurn {
        provider_turn_id: Some("assistant-1".to_string()),
        role: ObservedExternalProviderTurnRole::Assistant,
        text: "complete external reply".to_string(),
        observed_at_ms: Some(84),
    };

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: attached_external_observer_target_from_import(
                session.id().to_string(),
                agent.id().to_string(),
                None,
                import.clone(),
            ),
            turns: vec![turn.clone()],
        },
    )
    .expect("initial observed assistant turn should append");

    let stable = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: attached_external_observer_target_from_import(
                session.id().to_string(),
                agent.id().to_string(),
                Some("provider-run-2".to_string()),
                import,
            ),
            turns: vec![turn],
        },
    )
    .expect("provider-run-only change should not churn external history");

    assert_eq!(stable.changed_count, 0);
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].provider_run_id, None);
    assert_eq!(entries[0].text, "complete external reply");
}

#[test]
fn append_observed_external_turns_signals_attached_terminals_to_refresh_history() {
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
    let run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "default")
                .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    let import = ExternalProviderImportMetadata::observed_history(
        "opencode:thread-observed".to_string(),
        "opencode".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: attached_external_observer_target_from_import(
                session.id().to_string(),
                agent.id().to_string(),
                Some(run.id().to_string()),
                import,
            ),
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("item-1".to_string()),
                role: ObservedExternalProviderTurnRole::Assistant,
                text: "observed reply".to_string(),
                observed_at_ms: Some(42),
            }],
        },
    )
    .expect("observed turn should append");

    assert_eq!(outcome.changed_count, 1);
    let records = app
        .terminal_mut()
        .drain_output_records(session.id(), attachment.id());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider_run_id, run.id());
    assert_eq!(records[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(
        records[0].kind,
        crate::terminal::TerminalOutputKind::ProviderStatus
    );
    assert_eq!(
        records[0].merge_key.as_deref(),
        Some("external:opencode:thread-observed:item-1")
    );
}

#[test]
fn append_observed_external_turns_signals_history_refresh_without_provider_run() {
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
        "codex:thread-observed".to_string(),
        "codex".to_string(),
        "thread-observed".to_string(),
    );
    let agent =
        persist_external_import_metadata(&mut app, session.id(), agent.id(), import.clone())
            .expect("metadata should persist");
    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: attached_external_observer_target_from_import(
                session.id().to_string(),
                agent.id().to_string(),
                None,
                import,
            ),
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("item-1".to_string()),
                role: ObservedExternalProviderTurnRole::Assistant,
                text: "observed reply".to_string(),
                observed_at_ms: Some(42),
            }],
        },
    )
    .expect("observed turn should append");

    assert_eq!(outcome.changed_count, 1);
    let records = app
        .terminal_mut()
        .drain_output_records(session.id(), attachment.id());
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].provider_run_id,
        format!("external-observer:{}", agent.id())
    );
    assert_eq!(records[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(
        records[0].kind,
        crate::terminal::TerminalOutputKind::ProviderStatus
    );
    assert_eq!(
        records[0].bytes,
        EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS.as_bytes()
    );
    let metadata = records[0]
        .external_observation_metadata
        .as_ref()
        .expect("external history refresh should carry observed metadata");
    assert_eq!(
        metadata.source,
        SessionHistoryEntrySource::ExternalProviderObserved
    );
    assert_eq!(metadata.external_provider.as_deref(), Some("codex"));
    assert_eq!(
        metadata.external_provider_session_id.as_deref(),
        Some("thread-observed")
    );
    assert_eq!(
        metadata.external_provider_turn_id.as_deref(),
        Some("item-1")
    );
    assert_eq!(metadata.observed_at_ms, Some(42));
}

#[test]
fn append_observed_external_completion_signals_settled_state_refresh() {
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
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-1".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        observed_at_ms: Some(42),
    };
    let completion = ObservedExternalProviderTurn {
        provider_turn_id: Some("task-complete-1".to_string()),
        role: ObservedExternalProviderTurnRole::Status,
        text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
        observed_at_ms: Some(84),
    };

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone()],
        },
    )
    .expect("prompt should append");
    let _ = app
        .terminal_mut()
        .drain_output_records(session.id(), attachment.id());

    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt, completion],
        },
    )
    .expect("completion should append and settle");

    assert!(outcome.external_active_prompt_settled);
    let records = app
        .terminal_mut()
        .drain_output_records(session.id(), attachment.id());
    assert_eq!(
        records.len(),
        2,
        "completion must signal both the new history row and the settled state projection"
    );
    assert_eq!(
        records[0].bytes,
        EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS.as_bytes()
    );
    assert_eq!(
        records[1].bytes,
        EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS.as_bytes()
    );
    assert_eq!(
        records[0]
            .external_observation_metadata
            .as_ref()
            .map(|metadata| metadata.source),
        Some(SessionHistoryEntrySource::ExternalProviderObserved)
    );
    assert_eq!(
        records[1].merge_key.as_deref(),
        Some(
            "external:codex:thread-observed:state:active_prompt_settled:external:codex:thread-observed:task-complete-1"
        )
    );
    let state_metadata = records[1]
        .external_observation_metadata
        .as_ref()
        .expect("state refresh should carry observed metadata");
    assert_eq!(
        state_metadata.source,
        SessionHistoryEntrySource::ExternalProviderObserved
    );
    assert_eq!(state_metadata.external_provider.as_deref(), Some("codex"));
    assert_eq!(
        state_metadata.external_provider_session_id.as_deref(),
        Some("thread-observed")
    );
    assert_eq!(
        state_metadata.external_provider_turn_id.as_deref(),
        Some("user-1")
    );
    assert_eq!(state_metadata.observed_at_ms, Some(84));
    assert_eq!(
        state_metadata
            .external_observation
            .as_ref()
            .map(|observation| observation.settles_active_prompt),
        Some(true)
    );
}

#[test]
fn resume_state_maps_known_external_providers() {
    assert_eq!(
        ProviderResumeState::from_external_provider_session("codex", "thread-1").codex_thread_id(),
        Some("thread-1")
    );
    assert_eq!(
        ProviderResumeState::from_external_provider_session("opencode", "session-1")
            .opencode_session_id(),
        Some("session-1")
    );
    assert_eq!(
        ProviderResumeState::from_external_provider_session("claude", "session-2")
            .claude_session_id(),
        Some("session-2")
    );
    assert!(
        ProviderResumeState::from_external_provider_session("dev-stub", "session-3").is_empty()
    );
}
