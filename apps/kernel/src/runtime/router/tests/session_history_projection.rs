use super::*;

#[tokio::test]
async fn session_history_load_uses_warmed_session_projection_without_app_lock() {
    let mut config = DaemonConfig::for_tests();
    config.session_history_read_delay_ms = 25;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-history-load",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    app.append_user_prompt_history(
        &session_id,
        attachment.id(),
        &agent_id,
        "history from disk",
        &[],
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command =
        KernelCommand::from_local_request("cmd-history-state-warm", None, None, &state_request);
    router
        .dispatch(state_command, state_request)
        .await
        .expect("state read should warm session projection");

    let app_guard = app.lock().await;
    let history_request = LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
        session_id: session_id.clone(),
        agent_id: Some(agent_id.clone()),
        round_count: Some(10),
        max_chars: None,
        before_entry_index: None,
        before_entry_char_offset: None,
    });
    let history_command = KernelCommand::from_local_request(
        "cmd-history-without-app-lock",
        None,
        None,
        &history_request,
    );
    let history_router = router.clone();
    let history_task = tokio::spawn(async move {
        history_router
            .dispatch(history_command, history_request)
            .await
    });

    let history_response = timeout(Duration::from_millis(250), history_task)
        .await
        .expect("history load should finish while app lock is held")
        .expect("history task should join")
        .expect("history should resolve");
    drop(app_guard);

    match history_response {
        LocalDaemonResponse::SessionHistory { entries, .. } => {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].entry.text.trim_end(), "history from disk");
        }
        _ => panic!("unexpected history response"),
    }
}

#[tokio::test]
async fn query_history_reads_operational_events() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-history-query",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    app.append_user_prompt_history(
        &session_id,
        attachment.id(),
        &agent_id,
        "find this history event",
        &[],
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let history_request = LocalDaemonRequest::QueryRecall(QueryRecallRequest {
        session_id: Some(session_id.clone()),
        agent_id: Some(agent_id.clone()),
        text: Some("history event".to_string()),
        limit: Some(5),
        ..QueryRecallRequest::default()
    });
    let history_command =
        KernelCommand::from_local_request("cmd-history-query", None, None, &history_request);

    let response = router
        .dispatch(history_command, history_request)
        .await
        .expect("recall query should resolve");

    match response {
        LocalDaemonResponse::RecallEvents {
            events,
            next_sequence,
        } => {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].session_id.as_deref(), Some(session_id.as_str()));
            assert_eq!(events[0].agent_id.as_deref(), Some(agent_id.as_str()));
            assert_eq!(
                events[0].content.as_deref().map(str::trim_end),
                Some("find this history event")
            );
            assert!(next_sequence.is_none());
        }
        _ => panic!("unexpected recall query response"),
    }
}

#[tokio::test]
async fn warmed_session_history_projection_tracks_appends_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-history-projection",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    app.append_user_prompt_history(&session_id, attachment.id(), &agent_id, "first", &[]);

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let history_request = LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
        session_id: session_id.clone(),
        agent_id: Some(agent_id.clone()),
        round_count: Some(10),
        max_chars: None,
        before_entry_index: None,
        before_entry_char_offset: None,
    });
    let history_command =
        KernelCommand::from_local_request("cmd-history-warm", None, None, &history_request);
    router
        .dispatch(history_command, history_request)
        .await
        .expect("initial history read should warm projection");

    {
        let app = app.lock().await;
        app.append_user_prompt_history(&session_id, attachment.id(), &agent_id, "second", &[]);
    }

    let app_guard = app.lock().await;
    let projected_history_request =
        LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            round_count: Some(10),
            max_chars: None,
            before_entry_index: None,
            before_entry_char_offset: None,
        });
    let projected_history_command = KernelCommand::from_local_request(
        "cmd-history-projection",
        None,
        None,
        &projected_history_request,
    );
    let history_router = router.clone();
    let history_task = tokio::spawn(async move {
        history_router
            .dispatch(projected_history_command, projected_history_request)
            .await
    });

    tokio::task::yield_now().await;
    assert!(
            history_task.is_finished(),
            "warmed GetSessionHistory should be served from the history projection without app lock access"
        );
    drop(app_guard);

    let history_response = history_task
        .await
        .expect("history task should join")
        .expect("history should resolve");
    match history_response {
        LocalDaemonResponse::SessionHistory { entries, .. } => {
            let texts = entries
                .into_iter()
                .map(|entry| entry.entry.text.trim_end().to_string())
                .collect::<Vec<_>>();
            assert_eq!(texts, vec!["first".to_string(), "second".to_string()]);
        }
        _ => panic!("unexpected history response"),
    }
}

#[tokio::test]
async fn agent_scoped_session_history_warms_full_session_projection() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let first_agent_id = first_agent.id().to_string();
    let second_agent = spawn_test_agent(&mut app, &session_id, "second", "dev-stub");
    let second_agent_id = second_agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-history-projection-agents",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    app.append_user_prompt_history(
        &session_id,
        attachment.id(),
        &first_agent_id,
        "first agent transcript",
        &[],
    );
    app.append_user_prompt_history(
        &session_id,
        attachment.id(),
        &second_agent_id,
        "second agent transcript",
        &[],
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let first_history_request = LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
        session_id: session_id.clone(),
        agent_id: Some(first_agent_id.clone()),
        round_count: Some(10),
        max_chars: None,
        before_entry_index: None,
        before_entry_char_offset: None,
    });
    let first_history_command = KernelCommand::from_local_request(
        "cmd-history-first-agent-warm",
        None,
        None,
        &first_history_request,
    );
    let first_response = router
        .dispatch(first_history_command, first_history_request)
        .await
        .expect("first agent history should resolve");
    match first_response {
        LocalDaemonResponse::SessionHistory { entries, .. } => {
            let texts = entries
                .into_iter()
                .map(|entry| entry.entry.text.trim_end().to_string())
                .collect::<Vec<_>>();
            assert_eq!(texts, vec!["first agent transcript".to_string()]);
        }
        _ => panic!("unexpected history response"),
    }

    let app_guard = app.lock().await;
    let second_history_request = LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
        session_id: session_id.clone(),
        agent_id: Some(second_agent_id.clone()),
        round_count: Some(10),
        max_chars: None,
        before_entry_index: None,
        before_entry_char_offset: None,
    });
    let second_history_command = KernelCommand::from_local_request(
        "cmd-history-second-agent-projection",
        None,
        None,
        &second_history_request,
    );
    let history_router = router.clone();
    let second_history_task = tokio::spawn(async move {
        history_router
            .dispatch(second_history_command, second_history_request)
            .await
    });

    tokio::task::yield_now().await;
    assert!(
            second_history_task.is_finished(),
            "agent-scoped warmed GetSessionHistory should use the session projection without app lock access"
        );
    drop(app_guard);

    let second_response = second_history_task
        .await
        .expect("history task should join")
        .expect("second agent history should resolve");
    match second_response {
        LocalDaemonResponse::SessionHistory { entries, .. } => {
            let texts = entries
                .into_iter()
                .map(|entry| entry.entry.text.trim_end().to_string())
                .collect::<Vec<_>>();
            assert_eq!(texts, vec!["second agent transcript".to_string()]);
        }
        _ => panic!("unexpected history response"),
    }
}
