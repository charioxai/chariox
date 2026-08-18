use super::*;

#[test]
fn append_native_provider_output_batch_fans_out_and_records_history() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-batch-output", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-batch-output".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            account_profile: None,
            session_id: session.id().to_string(),
            alias: Some("parallel".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("slow-structured".to_string()),
            effort: Some("default".to_string()),
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };
    let (default_run_id, spawned_run_id) = harness.with_app_mut(|app| {
        (
            launch_slow_structured_run(app, session.id(), default_agent.id()),
            launch_slow_structured_run(app, session.id(), spawned.id()),
        )
    });

    let records = match harness
        .dispatch(LocalDaemonRequest::AppendNativeProviderOutputBatch(
            AppendNativeProviderOutputBatchRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                outputs: vec![
                    AppendNativeProviderOutputBatchItem {
                        provider_run_id: default_run_id.clone(),
                        kind: TerminalOutputKind::ProviderOutput,
                        merge_key: Some("batch-default".to_string()),
                        text: "hello from default\n".to_string(),
                    },
                    AppendNativeProviderOutputBatchItem {
                        provider_run_id: spawned_run_id.clone(),
                        kind: TerminalOutputKind::ProviderOutput,
                        merge_key: Some("batch-spawned".to_string()),
                        text: "hello from spawned\n".to_string(),
                    },
                ],
            },
        ))
        .expect("batch output should append")
    {
        LocalDaemonResponse::TerminalOutput { records } => records,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].bytes, b"hello from default\n");
    assert_eq!(records[1].bytes, b"hello from spawned\n");
    let default_history = harness
        .with_app(|app| app.load_session_history_entries(&session, Some(default_agent.id())))
        .expect("default history should load");
    assert!(default_history.iter().any(|entry| {
        entry.provider_run_id.as_deref() == Some(default_run_id.as_str())
            && entry.text.contains("hello from default")
    }));
    let spawned_history = harness
        .with_app(|app| app.load_session_history_entries(&session, Some(spawned.id())))
        .expect("spawned history should load");
    assert!(spawned_history.iter().any(|entry| {
        entry.provider_run_id.as_deref() == Some(spawned_run_id.as_str())
            && entry.text.contains("hello from spawned")
    }));
}

#[test]
fn append_native_provider_output_batch_keeps_repeated_private_outputs_scoped() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-private-batch-output", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        _ => panic!("unexpected local response"),
    };
    let (
        agent_two_id,
        run_two,
        owner_attachment_id,
        user_two_attachment_id,
        user_three_attachment_id,
    ) = harness.with_app_mut(|app| {
        {
            let mut sessions = app.sessions_mut();
            let (_, invite_two) = sessions
                .create_session_invite(
                    session.id(),
                    "invite-private-batch-output".to_string(),
                    "local".to_string(),
                    None,
                    Some(2),
                    crate::session::CollaborationLevel::Private,
                )
                .expect("invite should be created");
            sessions
                .join_session_invite(
                    session.id(),
                    invite_two.invite_id(),
                    "user-2".to_string(),
                    1,
                )
                .expect("user-2 should join");
            sessions
                .join_session_invite(
                    session.id(),
                    invite_two.invite_id(),
                    "user-3".to_string(),
                    2,
                )
                .expect("user-3 should join");
        }
        let agent_two = {
            let mut sessions = app.sessions_mut();
            let mut agents = app.agents_mut();
            agents
                .create_agent(
                    crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                        .with_alias("user-two")
                        .with_owner_user_id("user-2"),
                    &mut sessions,
                )
                .expect("user-2 agent should be created")
        };
        let owner_attachment = crate::app::KernelSessionService::new(app)
            .attach(crate::attachment::AttachRequest::for_user(
                session.id(),
                "client-owner-private-batch",
                ClientCapabilityLevel::FullTerminal,
                "local",
            ))
            .expect("owner attachment should attach");
        let user_two_attachment = crate::app::KernelSessionService::new(app)
            .attach(crate::attachment::AttachRequest::for_user(
                session.id(),
                "client-two-private-batch",
                ClientCapabilityLevel::FullTerminal,
                "user-2",
            ))
            .expect("user-2 attachment should attach");
        let user_three_attachment = crate::app::KernelSessionService::new(app)
            .attach(crate::attachment::AttachRequest::for_user(
                session.id(),
                "client-three-private-batch",
                ClientCapabilityLevel::FullTerminal,
                "user-3",
            ))
            .expect("user-3 attachment should attach");
        let run_two = launch_slow_structured_run(app, session.id(), agent_two.id());
        (
            agent_two.id().to_string(),
            run_two,
            owner_attachment.id().to_string(),
            user_two_attachment.id().to_string(),
            user_three_attachment.id().to_string(),
        )
    });

    let records = match harness
        .dispatch(LocalDaemonRequest::AppendNativeProviderOutputBatch(
            AppendNativeProviderOutputBatchRequest {
                session_id: session.id().to_string(),
                attachment_id: user_two_attachment_id.clone(),
                outputs: vec![
                    AppendNativeProviderOutputBatchItem {
                        provider_run_id: run_two.clone(),
                        kind: TerminalOutputKind::ProviderOutput,
                        merge_key: Some("private-batch-one".to_string()),
                        text: "user two private batch one\n".to_string(),
                    },
                    AppendNativeProviderOutputBatchItem {
                        provider_run_id: run_two.clone(),
                        kind: TerminalOutputKind::ProviderOutput,
                        merge_key: Some("private-batch-two".to_string()),
                        text: "user two private batch two\n".to_string(),
                    },
                ],
            },
        ))
        .expect("private batch output should append")
    {
        LocalDaemonResponse::TerminalOutput { records } => records,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(records.len(), 2);
    for record in &records {
        assert_eq!(record.agent_id.as_deref(), Some(agent_two_id.as_str()));
        assert_eq!(
            record.recipient_attachment_ids,
            vec![user_two_attachment_id.clone()]
        );
        assert_eq!(
            record.pending_recipient_attachment_ids,
            vec![user_two_attachment_id.clone()]
        );
    }

    harness.with_app_mut(|app| {
        let owner_records = crate::app::provider_output::pump_terminal_output_for_attachment(
            app,
            session.id(),
            owner_attachment_id.as_str(),
        )
        .expect("owner drain should succeed");
        let user_two_records = crate::app::provider_output::pump_terminal_output_for_attachment(
            app,
            session.id(),
            user_two_attachment_id.as_str(),
        )
        .expect("user-2 drain should succeed");
        let user_three_records = crate::app::provider_output::pump_terminal_output_for_attachment(
            app,
            session.id(),
            user_three_attachment_id.as_str(),
        )
        .expect("user-3 drain should succeed");

        assert!(owner_records.is_empty());
        assert!(user_three_records.is_empty());
        assert_eq!(
            user_two_records
                .iter()
                .map(|record| record.bytes.as_slice())
                .collect::<Vec<_>>(),
            vec![
                b"user two private batch one\n".as_slice(),
                b"user two private batch two\n".as_slice(),
            ]
        );
    });
}
