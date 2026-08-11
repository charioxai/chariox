use super::*;

#[test]
fn terminal_output_and_subscription_snapshots_are_scoped_to_attachment_owner() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, owner_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-collab-output",
            "worktree-collab-output",
        ))
        .expect("session should be created");
    {
        let mut sessions = app.sessions_mut();
        let (_, invite_two) = sessions
            .create_session_invite(
                session.id(),
                "invite-collab-output".to_string(),
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
    let (agent_two, _agent_three) = {
        let mut sessions = app.sessions_mut();
        let mut agents = app.agents_mut();
        let agent_two = agents
            .create_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("user-two")
                    .with_owner_user_id("user-2"),
                &mut sessions,
            )
            .expect("user-2 agent should be created");
        let agent_three = agents
            .create_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("user-three")
                    .with_owner_user_id("user-3"),
                &mut sessions,
            )
            .expect("user-3 agent should be created");
        (agent_two, agent_three)
    };
    let owner_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-owner",
            ClientCapabilityLevel::FullTerminal,
            "local",
        ))
        .expect("owner attachment should attach");
    let user_two_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-two",
            ClientCapabilityLevel::FullTerminal,
            "user-2",
        ))
        .expect("user-2 attachment should attach");
    let user_three_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-three",
            ClientCapabilityLevel::FullTerminal,
            "user-3",
        ))
        .expect("user-3 attachment should attach");

    let run_two = launch_slow_structured_run(&mut app, session.id(), agent_two.id());
    app.providers_mut()
        .push_finished_structured_output_poll_for_test(
            run_two.clone(),
            Ok(Some(ProviderPromptSignalBatch {
                chunks: vec![ProviderPromptChunk {
                    kind: TerminalOutputKind::ProviderOutput,
                    merge_key: Some("private-user-two".to_string()),
                    bytes: b"user two private output\n".to_vec(),
                }],
                ..ProviderPromptSignalBatch::default()
            })),
        );
    let all_attachments = app.attachments().list_session_attachment_ids(session.id());
    ProviderOutputPump::new(&mut app)
        .pump_provider_output(ProviderOutputPumpRequest {
            session_id: session.id(),
            provider_run_id: &run_two,
            recipient_attachment_ids: all_attachments,
            initial_liveness_already_checked: false,
        })
        .expect("provider output should pump");

    let owner_records = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        owner_attachment.id(),
    )
    .expect("owner drain should succeed");
    let user_two_records = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        user_two_attachment.id(),
    )
    .expect("user-2 drain should succeed");
    let user_three_records = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        user_three_attachment.id(),
    )
    .expect("user-3 drain should succeed");

    assert!(
        owner_records.is_empty(),
        "owner must not see user-2 raw output"
    );
    assert!(
        user_three_records.is_empty(),
        "user-3 must not see user-2 raw output"
    );
    assert_eq!(user_two_records.len(), 1);
    assert_eq!(
        user_two_records[0].agent_id.as_deref(),
        Some(agent_two.id())
    );
    assert_eq!(user_two_records[0].bytes, b"user two private output\n");

    let owner_snapshot = crate::runtime_transport::watch_subscription_state(
        &mut app,
        session.id(),
        owner_attachment.id(),
        true,
        None,
        0,
    );
    let user_two_snapshot = crate::runtime_transport::watch_subscription_state(
        &mut app,
        session.id(),
        user_two_attachment.id(),
        true,
        None,
        0,
    );

    let owner_snapshot = match owner_snapshot {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => {
            snapshot.expect("owner snapshot should emit")
        }
        _ => panic!("unexpected owner watch result"),
    };
    let user_two_snapshot = match user_two_snapshot {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => {
            snapshot.expect("user-2 snapshot should emit")
        }
        _ => panic!("unexpected user-2 watch result"),
    };

    assert_eq!(owner_snapshot.session.agents().len(), 3);
    let owner_visible_agent = owner_snapshot
        .session
        .agents()
        .iter()
        .find(|agent| agent.id() == owner_agent.id())
        .expect("owner should see own agent");
    assert_eq!(owner_visible_agent.provider(), owner_agent.provider());
    let owner_redacted_agent = owner_snapshot
        .session
        .agents()
        .iter()
        .find(|agent| agent.id() == agent_two.id())
        .expect("owner should see redacted collaborator handle");
    assert_eq!(owner_redacted_agent.provider(), "redacted");
    assert_eq!(owner_redacted_agent.model(), None);

    assert_eq!(user_two_snapshot.session.agents().len(), 3);
    let user_two_visible_agent = user_two_snapshot
        .session
        .agents()
        .iter()
        .find(|agent| agent.id() == agent_two.id())
        .expect("user-2 should see own agent");
    assert_ne!(user_two_visible_agent.provider(), "redacted");
    let user_two_redacted_owner_agent = user_two_snapshot
        .session
        .agents()
        .iter()
        .find(|agent| agent.id() == owner_agent.id())
        .expect("user-2 should see redacted owner handle");
    assert_eq!(user_two_redacted_owner_agent.provider(), "redacted");
    assert_eq!(user_two_redacted_owner_agent.model(), None);
    assert_eq!(
        owner_snapshot
            .session
            .collaboration_agent_counts()
            .expect("owner collaboration counts")
            .other_user_agent_count,
        2
    );
    assert_eq!(
        user_two_snapshot
            .session
            .collaboration_agent_counts()
            .expect("user-2 collaboration counts")
            .collaborator_count,
        2
    );
}

#[test]
fn terminal_trace_fanout_respects_collaboration_levels() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, owner_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-collaboration-level-output",
            "worktree-collaboration-level-output",
        ))
        .expect("session should be created");
    {
        let mut sessions = app.sessions_mut();
        for (invite_id, user_id, collaboration_level) in [
            (
                "invite-private-output",
                "user-private",
                crate::session::CollaborationLevel::Private,
            ),
            (
                "invite-transparent-output",
                "user-transparent",
                crate::session::CollaborationLevel::Transparent,
            ),
            (
                "invite-full-output",
                "user-full",
                crate::session::CollaborationLevel::Full,
            ),
        ] {
            let (_, invite) = sessions
                .create_session_invite(
                    session.id(),
                    invite_id.to_string(),
                    "local".to_string(),
                    None,
                    Some(1),
                    collaboration_level,
                )
                .expect("invite should be created");
            sessions
                .join_session_invite(session.id(), invite.invite_id(), user_id.to_string(), 1)
                .expect("collaborator should join");
        }
    }

    let owner_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-owner-collaboration-level-output",
            ClientCapabilityLevel::FullTerminal,
            "local",
        ))
        .expect("owner attachment should attach");
    let private_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-private-collaboration-level-output",
            ClientCapabilityLevel::FullTerminal,
            "user-private",
        ))
        .expect("private attachment should attach");
    let transparent_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-transparent-collaboration-level-output",
            ClientCapabilityLevel::FullTerminal,
            "user-transparent",
        ))
        .expect("transparent attachment should attach");
    let full_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            session.id(),
            "client-full-collaboration-level-output",
            ClientCapabilityLevel::FullTerminal,
            "user-full",
        ))
        .expect("full attachment should attach");
    let all_attachments = vec![
        owner_attachment.id().to_string(),
        private_attachment.id().to_string(),
        transparent_attachment.id().to_string(),
        full_attachment.id().to_string(),
    ];
    let owner_run = launch_slow_structured_run(&mut app, session.id(), owner_agent.id());

    app.fan_out_output_for_agent(
        session.id(),
        &owner_run,
        Some(owner_agent.id()),
        TerminalOutputKind::ProviderOutput,
        Some("shared-semantic-output".to_string()),
        all_attachments.clone(),
        b"shared semantic output\n",
    );

    let owner_semantic = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        owner_attachment.id(),
    )
    .expect("owner semantic output should drain");
    let private_semantic = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        private_attachment.id(),
    )
    .expect("private semantic output should drain");
    let transparent_semantic = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        transparent_attachment.id(),
    )
    .expect("transparent semantic output should drain");
    let full_semantic = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        full_attachment.id(),
    )
    .expect("full semantic output should drain");

    for records in [&owner_semantic, &transparent_semantic, &full_semantic] {
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, TerminalOutputKind::ProviderOutput);
        assert_eq!(records[0].agent_id.as_deref(), Some(owner_agent.id()));
        assert_eq!(records[0].bytes, b"shared semantic output\n");
    }
    assert!(
        private_semantic.is_empty(),
        "private collaborator must not see owner semantic output"
    );

    app.fan_out_output_for_agent(
        session.id(),
        &owner_run,
        Some(owner_agent.id()),
        TerminalOutputKind::ProviderTerminal,
        Some("owner-provider-terminal".to_string()),
        all_attachments.clone(),
        b"owner raw terminal\n",
    );

    let owner_terminal = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        owner_attachment.id(),
    )
    .expect("owner terminal output should drain");
    let private_terminal = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        private_attachment.id(),
    )
    .expect("private terminal output should drain");
    let transparent_terminal = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        transparent_attachment.id(),
    )
    .expect("transparent terminal output should drain");
    let full_terminal = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        full_attachment.id(),
    )
    .expect("full terminal output should drain");

    assert_eq!(owner_terminal.len(), 1);
    assert_eq!(owner_terminal[0].kind, TerminalOutputKind::ProviderTerminal);
    assert_eq!(owner_terminal[0].bytes, b"owner raw terminal\n");
    assert!(private_terminal.is_empty());
    assert!(transparent_terminal.is_empty());
    assert!(full_terminal.is_empty());

    app.echo_promoted_queued_prompt_to_attachments(
        session.id(),
        &owner_run,
        "prompt-collaboration-level-output",
        owner_attachment.id(),
        "shared owner prompt",
        &[],
    );

    let owner_echo = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        owner_attachment.id(),
    )
    .expect("owner prompt echo should drain");
    let private_echo = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        private_attachment.id(),
    )
    .expect("private prompt echo should drain");
    let transparent_echo = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        transparent_attachment.id(),
    )
    .expect("transparent prompt echo should drain");
    let full_echo = crate::app::provider_output::pump_terminal_output_for_attachment(
        &mut app,
        session.id(),
        full_attachment.id(),
    )
    .expect("full prompt echo should drain");

    for records in [&owner_echo, &transparent_echo, &full_echo] {
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, TerminalOutputKind::PromptEcho);
        assert_eq!(records[0].agent_id.as_deref(), Some(owner_agent.id()));
        assert_eq!(records[0].bytes, b"shared owner prompt\n");
    }
    assert!(
        private_echo.is_empty(),
        "private collaborator must not see another user's prompt echo"
    );

    app.record_assistant_message_completion(
        session.id(),
        &owner_run,
        all_attachments,
        "message-collaboration-level-output",
        42,
    );

    let owner_completions = app
        .terminal_mut()
        .drain_completion_records(session.id(), owner_attachment.id());
    let private_completions = app
        .terminal_mut()
        .drain_completion_records(session.id(), private_attachment.id());
    let transparent_completions = app
        .terminal_mut()
        .drain_completion_records(session.id(), transparent_attachment.id());
    let full_completions = app
        .terminal_mut()
        .drain_completion_records(session.id(), full_attachment.id());

    for completions in [
        &owner_completions,
        &transparent_completions,
        &full_completions,
    ] {
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].agent_id.as_deref(), Some(owner_agent.id()));
        assert_eq!(
            completions[0].message_id,
            "message-collaboration-level-output"
        );
    }
    assert!(
        private_completions.is_empty(),
        "private collaborator must not see owner completion"
    );
}
