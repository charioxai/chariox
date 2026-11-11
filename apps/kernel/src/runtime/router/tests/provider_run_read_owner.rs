use super::*;

// MP-08 / MP-11: a provider run holds the owner's prompts, outputs and model; a read that misses
// the projection must refuse a Private member exactly as the projected read does.
#[tokio::test]
async fn a_private_member_cannot_read_another_users_unprojected_provider_run() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, owner) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let mut room = app.session_state_store().get_session(session.id()).unwrap();
    room.add_member(
        "private-member",
        None,
        crate::session::CollaborationLevel::Private,
    );
    app.sessions_mut().restore_session(room);
    let launch = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "owner-model",
    )
    .with_agent_id(owner.id());
    let run = crate::provider::RuntimeProviderRun::new(
        "owner-run",
        &launch,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "owner-run".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test".to_string()),
        },
    );
    assert_ne!(run.owner_user_id(), "private-member");
    // Not projected: the read takes the provider-store path.
    app.providers_mut().insert_run_for_test(run);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let request = LocalDaemonRequest::GetProviderRun(crate::local::GetProviderRunRequest {
        provider_run_id: "owner-run".into(),
    });
    let mut command = KernelCommand::from_local_request("member-run-read", None, None, &request);
    command.caller.user_id = Some("private-member".into());

    let read = router.dispatch(command, request).await;

    assert!(
        matches!(read, Err(DaemonError::OwnershipAccessDenied { .. })),
        "a Private member read the room owner's provider run: {read:?}"
    );
}

// MP-08 / MP-11: the refusal must come before the read handler runs, not from the response
// redaction after it: the handler publishes the run to the projection and schedules a
// selection sync for a run the caller may not see.
#[tokio::test]
async fn a_refused_unprojected_provider_run_read_does_not_run_the_handler() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, owner) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let mut room = app.session_state_store().get_session(session.id()).unwrap();
    room.add_member(
        "private-member",
        None,
        crate::session::CollaborationLevel::Private,
    );
    app.sessions_mut().restore_session(room);
    let launch = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "owner-model",
    )
    .with_agent_id(owner.id());
    let run = crate::provider::RuntimeProviderRun::new(
        "owner-run",
        &launch,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "owner-run".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("test".to_string()),
        },
    );
    app.providers_mut().insert_run_for_test(run);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    assert!(router.provider_run_projection.get("owner-run").is_none());
    let request = LocalDaemonRequest::GetProviderRun(crate::local::GetProviderRunRequest {
        provider_run_id: "owner-run".into(),
    });
    let mut command = KernelCommand::from_local_request("member-run-read", None, None, &request);
    command.caller.user_id = Some("private-member".into());

    let read = router.dispatch(command, request).await;

    assert!(matches!(
        read,
        Err(DaemonError::OwnershipAccessDenied { .. })
    ));
    assert!(
        router.provider_run_projection.get("owner-run").is_none(),
        "the refused read still ran the handler and published the owner's run"
    );
}
