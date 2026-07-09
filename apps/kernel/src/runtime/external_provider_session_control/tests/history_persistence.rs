use super::*;

#[test]
fn append_observed_external_turns_persist_as_reloadable_regular_history_turn() {
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
                    provider_turn_id: Some("reasoning-1".to_string()),
                    role: ObservedExternalProviderTurnRole::Reasoning,
                    text: "external reasoning".to_string(),
                    observed_at_ms: Some(63),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("tool-1".to_string()),
                    role: ObservedExternalProviderTurnRole::Tool,
                    text: "{\"tool\":\"bash\",\"status\":\"completed\"}".to_string(),
                    observed_at_ms: Some(72),
                },
                ObservedExternalProviderTurn {
                    provider_turn_id: Some("assistant-1".to_string()),
                    role: ObservedExternalProviderTurnRole::Assistant,
                    text: "external answer".to_string(),
                    observed_at_ms: Some(84),
                },
            ],
        },
    )
    .expect("observed external turn should append");

    assert_eq!(outcome.changed_count, 4);
    let legacy_entries = app
        .history_store()
        .load(&session)
        .expect("legacy session history should load");
    assert_eq!(
        legacy_entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            SessionHistoryEntryKind::UserPrompt,
            SessionHistoryEntryKind::ProviderReasoning,
            SessionHistoryEntryKind::ProviderTool,
            SessionHistoryEntryKind::ProviderOutput,
        ]
    );
    assert_eq!(legacy_entries[0].text, "external prompt");
    assert_eq!(
        legacy_entries[0].source_attachment_id.as_deref(),
        Some("external:codex")
    );
    assert_eq!(
        legacy_entries[0].source,
        Some(crate::history::SessionHistoryEntrySource::ExternalProviderObserved)
    );

    let response = tokio::runtime::Runtime::new()
        .expect("runtime should create")
        .block_on(
            crate::runtime::history_requests::execute_session_history_outline_request(
                app.operational_history_store(),
                crate::local::GetSessionHistoryOutlineRequest {
                    session_id: session.id().to_string(),
                    agent_ids: Some(vec![agent.id().to_string()]),
                    latest_prompt_count: Some(4),
                    cursor: None,
                },
            ),
        )
        .expect("outline should load");
    let crate::local::LocalDaemonResponse::SessionHistoryOutline { agents } = response else {
        panic!("unexpected response")
    };
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].turns.len(), 1);
    let turn = &agents[0].turns[0];
    assert_eq!(turn.prompt_origin, PromptOrigin::External);
    assert_eq!(turn.external_provider.as_deref(), Some("codex"));
    assert_eq!(
        turn.external_provider_session_id.as_deref(),
        Some("thread-observed")
    );
    assert_eq!(turn.external_provider_turn_id.as_deref(), Some("user-1"));
    assert_eq!(turn.user_prompt.entry.text, "external prompt");
    assert_eq!(
        turn.user_prompt.entry.source_attachment_id.as_deref(),
        Some("external:codex")
    );
    assert_eq!(
        turn.user_prompt.entry.source,
        Some(crate::history::SessionHistoryEntrySource::ExternalProviderObserved)
    );
    assert_eq!(
        turn.summary
            .as_ref()
            .expect("assistant summary should load")
            .entry
            .text,
        "external answer"
    );
    assert_eq!(turn.blobs.len(), 1);
    assert_eq!(
        turn.blobs[0].kind,
        SessionHistoryEntryKind::ProviderReasoning
    );
    assert_eq!(turn.blobs[0].entry_count, 2);
}

#[test]
fn append_observed_external_turns_do_not_attribute_leading_blobs_to_future_prompt() {
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
                turns: vec![
                    ObservedExternalProviderTurn {
                        provider_turn_id: Some("status-before-user".to_string()),
                        role: ObservedExternalProviderTurnRole::Status,
                        text: "codex token_count\n{\"info\":{\"total_token_usage\":{\"total_tokens\":42}}}"
                            .to_string(),
                        observed_at_ms: Some(21),
                    },
                    ObservedExternalProviderTurn {
                        provider_turn_id: Some("user-1".to_string()),
                        role: ObservedExternalProviderTurnRole::User,
                        text: "external prompt".to_string(),
                        observed_at_ms: Some(42),
                    },
                    ObservedExternalProviderTurn {
                        provider_turn_id: Some("assistant-1".to_string()),
                        role: ObservedExternalProviderTurnRole::Assistant,
                        text: "external answer".to_string(),
                        observed_at_ms: Some(84),
                    },
                ],
            },
        )
        .expect("observed external turn should append");

    assert_eq!(outcome.changed_count, 3);
    let entries = app
        .load_session_history_entries(&session, Some(agent.id()))
        .expect("history should load");
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].kind, SessionHistoryEntryKind::ProviderStatus);
    assert_eq!(
        entries[0].external_provider_turn_id.as_deref(),
        Some("status-before-user")
    );
    assert_eq!(
        entries[1].external_provider_turn_id.as_deref(),
        Some("user-1")
    );
    assert_eq!(
        entries[2].external_provider_turn_id.as_deref(),
        Some("user-1")
    );
}

#[test]
fn append_observed_external_user_turn_creates_external_active_prompt() {
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
                provider_turn_id: Some("user-1".to_string()),
                role: ObservedExternalProviderTurnRole::User,
                text: "external prompt".to_string(),
                observed_at_ms: Some(42),
            }],
        },
    )
    .expect("observed user turn should append");

    assert_eq!(outcome.changed_count, 1);
    assert!(!outcome.external_active_prompt_settled);
    let active_prompt = app
        .prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("external user turn should mark active prompt");
    assert_eq!(active_prompt.prompt_origin(), PromptOrigin::External);
    assert_eq!(active_prompt.status(), PromptStatus::Running);
    assert_eq!(active_prompt.prompt(), "external prompt");
    assert_eq!(active_prompt.id(), "external:codex:thread-observed:user-1");
    let mirrored_session = app
        .sessions()
        .get_session(session.id())
        .expect("session mirror should load");
    assert_eq!(
        mirrored_session
            .active_prompt_for_agent(agent.id())
            .map(|prompt| prompt.prompt_origin()),
        Some(PromptOrigin::External)
    );
}

#[test]
fn append_observed_external_assistant_turn_clears_external_active_prompt_after_stable_poll() {
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
    append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("user-1".to_string()),
                role: ObservedExternalProviderTurnRole::User,
                text: "external prompt".to_string(),
                observed_at_ms: Some(42),
            }],
        },
    )
    .expect("observed user turn should append");
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "external user turn should mark active before assistant output"
    );

    let first_assistant_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target: target.clone(),
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("assistant-1".to_string()),
                role: ObservedExternalProviderTurnRole::Assistant,
                text: "external reply".to_string(),
                observed_at_ms: Some(84),
            }],
        },
    )
    .expect("observed assistant turn should append");

    assert_eq!(first_assistant_outcome.changed_count, 1);
    assert!(!first_assistant_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_some(),
        "new assistant output should not settle the external active marker until it is stable"
    );

    let stable_assistant_outcome = append_observed_external_turns_for_attached_target(
        &mut app,
        AttachedExternalObserverRead {
            target,
            turns: vec![ObservedExternalProviderTurn {
                provider_turn_id: Some("assistant-1".to_string()),
                role: ObservedExternalProviderTurnRole::Assistant,
                text: "external reply".to_string(),
                observed_at_ms: Some(84),
            }],
        },
    )
    .expect("stable observed assistant turn should settle");

    assert_eq!(stable_assistant_outcome.changed_count, 0);
    assert!(stable_assistant_outcome.external_active_prompt_settled);
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .expect("active prompt should load")
            .is_none(),
        "assistant output should settle the external active marker"
    );
    let mirrored_session = app
        .sessions()
        .get_session(session.id())
        .expect("session mirror should load");
    assert!(mirrored_session
        .active_prompt_for_agent(agent.id())
        .is_none());
}
