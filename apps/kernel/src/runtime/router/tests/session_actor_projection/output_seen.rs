use super::*;

#[tokio::test]
async fn acknowledge_output_seen_uses_agent_store_membership_projection() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let projection = router
        .runtime_state
        .session_snapshot_projection(&session_id, 0)
        .expect("projection should resolve");
    assert_eq!(
        projection.metadata.projection_version,
        SESSION_SNAPSHOT_PROJECTION_VERSION
    );
    let stored_session = projection.session;
    assert!(
        stored_session
            .agents()
            .iter()
            .any(|candidate| candidate.id() == agent_id),
        "projection should include the created agent"
    );

    let acknowledged = router
        .runtime_state
        .acknowledge_agent_output_seen(&session_id, &agent_id, DEFAULT_LOCAL_USER_ID)
        .await
        .expect("output acknowledgement should use agent-store membership");

    assert_eq!(acknowledged.session.id(), session_id);
    assert!(
        !acknowledged.changed,
        "acknowledging output with no unread state should be a no-op"
    );
}

#[tokio::test]
async fn duplicate_output_seen_ack_does_not_publish_session_projection() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    router
        .runtime_state
        .session_snapshot_projection(&session_id, 0)
        .expect("projection should resolve");
    let before_projection_sequence = router.session_projection_change_sequence();
    let ack_request =
        LocalDaemonRequest::AcknowledgeAgentOutputSeen(AcknowledgeAgentOutputSeenRequest {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
        });
    let ack_command =
        KernelCommand::from_local_request("cmd-output-seen-noop", None, None, &ack_request);

    let response = router
        .dispatch(ack_command, ack_request)
        .await
        .expect("duplicate output acknowledgement should still return success");

    assert!(matches!(
        response,
        LocalDaemonResponse::AgentOutputSeenAcknowledged {
            session_id: ref acknowledged_session_id,
            agent_id: ref acknowledged_agent_id,
        } if acknowledged_session_id == &session_id && acknowledged_agent_id == &agent_id
    ));
    assert_eq!(
        router.session_projection_change_sequence(),
        before_projection_sequence,
        "no-op output acknowledgements must not publish session projections"
    );
}

#[test]
fn output_seen_ack_clears_unread_activity_for_same_user_attachments_only() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(8 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("test runtime should build");
    runtime.block_on(async {
        tokio::spawn(async move {
            output_seen_ack_clears_unread_activity_for_same_user_attachments_only_inner().await
        })
        .await
        .expect("test task should complete");
    });
}

async fn output_seen_ack_clears_unread_activity_for_same_user_attachments_only_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    {
        let mut sessions = app.sessions_mut();
        let mut session = sessions
            .get_session(&session_id)
            .expect("session should be stored");
        session.add_member(
            "user-2",
            Some(DEFAULT_LOCAL_USER_ID.to_string()),
            crate::session::CollaborationLevel::Full,
        );
        sessions.restore_session(session);
    }
    let first_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            &session_id,
            "cli-same-user-a",
            ClientCapabilityLevel::FullTerminal,
            DEFAULT_LOCAL_USER_ID,
        ))
        .expect("first attachment should attach");
    let second_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            &session_id,
            "cli-same-user-b",
            ClientCapabilityLevel::FullTerminal,
            DEFAULT_LOCAL_USER_ID,
        ))
        .expect("second attachment should attach");
    let collaborator_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            &session_id,
            "cli-collaborator",
            ClientCapabilityLevel::FullTerminal,
            "user-2",
        ))
        .expect("collaborator attachment should attach");
    app.sessions_mut()
        .note_agent_output_sequence(&session_id, &agent_id, 7)
        .expect("agent output sequence should be noted");

    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let before_first = router
        .session_snapshot_projection_for_attachment(&session_id, first_attachment.id(), 0)
        .expect("first attachment projection should resolve");
    let before_second = router
        .session_snapshot_projection_for_attachment(&session_id, second_attachment.id(), 0)
        .expect("second attachment projection should resolve");
    let before_collaborator = router
        .session_snapshot_projection_for_attachment(&session_id, collaborator_attachment.id(), 0)
        .expect("collaborator attachment projection should resolve");
    assert!(
        before_first.agent_activity[&agent_id].unread_idle_output,
        "first same-user attachment should see unread output"
    );
    assert!(
        before_second.agent_activity[&agent_id].unread_idle_output,
        "second same-user attachment should see unread output"
    );
    assert!(
        before_collaborator.agent_activity[&agent_id].unread_idle_output,
        "collaborator should have an independent unread receipt"
    );
    let public_snapshot_request = LocalDaemonRequest::GetWaitingRoomPublicSnapshot(
        crate::local::GetWaitingRoomPublicSnapshotRequest,
    );
    let public_snapshot_command = KernelCommand::from_local_request(
        "cmd-output-unseen-waiting-room",
        None,
        None,
        &public_snapshot_request,
    );
    let response = router
        .dispatch(public_snapshot_command, public_snapshot_request)
        .await
        .expect("waiting room unread refresh should succeed");
    let LocalDaemonResponse::WaitingRoomPublicSnapshot { snapshot } = response else {
        panic!("unexpected waiting room response before ack");
    };
    let unread_session = snapshot
        .sessions
        .iter()
        .find(|candidate| candidate.id == session_id)
        .expect("session should be in unread waiting room snapshot");
    let unread_agent = unread_session
        .agents
        .iter()
        .find(|candidate| candidate.id == agent_id)
        .expect("agent should be in unread waiting room snapshot");
    assert_eq!(unread_session.activity.unread_idle_agent_count, 1);
    assert!(unread_agent.activity.unread_idle_output);

    let ack_request =
        LocalDaemonRequest::AcknowledgeAgentOutputSeen(AcknowledgeAgentOutputSeenRequest {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
        });
    let ack_command = KernelCommand::from_local_request(
        "cmd-output-seen-cross-attachment",
        None,
        None,
        &ack_request,
    );
    router
        .dispatch(ack_command, ack_request)
        .await
        .expect("output acknowledgement should succeed");

    let after_first = router
        .session_snapshot_projection_for_attachment(&session_id, first_attachment.id(), 1)
        .expect("first attachment projection should resolve after ack");
    let after_second = router
        .session_snapshot_projection_for_attachment(&session_id, second_attachment.id(), 1)
        .expect("second attachment projection should resolve after ack");
    let after_collaborator = router
        .session_snapshot_projection_for_attachment(&session_id, collaborator_attachment.id(), 1)
        .expect("collaborator attachment projection should resolve after ack");
    assert!(
        !after_first.agent_activity[&agent_id].unread_idle_output,
        "ack should clear unread output for the acknowledging user"
    );
    assert!(
        !after_second.agent_activity[&agent_id].unread_idle_output,
        "ack should clear unread output in another same-user terminal"
    );
    assert!(
        after_collaborator.agent_activity[&agent_id].unread_idle_output,
        "ack should not clear another user's unread output"
    );
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-output-seen-refresh-state",
        None,
        None,
        &state_request,
    );
    let response = router
        .dispatch(state_command, state_request)
        .await
        .expect("session state refresh should succeed");
    assert!(matches!(
        response,
        LocalDaemonResponse::SessionState { agent_activity, .. }
            if !agent_activity[&agent_id].unread_idle_output
    ));

    let public_snapshot_request = LocalDaemonRequest::GetWaitingRoomPublicSnapshot(
        crate::local::GetWaitingRoomPublicSnapshotRequest,
    );
    let public_snapshot_command = KernelCommand::from_local_request(
        "cmd-output-seen-refresh-waiting-room",
        None,
        None,
        &public_snapshot_request,
    );
    let response = router
        .dispatch(public_snapshot_command, public_snapshot_request)
        .await
        .expect("waiting room refresh should succeed");
    let LocalDaemonResponse::WaitingRoomPublicSnapshot { snapshot } = response else {
        panic!("unexpected waiting room response");
    };
    let refreshed_session = snapshot
        .sessions
        .iter()
        .find(|candidate| candidate.id == session_id)
        .expect("session should be in waiting room snapshot");
    let refreshed_agent = refreshed_session
        .agents
        .iter()
        .find(|candidate| candidate.id == agent_id)
        .expect("agent should be in waiting room snapshot");
    assert_eq!(refreshed_session.activity.unread_idle_agent_count, 0);
    assert!(!refreshed_agent.activity.unread_idle_output);
}

#[tokio::test]
async fn output_seen_ack_survives_kernel_restart() {
    let config = DaemonConfig::for_tests();
    let (session_id, agent_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        app.sessions_mut()
            .note_agent_output_sequence(&session_id, &agent_id, 7)
            .expect("agent output sequence should be noted");
        app.save_durable_state_snapshot()
            .expect("unread output state should be snapshotted");

        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let ack_request =
            LocalDaemonRequest::AcknowledgeAgentOutputSeen(AcknowledgeAgentOutputSeenRequest {
                session_id: session_id.clone(),
                agent_id: agent_id.clone(),
            });
        let ack_command =
            KernelCommand::from_local_request("cmd-output-seen-durable", None, None, &ack_request);
        router
            .dispatch(ack_command, ack_request)
            .await
            .expect("output acknowledgement should succeed");
        let acknowledged = router
            .runtime_state
            .session_snapshot_projection(&session_id, 0)
            .expect("acknowledged session should remain projected");
        assert!(!acknowledged.agent_activity[&agent_id].unread_idle_output);
        drop(router);
        (session_id, agent_id)
    };

    let restored = DaemonApp::bootstrap(config).expect("second daemon should boot");
    let session = restored
        .sessions()
        .get_session(&session_id)
        .expect("session should restore");
    assert!(
        !session.agent_has_unread_output(DEFAULT_LOCAL_USER_ID, &agent_id),
        "acknowledged output must stay read after kernel restart"
    );
}
