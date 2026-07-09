use super::*;

#[test]
fn append_observed_external_user_turn_for_arroba_owned_run_adds_history_and_active_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let run = test_codex_run(
        session.id(),
        agent.id(),
        "run-arroba-owned",
        "thread-arroba",
    );
    app.providers_mut().insert_run_for_test(run);
    let target = single_attached_target(&app);

    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("user-native".to_string()),
                role: ObservedExternalProviderTurnRole::User,
                text: "native prompt outside Arroba".to_string(),
                observed_at_ms: Some(42),
            }],
        },
    )
    .expect("observed external native user turn should append");

    assert_eq!(outcome.changed_count, 1);
    let active = app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("external active prompt should be set");
    assert_eq!(active.prompt_origin(), PromptOrigin::External);
    assert_eq!(active.prompt(), "native prompt outside Arroba");
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].is_external_provider_observed());
    assert_eq!(entries[0].text, "native prompt outside Arroba");
    assert!(app
        .agents()
        .get_agent(agent.id())
        .expect("agent should load")
        .external_provider_import()
        .is_none());
}

#[test]
fn append_observed_arroba_owned_prompt_echoes_are_skipped_without_import_metadata() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let run = test_codex_run(
        session.id(),
        agent.id(),
        "run-arroba-owned",
        "thread-arroba",
    );
    app.providers_mut().insert_run_for_test(run);
    app.append_history_entry(
        session.id(),
        SessionHistoryEntry::user_prompt(
            session.id(),
            "attachment-1",
            agent.id(),
            "arroba owned prompt",
        ),
    );
    let target = single_attached_target(&app);
    let cursor_key = match target.cursor_source.clone() {
        AttachedExternalObserverCursorSource::ArrobaOwned(key) => key,
        AttachedExternalObserverCursorSource::Imported(_) => {
            panic!("Arroba-owned target should not use import metadata")
        }
    };

    let outcome = append_observed_external_turns_for_attached_target(
            &mut app,
            AttachedExternalObserverRead {
                target,
                turns: vec![
                    ObservedExternalProviderTurn {
                        provider_turn_id: Some("user-owned".to_string()),
                        role: ObservedExternalProviderTurnRole::User,
                        text: "arroba owned prompt\n<image name=[Image #1] path=\"/tmp/screenshot.png\"> </image>".to_string(),
                        observed_at_ms: Some(42),
                    },
                    ObservedExternalProviderTurn {
                        provider_turn_id: Some("assistant-owned".to_string()),
                        role: ObservedExternalProviderTurnRole::Assistant,
                        text: "reply to Arroba owned prompt".to_string(),
                        observed_at_ms: Some(84),
                    },
                ],
            },
        )
        .expect("observed Arroba-owned prompt echo should be skipped");

    assert_eq!(outcome.changed_count, 0);
    assert!(app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("active prompt should load")
        .is_none());
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text, "arroba owned prompt");
    assert!(app
        .agents()
        .get_agent(agent.id())
        .expect("agent should load")
        .external_provider_import()
        .is_none());
    let cursor = app
        .attached_provider_transcript_cursor_store()
        .get(&cursor_key);
    assert_eq!(
        cursor.last_observed_turn_id.as_deref(),
        Some("assistant-owned")
    );
    assert!(cursor
        .arroba_owned_observed_prompt_turn_ids
        .contains("user-owned"));
}

#[test]
fn append_observed_arroba_owned_prompt_cursor_prevents_reimport_after_reload() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("app should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    let import = ExternalProviderImportMetadata::observed_history(
        "codex:thread-observed".to_string(),
        "codex".to_string(),
        "thread-observed".to_string(),
    )
    .with_cursor(crate::provider::ExternalProviderObservedCursor {
        last_observed_turn_id: Some("assistant-owned".to_string()),
        last_observed_at_ms: Some(84),
        last_observed_merge_key: Some("external:codex:thread-observed:assistant-owned".to_string()),
        arroba_owned_observed_prompt_turn_ids: std::collections::BTreeSet::from([
            "user-owned".to_string()
        ]),
    });
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
                    text: "previously classified Arroba prompt".to_string(),
                    observed_at_ms: Some(42),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-owned".to_string()),
                    role: ObservedExternalProviderTurnRole::Assistant,
                    text: "reply to previously classified Arroba prompt".to_string(),
                    observed_at_ms: Some(84),
                },
            ],
        },
    )
    .expect("cursor-classified Arroba-owned prompt should stay skipped");

    assert_eq!(outcome.changed_count, 0);
    assert!(app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("active prompt should load")
        .is_none());
    assert!(app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load")
        .is_empty());
}

#[test]
fn append_observed_arroba_owned_prompt_echo_uses_prompt_owner_queue_when_mirror_is_stale() {
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
    let run = test_codex_run(
        session.id(),
        agent.id(),
        "run-arroba-owned-stale-queue-mirror",
        "thread-arroba-stale-queue-mirror",
    );
    app.providers_mut().insert_run_for_test(run);
    let target = single_attached_target(&app);
    let queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued Arroba prompt",
        PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, true)
        .expect("forced queued prompt should queue in prompt owner")
    else {
        panic!("forced prompt should queue");
    };
    app.sessions_mut()
        .mirror_agent_prompt_state(
            session.id(),
            agent.id(),
            None,
            std::collections::VecDeque::new(),
        )
        .expect("test drift should clear the session prompt mirror");
    assert!(
        app.sessions()
            .get_session(session.id())
            .expect("session should load")
            .queued_prompts_for_agent(agent.id())
            .map(|queued| queued.is_empty())
            .unwrap_or(true),
        "session mirror should not expose the queued prompt"
    );
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(session.id(), agent.id())
            .expect("prompt owner should read"),
        1,
        "prompt owner remains authoritative for the queued prompt"
    );

    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-owned".to_string()),
                    role: ObservedExternalProviderTurnRole::User,
                    text: "queued Arroba prompt".to_string(),
                    observed_at_ms: Some(42),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-owned".to_string()),
                    role: ObservedExternalProviderTurnRole::Assistant,
                    text: "reply to queued Arroba prompt".to_string(),
                    observed_at_ms: Some(84),
                },
            ],
        },
    )
    .expect("observed Arroba-owned queued prompt echo should be skipped");

    assert_eq!(outcome.changed_count, 0);
    assert!(
        app.load_session_history_entries(&session, Some(agent.id()))
            .expect("history should load")
            .is_empty(),
        "Arroba-owned provider echo must not be imported as an external turn"
    );
}

#[tokio::test]
async fn append_observed_arroba_owned_completion_settles_and_advances_queued_prompt() {
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
    let run = test_codex_run(
        session.id(),
        agent.id(),
        "run-arroba-owned",
        "thread-arroba",
    );
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let target = single_attached_target(&app);
    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("user-native".to_string()),
                role: ObservedExternalProviderTurnRole::User,
                text: "native prompt outside Arroba".to_string(),
                observed_at_ms: Some(42),
            }],
        },
    )
    .expect("native user turn should mark active prompt");
    let queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued Arroba prompt",
        PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("Arroba prompt should queue behind external active prompt")
    else {
        panic!("Arroba prompt should not start while external active prompt is running");
    };
    let queued_prompt_id = prompt.id().to_string();
    assert!(queued_prompt_id.starts_with("pending-prompt-"));
    assert_eq!(prompt.pending_prompt_id(), Some(queued_prompt_id.as_str()));

    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-native".to_string()),
                    role: ObservedExternalProviderTurnRole::User,
                    text: "native prompt outside Arroba".to_string(),
                    observed_at_ms: Some(42),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("complete-native".to_string()),
                    role: ObservedExternalProviderTurnRole::Status,
                    text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
                    observed_at_ms: Some(84),
                },
            ],
        },
    )
    .expect("completion should settle active external prompt");
    assert!(outcome.external_active_prompt_settled);

    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app),
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    );
    let dispatched = router
        .runtime_state()
        .dispatch_next_queued_prompt_after_external_settlement(session.id(), agent.id(), run.id())
        .await
        .expect("queued prompt should dispatch after external settlement");
    assert!(dispatched);
    let session = router
        .runtime_state()
        .session_snapshot(session.id())
        .await
        .expect("session should snapshot");
    let active_prompt = session
        .active_prompt_for_agent(agent.id())
        .expect("promoted queued prompt should become active");
    assert_ne!(active_prompt.id(), queued_prompt_id);
    assert!(active_prompt.id().starts_with("prompt-"));
    assert_eq!(active_prompt.pending_prompt_id(), None);
    assert_eq!(active_prompt.prompt(), "queued Arroba prompt");
    assert!(session
        .queued_prompts_for_agent(agent.id())
        .map(|queued| queued.is_empty())
        .unwrap_or(true));
}

#[tokio::test]
async fn observed_external_completion_advances_queue_when_active_prompt_mirror_was_lost() {
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
    let run = test_codex_run(
        session.id(),
        agent.id(),
        "run-external-lost-mirror",
        "thread-external-lost-mirror",
    );
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let target = single_attached_target(&app);

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("user-native".to_string()),
                role: ObservedExternalProviderTurnRole::User,
                text: "native prompt outside Arroba".to_string(),
                observed_at_ms: Some(42),
            }],
        },
    )
    .expect("native user turn should mark active prompt");
    let queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued Arroba prompt",
        PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("Arroba prompt should queue behind external active prompt")
    else {
        panic!("Arroba prompt should not start while external active prompt is running");
    };
    let queued_prompt_id = prompt.id().to_string();
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
        .expect("test drift should clear the external active prompt mirror");
    let mirrored_session = app
        .sessions()
        .get_session(session.id())
        .expect("session mirror should load");
    assert!(
        mirrored_session
            .active_prompt_for_agent(agent.id())
            .is_none(),
        "test fixture should model a lost external active prompt mirror"
    );
    assert_eq!(
        mirrored_session
            .queued_prompts_for_agent(agent.id())
            .map(|queued| queued.len()),
        Some(1)
    );

    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-native".to_string()),
                    role: ObservedExternalProviderTurnRole::User,
                    text: "native prompt outside Arroba".to_string(),
                    observed_at_ms: Some(42),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("complete-native".to_string()),
                    role: ObservedExternalProviderTurnRole::Status,
                    text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
                    observed_at_ms: Some(84),
                },
            ],
        },
    )
    .expect("completion should settle even when the mirror was already missing");
    assert!(outcome.external_active_prompt_settled);

    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app),
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    );
    let dispatched = router
        .runtime_state()
        .dispatch_next_queued_prompt_after_external_settlement(session.id(), agent.id(), run.id())
        .await
        .expect("queued prompt should dispatch after external settlement");
    assert!(dispatched);
    let session = router
        .runtime_state()
        .session_snapshot(session.id())
        .await
        .expect("session should snapshot");
    let active_prompt = session
        .active_prompt_for_agent(agent.id())
        .expect("promoted queued prompt should become active");
    assert_ne!(active_prompt.id(), queued_prompt_id);
    assert_eq!(active_prompt.pending_prompt_id(), None);
    assert_eq!(active_prompt.prompt(), "queued Arroba prompt");
}

#[test]
fn observed_external_completion_uses_prompt_owner_queue_when_session_queue_mirror_was_lost() {
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
    let run = test_codex_run(
        session.id(),
        agent.id(),
        "run-external-lost-queue-mirror",
        "thread-external-lost-queue-mirror",
    );
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run);
    let target = single_attached_target(&app);
    let user_turn = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-native".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "native prompt outside Arroba".to_string(),
        observed_at_ms: Some(42),
    };
    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![user_turn.clone()],
        },
    )
    .expect("native user turn should mark active prompt");
    let queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued Arroba prompt",
        PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("Arroba prompt should queue behind external active prompt")
    else {
        panic!("Arroba prompt should not start while external active prompt is running");
    };
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
        .expect("test drift should clear the external active prompt");
    app.sessions_mut()
        .mirror_agent_prompt_state(
            session.id(),
            agent.id(),
            None,
            std::collections::VecDeque::new(),
        )
        .expect("test drift should clear the session queued prompt mirror");
    assert_eq!(
        app.prompt_owner_queued_prompt_count_for_agent(session.id(), agent.id())
            .expect("prompt owner should read"),
        1
    );
    assert!(
        app.sessions()
            .get_session(session.id())
            .expect("session should load")
            .queued_prompts_for_agent(agent.id())
            .map(|queued| queued.is_empty())
            .unwrap_or(true),
        "session mirror should not expose the queued prompt"
    );

    let outcome = append_observed_external_turns_for_attached_target_with_options(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![
                user_turn,
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("complete-native".to_string()),
                    role: ObservedExternalProviderTurnRole::Status,
                    text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
                    observed_at_ms: Some(84),
                },
            ],
        },
        AttachedExternalObserverAppendOptions {
            allow_external_active_prompt_settlement: true,
        },
    )
    .expect("completion should settle even when the session queue mirror was lost");

    assert!(outcome.external_active_prompt_settled);
}

#[test]
fn stable_external_settlement_with_lost_mirror_signals_hidden_history_refresh() {
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
        import,
    );
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: Some("user-1".to_string()),
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt".to_string(),
        observed_at_ms: Some(42),
    };
    let assistant = ObservedExternalProviderTurn {
        provider_turn_id: Some("assistant-1".to_string()),
        role: ObservedExternalProviderTurnRole::Assistant,
        text: "external reply".to_string(),
        observed_at_ms: Some(84),
    };

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone()],
        },
    )
    .expect("observed user turn should append");
    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone(), assistant.clone()],
        },
    )
    .expect("observed assistant turn should append");
    let _ = app
        .terminal_mut()
        .drain_output_records(session.id(), attachment.id());

    let queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued Arroba prompt",
        PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("Arroba prompt should queue behind external active prompt")
    else {
        panic!("Arroba prompt should not start while external prompt is running");
    };
    app.prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
        .expect("test drift should clear the external active prompt mirror");

    let outcome = append_observed_external_turns_for_attached_target_with_options(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt, assistant],
        },
        AttachedExternalObserverAppendOptions {
            allow_external_active_prompt_settlement: true,
        },
    )
    .expect("stable assistant turn should settle even when mirror is missing");

    assert_eq!(outcome.changed_count, 0);
    assert!(outcome.external_active_prompt_settled);
    let records = app
        .terminal_mut()
        .drain_output_records(session.id(), attachment.id());
    assert_eq!(
        records.len(),
        1,
        "hidden settlement history signal should fan out to attached terminals"
    );
    assert_eq!(
        records[0].bytes,
        EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS.as_bytes()
    );
    assert!(records[0]
        .merge_key
        .as_deref()
        .is_some_and(|merge_key| merge_key.contains(":state:active_prompt_settled:")));
    assert_eq!(
        records[0]
            .external_observation_metadata
            .as_ref()
            .and_then(|metadata| metadata.external_observation.as_ref())
            .map(|observation| observation.settles_active_prompt),
        Some(true)
    );
}

#[test]
fn append_observed_external_turns_persists_cursor_without_provider_run() {
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
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(
        entries[0].source,
        Some(crate::history::SessionHistoryEntrySource::ExternalProviderObserved)
    );
    assert_eq!(entries[0].provider_run_id, None);
    assert_eq!(entries[0].external_provider.as_deref(), Some("codex"));
    let persisted = app
        .agents()
        .get_agent(agent.id())
        .expect("agent should exist");
    let cursor = &persisted
        .external_provider_import()
        .expect("metadata should persist")
        .observed_cursor;
    assert_eq!(cursor.last_observed_turn_id.as_deref(), Some("item-1"));
    assert_eq!(cursor.last_observed_at_ms, Some(42));
}
