use super::*;

#[test]
fn session_response_projection_action_uses_response_session_and_removes_deleted_sessions() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should be available");

    match session_response_projection_action(&LocalDaemonResponse::SessionAliased {
        session: session_snapshot.clone(),
    }) {
        Some(SessionProjectionAction::Update(projected)) => {
            assert_eq!(projected.id(), session.id());
        }
        _ => panic!("session-bearing response should update projections"),
    }

    match session_response_projection_action(&LocalDaemonResponse::SessionDeleted {
        session: session_snapshot,
    }) {
        Some(SessionProjectionAction::Remove { session_id }) => {
            assert_eq!(session_id, session.id());
        }
        _ => panic!("deleted-session response should remove projections"),
    }
}

#[test]
fn explicit_terminal_run_bypasses_missing_session_active_run_preflight() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "terminal-client",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("terminal should attach");
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should be available");
    assert_eq!(session_snapshot.active_provider_run_id(), None);
    let projection = SessionStateProjectionStore::default();
    projection.update(session_snapshot);

    let explicit = LocalDaemonRequest::SendTerminalInput(SendTerminalInputRequest {
        session_id: session.id().to_string(),
        attachment_id: attachment.id().to_string(),
        provider_run_id: Some("projected-provider-run".to_string()),
        data_base64: String::new(),
    });
    assert!(
        projected_terminal_input_absence_response(&projection, &explicit).is_none(),
        "an explicit provider run should be validated by terminal routing",
    );

    let implicit = LocalDaemonRequest::SendTerminalInput(SendTerminalInputRequest {
        session_id: session.id().to_string(),
        attachment_id: attachment.id().to_string(),
        provider_run_id: None,
        data_base64: String::new(),
    });
    assert!(matches!(
        projected_terminal_input_absence_response(&projection, &implicit),
        Some(Err(DaemonError::NoActiveProviderRun { session_id }))
            if session_id == session.id()
    ));
}
