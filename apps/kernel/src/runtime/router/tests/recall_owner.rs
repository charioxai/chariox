use super::*;

// MP-08 / MP-11: recall without a room reads every room's operational history on the kernel;
// a Room member who is not the kernel's owner recalls only through a room they belong to.
#[tokio::test]
async fn a_private_member_cannot_recall_without_a_room() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let mut room = app.session_state_store().get_session(session.id()).unwrap();
    room.add_member(
        "private-member",
        None,
        crate::session::CollaborationLevel::Private,
    );
    app.sessions_mut().restore_session(room);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let unscoped = [
        LocalDaemonRequest::QueryRecall(crate::local::QueryRecallRequest::default()),
        serde_json::from_value(serde_json::json!({ "SearchRecall": { "query": "secret" } }))
            .unwrap(),
    ];
    for request in unscoped {
        let mut command = KernelCommand::from_local_request("member-recall", None, None, &request);
        command.caller.user_id = Some("private-member".into());
        let recalled = router.dispatch(command, request.clone()).await;
        assert!(
            matches!(recalled, Err(DaemonError::OwnershipAccessDenied { .. })),
            "a Private member recalled every room's history with {request:?}: {recalled:?}"
        );
        let owner = KernelCommand::from_local_request("owner-recall", None, None, &request);
        let owned = router.dispatch(owner, request.clone()).await;
        assert!(
            owned.is_ok(),
            "the kernel owner lost {request:?}: {owned:?}"
        );
    }
}
