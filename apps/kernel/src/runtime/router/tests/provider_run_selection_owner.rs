use super::*;

// MP-08 / MP-11: the selection is the model the agent's owner pays for and runs, so only someone
// who may act for the agent's owner changes it; a Private member cannot switch another user's model.
#[tokio::test]
async fn a_private_member_cannot_change_another_users_provider_run_selection() {
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
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let request = LocalDaemonRequest::UpdateProviderRunSelection(
        crate::local::UpdateProviderRunSelectionRequest {
            session_id: session.id().into(),
            provider_run_id: "owner-run".into(),
            model: Some("member-model".into()),
            variant: None,
            clear_variant: false,
        },
    );
    let mut command = KernelCommand::from_local_request("member-selection", None, None, &request);
    command.caller.user_id = Some("private-member".into());

    let updated = router.dispatch(command, request).await;

    assert!(
        updated.is_err(),
        "a Private member changed the model of the room owner's agent: {updated:?}"
    );
}
