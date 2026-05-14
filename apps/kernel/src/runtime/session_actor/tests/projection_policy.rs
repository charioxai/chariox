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
