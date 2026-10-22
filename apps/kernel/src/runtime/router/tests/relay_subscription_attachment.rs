use super::*;

#[tokio::test]
async fn relay_subscription_accepts_an_attachment_the_session_projection_has_not_caught_up_with() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let projected_before_attach = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("session snapshot should be available");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-late-projection",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    router.session_projection.update(projected_before_attach);

    router
        .ensure_relay_subscription_attachment(&session_id, attachment.id())
        .await
        .expect("an attachment the kernel holds must not be refused because the projection lags");

    match router
        .ensure_relay_subscription_attachment(&session_id, "attachment-never-joined")
        .await
    {
        Err(
            DaemonError::AttachmentNotInSession { .. } | DaemonError::AttachmentNotFound { .. },
        ) => {}
        other => panic!("an unknown attachment must still be refused: {other:?}"),
    }
}
