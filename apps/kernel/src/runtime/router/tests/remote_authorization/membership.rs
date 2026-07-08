use super::*;

#[tokio::test]
async fn remote_session_requests_require_membership() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-membership",
            "worktree-a",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let denied = router
        .dispatch(
            remote_command_for_request(&request, Some("user-2")),
            request,
        )
        .await
        .expect_err("non-member should be rejected");
    assert!(matches!(
        denied,
        DaemonError::SessionAccessDenied {
            session_id: denied_session,
            user_id
        } if denied_session == session_id && user_id == "user-2"
    ));

    let request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest { session_id });
    let missing = router
        .dispatch(remote_command_for_request(&request, None), request)
        .await
        .expect_err("remote session request without user id should be rejected");
    assert!(matches!(
        missing,
        DaemonError::MissingSessionCallerIdentity { .. }
    ));
}

#[tokio::test]
async fn remote_provider_batch_launch_requires_membership_in_every_session() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session_a = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-batch-membership-a",
            "worktree-batch-membership-a",
        ))
        .expect("session a should be created");
    let session_b = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-batch-membership-b",
            "worktree-batch-membership-b",
        ))
        .expect("session b should be created");
    let session_a_id = session_a.id().to_string();
    let session_b_id = session_b.id().to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_a_id,
            "invite-user-2".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Private,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_a_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("user should join session a");

    let request = LocalDaemonRequest::LaunchProviderRuns(crate::local::LaunchProviderRunsRequest {
        max_concurrency: Some(2),
        launches: vec![
            crate::local::LaunchProviderRunRequest {
                session_id: session_a_id.clone(),
                agent_id: None,
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
            crate::local::LaunchProviderRunRequest {
                session_id: session_b_id.clone(),
                agent_id: None,
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
        ],
    });
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let denied = router
        .dispatch(
            remote_command_for_request(&request, Some("user-2")),
            request,
        )
        .await
        .expect_err("batch launch should require membership in every session");
    assert!(matches!(
        denied,
        DaemonError::SessionAccessDenied {
            session_id,
            user_id
        } if session_id == session_b_id && user_id == "user-2"
    ));
}

#[tokio::test]
async fn remote_session_list_is_filtered_to_memberships() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session_a = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-membership",
            "worktree-a",
        ))
        .expect("session a should be created");
    let session_b = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-membership",
            "worktree-b",
        ))
        .expect("session b should be created");
    let session_a_id = session_a.id().to_string();
    let session_b_id = session_b.id().to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_b_id,
            "invite-user-2".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Private,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_b_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("user should join session b");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let response = router
        .dispatch(
            remote_command_for_request(&request, Some("user-2")),
            request,
        )
        .await
        .expect("member list should succeed");
    match response {
        LocalDaemonResponse::SessionsListed { sessions } => {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].id(), session_b_id);
            assert_ne!(sessions[0].id(), session_a_id);
        }
        _ => panic!("unexpected list response"),
    }
}
