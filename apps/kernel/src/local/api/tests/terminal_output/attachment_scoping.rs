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
