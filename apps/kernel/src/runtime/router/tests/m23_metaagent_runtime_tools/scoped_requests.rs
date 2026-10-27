use super::*;

#[test]
fn local_metaagent_command_search_request_enforces_owner_scope() {
    run_large_stack_async_test(
        "local-metaagent-command-search",
        local_metaagent_command_search_request_enforces_owner_scope_impl,
    );
}

async fn local_metaagent_command_search_request_enforces_owner_scope_impl() {
    let env = TestMetaRuntimeEnv::new("local-command-search");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let mut owner_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    owner_caller.user_id = Some(metaagent.owner_user_id().to_string());

    let search_request =
        LocalDaemonRequest::SearchMetaagentCommands(SearchMetaagentCommandsRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            query: Some("agent".to_string()),
            tag: Some("agent".to_string()),
            scope: Some("session".to_string()),
            mutates: None,
            policy: Some("allow".to_string()),
            limit: Some(10),
        });
    let searched = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "search-metaagent-commands",
                KernelCommandSource::LocalCli,
                owner_caller,
                None,
                None,
                &search_request,
            ),
            search_request.clone(),
        )
        .await
        .expect("owner should search metaagent commands");
    let LocalDaemonResponse::MetaagentCommandsSearched { commands } = searched else {
        panic!("unexpected metaagent command search response: {searched:?}");
    };
    assert!(
        commands.iter().any(|command| {
            command
                .get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| name.contains("agent"))
        }),
        "command search should return agent command descriptors: {commands:?}"
    );

    let mut forged_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    forged_caller.user_id = Some("user-2".to_string());
    let denied = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "foreign-search-metaagent-commands",
                KernelCommandSource::LocalCli,
                forged_caller,
                None,
                None,
                &search_request,
            ),
            search_request,
        )
        .await
        .expect_err("another user must not search a metaagent command registry");
    assert!(
        denied
            .to_string()
            .contains("requires an owned session metaagent"),
        "{denied:?}"
    );
}

#[test]
fn local_metaagent_turn_inspection_requests_enforce_owner_scope() {
    run_large_stack_async_test(
        "local-metaagent-turn-inspection",
        local_metaagent_turn_inspection_requests_enforce_owner_scope_impl,
    );
}

async fn local_metaagent_turn_inspection_requests_enforce_owner_scope_impl() {
    let env = TestMetaRuntimeEnv::new("local-turn-inspection");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let prompt_entry = crate::history::SessionHistoryEntry::user_prompt(
        session.id(),
        "attachment-local-turn",
        worker.id(),
        "inspect this turn",
    );
    router
        .operational_history_store
        .append_transcript(
            &prompt_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(worker.id().to_string()),
                provider: Some(worker_run.provider().to_string()),
                model: Some(worker_run.model().to_string()),
                provider_run_id: Some(worker_run.id().to_string()),
                turn_id: Some("local-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("user prompt should append to operational history");
    let tool_entry = crate::history::SessionHistoryEntry::provider_output(
        session.id(),
        worker_run.id(),
        Some(worker.id()),
        crate::terminal::TerminalOutputKind::ProviderTool,
        None,
        serde_json::json!({
            "tool": "shell",
            "status": "completed",
            "input": {"command": "cargo test"}
        })
        .to_string(),
    );
    router
        .operational_history_store
        .append_transcript(
            &tool_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(worker.id().to_string()),
                provider: Some(worker_run.provider().to_string()),
                model: Some(worker_run.model().to_string()),
                provider_run_id: Some(worker_run.id().to_string()),
                turn_id: Some("local-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("provider tool output should append to operational history");

    let mut owner_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    owner_caller.user_id = Some(metaagent.owner_user_id().to_string());
    let overview_request =
        LocalDaemonRequest::GetMetaagentTurnOverview(GetMetaagentTurnOverviewRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            agent_ref: Some("worker".to_string()),
            turn_ref: Some("local-turn".to_string()),
            turns_back: None,
            limit: Some(20),
        });
    let overview = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "local-metaagent-turn-overview",
                KernelCommandSource::LocalCli,
                owner_caller.clone(),
                None,
                None,
                &overview_request,
            ),
            overview_request,
        )
        .await
        .expect("owner should inspect metaagent turn overview");
    let LocalDaemonResponse::MetaagentTurnOverview { overview } = overview else {
        panic!("unexpected metaagent turn overview response: {overview:?}");
    };
    assert_eq!(
        overview
            .pointer("/agent/id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
    let blob_id = overview
        .pointer("/turns/0/items")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.iter().find_map(|item| item.get("blob_id")))
        .and_then(serde_json::Value::as_str)
        .expect("overview should expose provider tool blob id")
        .to_string();

    let blob_request = LocalDaemonRequest::GetMetaagentTurnBlob(GetMetaagentTurnBlobRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        blob_id: blob_id.clone(),
    });
    let blob = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "local-metaagent-turn-blob",
                KernelCommandSource::LocalCli,
                owner_caller,
                None,
                None,
                &blob_request,
            ),
            blob_request,
        )
        .await
        .expect("owner should inspect metaagent turn blob");
    let LocalDaemonResponse::MetaagentTurnBlob { blob } = blob else {
        panic!("unexpected metaagent turn blob response: {blob:?}");
    };
    assert_eq!(
        blob.get("blob_id").and_then(serde_json::Value::as_str),
        Some(blob_id.as_str())
    );
    assert!(
        blob.pointer("/entries/0/entry/text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| text.contains("cargo test")),
        "{blob:?}"
    );

    let mut forged_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    forged_caller.user_id = Some("user-2".to_string());
    let forged_request =
        LocalDaemonRequest::GetMetaagentTurnOverview(GetMetaagentTurnOverviewRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            agent_ref: Some("worker".to_string()),
            turn_ref: Some("local-turn".to_string()),
            turns_back: None,
            limit: Some(20),
        });
    let denied = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "forged-local-metaagent-turn-overview",
                KernelCommandSource::LocalCli,
                forged_caller,
                None,
                None,
                &forged_request,
            ),
            forged_request,
        )
        .await
        .expect_err("foreign users must not inspect owned metaagent turns");
    assert!(
        denied.to_string().contains("owned session metaagent"),
        "{denied}"
    );
}

#[test]
fn local_metaagent_event_requests_enforce_owner_and_mutate_inbox() {
    run_large_stack_async_test(
        "local-metaagent-event-requests",
        local_metaagent_event_requests_enforce_owner_and_mutate_inbox_impl,
    );
}

async fn local_metaagent_event_requests_enforce_owner_and_mutate_inbox_impl() {
    let env = TestMetaRuntimeEnv::new("local-event-requests");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let event =
        app.metaagent_event_store()
            .record(crate::runtime::metaagent_event::NewMetaagentEvent {
                session_id: session.id().to_string(),
                metaagent_id: metaagent.id().to_string(),
                owner_user_id: metaagent.owner_user_id().to_string(),
                kind: "agent.turn.completed".to_string(),
                source_agent_id: Some("agent-1".to_string()),
                title: "Worker completed".to_string(),
                summary: "Worker completed a turn".to_string(),
                detail: serde_json::json!({ "prompt_id": "prompt-1" }),
                injected_prompt_id: None,
            });
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let mut owner_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    owner_caller.user_id = Some(metaagent.owner_user_id().to_string());

    let list_request = LocalDaemonRequest::ListMetaagentEvents(ListMetaagentEventsRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        limit: Some(10),
        status: None,
        kind: Some("agent.turn.completed".to_string()),
    });
    let listed = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "list-metaagent-events",
                KernelCommandSource::LocalCli,
                owner_caller.clone(),
                None,
                None,
                &list_request,
            ),
            list_request.clone(),
        )
        .await
        .expect("owner should list metaagent events");
    let LocalDaemonResponse::MetaagentEventsListed { events } = listed else {
        panic!("unexpected metaagent event list response: {listed:?}");
    };
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]
            .get("event_id")
            .and_then(serde_json::Value::as_str),
        Some(event.event_id.as_str())
    );

    let mut forged_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    forged_caller.user_id = Some("user-2".to_string());
    let denied = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "foreign-list-metaagent-events",
                KernelCommandSource::LocalCli,
                forged_caller,
                None,
                None,
                &list_request,
            ),
            list_request,
        )
        .await
        .expect_err("another user must not list a metaagent inbox");
    assert!(
        denied
            .to_string()
            .contains("requires an owned session metaagent"),
        "{denied:?}"
    );

    let read_request = LocalDaemonRequest::ReadMetaagentEvent(ReadMetaagentEventRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        event_id: event.event_id.clone(),
    });
    let read = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "read-metaagent-event",
                KernelCommandSource::LocalCli,
                owner_caller.clone(),
                None,
                None,
                &read_request,
            ),
            read_request,
        )
        .await
        .expect("owner should read metaagent event");
    let LocalDaemonResponse::MetaagentEventRead { event: read_event } = read else {
        panic!("unexpected metaagent event read response: {read:?}");
    };
    assert!(
        read_event
            .get("read_at_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "{read_event:?}"
    );

    let ack_request = LocalDaemonRequest::AckMetaagentEvents(AckMetaagentEventsRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        event_id: Some(event.event_id.clone()),
        event_ids: None,
        up_to_sequence: None,
    });
    let acked = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "ack-metaagent-event",
                KernelCommandSource::LocalCli,
                owner_caller,
                None,
                None,
                &ack_request,
            ),
            ack_request,
        )
        .await
        .expect("owner should ack metaagent event");
    let LocalDaemonResponse::MetaagentEventsAcked { acked } = acked else {
        panic!("unexpected metaagent event ack response: {acked:?}");
    };
    assert_eq!(acked.len(), 1);
    assert!(
        acked[0]
            .get("ack_at_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "{acked:?}"
    );
}
