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
