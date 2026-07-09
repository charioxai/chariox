use super::*;

#[test]
fn append_observed_external_turns_groups_codex_prompt_without_item_id_with_reply() {
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
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: None,
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt without provider item id".to_string(),
        observed_at_ms: Some(42),
    };
    let prompt_turn_id = prompt.stable_fallback_id();
    assert_eq!(prompt_turn_id, "observed-v1-user-b59a18f45f005eef");
    let reasoning = ObservedExternalProviderTurn {
        provider_turn_id: Some("reasoning-1".to_string()),
        role: ObservedExternalProviderTurnRole::Reasoning,
        text: "external reasoning".to_string(),
        observed_at_ms: Some(63),
    };
    let tool = ObservedExternalProviderTurn {
        provider_turn_id: Some("tool-1".to_string()),
        role: ObservedExternalProviderTurnRole::Tool,
        text: "{\"tool\":\"bash\",\"status\":\"completed\"}".to_string(),
        observed_at_ms: Some(72),
    };
    let reply_one = ObservedExternalProviderTurn {
        provider_turn_id: Some("msg-reply-1".to_string()),
        role: ObservedExternalProviderTurnRole::Assistant,
        text: "external reply one".to_string(),
        observed_at_ms: Some(84),
    };
    let reply_two = ObservedExternalProviderTurn {
        provider_turn_id: Some("msg-reply-2".to_string()),
        role: ObservedExternalProviderTurnRole::Assistant,
        text: "external reply two".to_string(),
        observed_at_ms: Some(126),
    };

    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![prompt.clone()],
        },
    )
    .expect("observed user turn should append");

    let changed_reply_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![
                prompt.clone(),
                reasoning.clone(),
                tool.clone(),
                reply_one.clone(),
                reply_two.clone(),
            ],
        },
    )
    .expect("observed assistant turn should append");

    assert_eq!(changed_reply_outcome.changed_count, 4);
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 5);
    assert_eq!(
        entries[0].external_provider_turn_id.as_deref(),
        Some(prompt_turn_id.as_str())
    );
    assert_eq!(
        entries[1].external_provider_turn_id.as_deref(),
        Some(prompt_turn_id.as_str())
    );
    assert_eq!(
        entries[2].external_provider_turn_id.as_deref(),
        Some(prompt_turn_id.as_str())
    );
    assert_eq!(
        entries[3].external_provider_turn_id.as_deref(),
        Some(prompt_turn_id.as_str())
    );
    assert_eq!(
        entries[4].external_provider_turn_id.as_deref(),
        Some(prompt_turn_id.as_str())
    );
    assert_eq!(entries[1].kind, SessionHistoryEntryKind::ProviderReasoning);
    assert_eq!(entries[2].kind, SessionHistoryEntryKind::ProviderTool);
    let expected_prompt_merge_key = format!("external:codex:thread-observed:{prompt_turn_id}");
    assert_eq!(
        entries[0].merge_key.as_deref(),
        Some(expected_prompt_merge_key.as_str())
    );
    assert_eq!(
        entries[1].merge_key.as_deref(),
        Some("external:codex:thread-observed:reasoning-1")
    );
    assert_eq!(
        entries[2].merge_key.as_deref(),
        Some("external:codex:thread-observed:tool-1")
    );
    assert_eq!(
        entries[3].merge_key.as_deref(),
        Some("external:codex:thread-observed:msg-reply-1")
    );
    assert_eq!(
        entries[4].merge_key.as_deref(),
        Some("external:codex:thread-observed:msg-reply-2")
    );
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "changed external assistant output should keep the external prompt marked running"
    );

    let stable_reply_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![
                prompt.clone(),
                reasoning.clone(),
                tool.clone(),
                reply_one,
                reply_two,
            ],
        },
    )
    .expect("stable observed assistant turn should stay active for Codex");

    assert_eq!(stable_reply_outcome.changed_count, 0);
    assert!(!stable_reply_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "Codex stable assistant output should stay active until task_complete"
    );

    let complete_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![
                prompt,
                reasoning,
                tool,
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("task-complete-1".to_string()),
                    role: ObservedExternalProviderTurnRole::Status,
                    text: "codex task_complete\n{\"turn_id\":\"turn-1\"}".to_string(),
                    observed_at_ms: Some(168),
                },
            ],
        },
    )
    .expect("Codex task_complete should settle");

    assert_eq!(complete_outcome.changed_count, 1);
    assert!(complete_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "Codex task_complete should clear the external active marker"
    );
}

#[test]
fn append_observed_external_turns_does_not_rewrite_different_fallback_identity() {
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
    let legacy_turn_id = "observed-user-legacydefault";
    let legacy_merge_key = format!("external:codex:thread-observed:{legacy_turn_id}");
    app.append_history_entry(
        session.id(),
        SessionHistoryEntry::external_provider_observed_with_merge_key(
            session.id(),
            None,
            agent.id(),
            SessionHistoryEntryKind::UserPrompt,
            "external prompt without provider item id",
            "codex",
            "thread-observed",
            Some(legacy_merge_key),
            Some(legacy_turn_id.to_string()),
            Some(42),
        ),
    );
    let prompt = ObservedExternalProviderTurn {
        provider_turn_id: None,
        role: ObservedExternalProviderTurnRole::User,
        text: "external prompt without provider item id".to_string(),
        observed_at_ms: Some(42),
    };
    let stable_turn_id = prompt.stable_fallback_id();

    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![prompt],
        },
    )
    .expect("external fallback history should append current identity");

    assert_eq!(outcome.changed_count, 1);
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].external_provider_turn_id.as_deref(),
        Some(legacy_turn_id)
    );
    assert_eq!(
        entries[1].external_provider_turn_id.as_deref(),
        Some(stable_turn_id.as_str())
    );
    let expected_merge_key = format!("external:codex:thread-observed:{stable_turn_id}");
    assert_eq!(
        entries[1].merge_key.as_deref(),
        Some(expected_merge_key.as_str())
    );
}

#[test]
fn append_observed_external_turns_skips_arroba_owned_prompt_echoes() {
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
            "arroba owned prompt",
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
                    text: "arroba owned prompt".to_string(),
                    observed_at_ms: Some(42),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-owned".to_string()),
                    role: ObservedExternalProviderTurnRole::Assistant,
                    text: "provider reply to arroba owned prompt".to_string(),
                    observed_at_ms: Some(84),
                },
            ],
        },
    )
    .expect("observed arroba-owned provider turn should be skipped");

    assert_eq!(outcome.changed_count, 0);
    assert!(!outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "arroba-owned prompt echoes should not create an external active prompt"
    );
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text, "arroba owned prompt");
    let persisted = app
        .agents()
        .get_agent(agent.id())
        .expect("agent should exist");
    let cursor = &persisted
        .external_provider_import()
        .expect("metadata should persist")
        .observed_cursor;
    assert_eq!(
        cursor.last_observed_turn_id.as_deref(),
        Some("assistant-owned")
    );
}

#[test]
fn arroba_owned_echo_without_completion_does_not_settle_queued_prompt() {
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
    let prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "arroba owned prompt",
        PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started {
        prompt: active_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("Arroba prompt should start")
    else {
        panic!("first Arroba prompt should start");
    };
    let queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued Arroba prompt",
        PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued {
        prompt: queued_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("second Arroba prompt should queue")
    else {
        panic!("second Arroba prompt should queue");
    };
    let target = single_attached_target(&app);

    let outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("user-owned".to_string()),
                    role: ObservedExternalProviderTurnRole::User,
                    text: "arroba owned prompt".to_string(),
                    observed_at_ms: Some(42),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-owned".to_string()),
                    role: ObservedExternalProviderTurnRole::Assistant,
                    text: "provider reply before completion".to_string(),
                    observed_at_ms: Some(84),
                },
            ],
        },
    )
    .expect("observed Arroba-owned echo should append without settlement");

    assert_eq!(outcome.changed_count, 0);
    assert!(!outcome.external_active_prompt_settled);
    let mirrored_session = app
        .sessions()
        .get_session(session.id())
        .expect("session should load");
    let prompt_state = mirrored_session
        .prompt_states()
        .get(agent.id())
        .expect("prompt state should exist");
    assert_eq!(
        prompt_state.active_prompt().map(|prompt| prompt.id()),
        Some(active_prompt.id())
    );
    assert_eq!(prompt_state.queued_prompts().len(), 1);
    assert_eq!(prompt_state.queued_prompts()[0].id(), queued_prompt.id());
}
