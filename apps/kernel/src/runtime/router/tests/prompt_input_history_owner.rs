use super::*;

// MP-08 / MP-11: a Room's prompt input history is each member's own recall of what they typed;
// a Private member must not read the owner's recorded inputs.
#[tokio::test]
async fn a_private_member_cannot_read_the_owners_prompt_input_history() {
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
    let record: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "RecordPromptInputHistory": {
            "session_id": session.id(), "kind": "command", "text": "/owner-only draft"
        }
    }))
    .unwrap();
    let owner = KernelCommand::from_local_request("owner-record", None, None, &record);
    router.dispatch(owner, record).await.unwrap();
    let read: LocalDaemonRequest = serde_json::from_value(serde_json::json!({
        "GetPromptInputHistory": { "session_id": session.id() }
    }))
    .unwrap();
    let mut member = KernelCommand::from_local_request("member-read", None, None, &read);
    member.caller.user_id = Some("private-member".into());
    let history = router.dispatch(member, read.clone()).await;
    assert!(
        !format!("{history:?}").contains("/owner-only draft"),
        "a Private member read the owner's prompt input history: {history:?}"
    );
    let owner = KernelCommand::from_local_request("owner-read", None, None, &read);
    let owned = router.dispatch(owner, read).await;
    assert!(
        format!("{owned:?}").contains("/owner-only draft"),
        "the owner lost their prompt input history: {owned:?}"
    );
}
