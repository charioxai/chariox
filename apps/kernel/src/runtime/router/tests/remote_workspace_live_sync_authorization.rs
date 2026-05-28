use super::*;

#[tokio::test]
async fn remote_workspace_live_sync_requests_require_membership_and_record_member_identity() {
    let denied_app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let denied_session = denied_app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-live-sync-auth",
            "/tmp/workspace-live-sync-auth-main",
        ))
        .expect("session should be created");
    let denied_session_id = denied_session.id().to_string();
    let denied_app = Arc::new(Mutex::new(denied_app));
    let denied_router = CommandRouter::with_interactive_capacity(Arc::clone(&denied_app), 2);

    let status_request =
        LocalDaemonRequest::GetWorkspaceLiveSyncStatus(GetWorkspaceLiveSyncStatusRequest {
            session_id: denied_session_id.clone(),
        });
    let denied = denied_router
        .dispatch(
            remote_command_for_request(&status_request, Some("user-2")),
            status_request,
        )
        .await
        .expect_err("non-member live sync status should be rejected");
    assert!(matches!(
        denied,
        DaemonError::SessionAccessDenied {
            session_id: ref denied_session,
            user_id: ref denied_user,
        } if denied_session == &denied_session_id && denied_user == "user-2"
    ));

    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-live-sync-auth",
            "/tmp/workspace-live-sync-auth-main",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_id,
            "invite-user-2".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Private,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("user should join session");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 2);

    let create_request = LocalDaemonRequest::CreateWorkspaceLink(CreateWorkspaceLinkRequest {
        session_id: session_id.clone(),
        name: "team-sync".to_string(),
    });
    let link = match router
        .dispatch(
            remote_command_for_request(&create_request, Some("user-2")),
            create_request,
        )
        .await
        .expect("member should create workspace link")
    {
        LocalDaemonResponse::WorkspaceLinkCreated { link, .. } => link,
        other => panic!("unexpected create response: {other:?}"),
    };
    assert_eq!(link.created_by_user_id(), "user-2");

    let attach_request = LocalDaemonRequest::AttachWorkspaceLink(AttachWorkspaceLinkRequest {
        session_id: session_id.clone(),
        link_ref: "team-sync".to_string(),
        repo_root: Some("/tmp/workspace-live-sync-auth-user-2".to_string()),
        branch: Some("tracked-peer".to_string()),
        repo_fingerprint: Some("repo-fingerprint-user-2".to_string()),
    });
    let attachment = match router
        .dispatch(
            remote_command_for_request(&attach_request, Some("user-2")),
            attach_request,
        )
        .await
        .expect("member should attach workspace live sync target")
    {
        LocalDaemonResponse::WorkspaceLinkAttached { attachment, .. } => attachment,
        other => panic!("unexpected attach response: {other:?}"),
    };
    assert_eq!(attachment.user_id(), "user-2");
    assert_eq!(
        attachment.repo_root(),
        "/tmp/workspace-live-sync-auth-user-2"
    );
    assert_eq!(attachment.branch(), Some("tracked-peer"));
    assert_eq!(
        attachment.repo_fingerprint(),
        Some("repo-fingerprint-user-2")
    );

    let status_request =
        LocalDaemonRequest::GetWorkspaceLiveSyncStatus(GetWorkspaceLiveSyncStatusRequest {
            session_id: session_id.clone(),
        });
    let status = match router
        .dispatch(
            remote_command_for_request(&status_request, Some("user-2")),
            status_request,
        )
        .await
        .expect("member live sync status should succeed")
    {
        LocalDaemonResponse::WorkspaceLiveSyncStatus { status } => status,
        other => panic!("unexpected status response: {other:?}"),
    };
    assert_eq!(status.sync_groups.len(), 1);
    assert_eq!(status.sync_groups[0].target_count, 1);
    assert_eq!(status.targets.len(), 1);
    assert_eq!(status.targets[0].user_id, "user-2");
    assert_eq!(
        status.targets[0].repo_root,
        "/tmp/workspace-live-sync-auth-user-2"
    );
    assert_eq!(status.targets[0].branch.as_deref(), Some("tracked-peer"));
}
