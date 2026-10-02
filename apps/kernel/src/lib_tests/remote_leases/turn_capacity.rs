use super::*;

fn capacity_one_worker() -> DaemonApp {
    let mut config = DaemonConfig::for_tests();
    config.remote_lease_capacity = Some(1);
    DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed")
}

fn leased_agent(app: &mut DaemonApp, home_agent_id: &str) -> crate::execution_lease::LeasedAgent {
    let lease = RemoteLeaseRuntime::new(app)
        .create_execution_lease(
            "home-kernel",
            &format!("session-{home_agent_id}"),
            home_agent_id,
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    RemoteLeaseRuntime::new(app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("sonnet".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .expect("leased agent should be created")
}

fn submit(app: &mut DaemonApp, leased_agent_id: &str) -> String {
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(app)
        .submit_leased_prompt(leased_agent_id, "remote leased prompt\n", Vec::new())
        .expect("leased prompt should be accepted");
    assert!(
        matches!(outcome, PromptSubmissionOutcome::Started { .. }),
        "the home must see an accepted turn as running: {outcome:?}"
    );
    provider_run_id
}

fn turn_reached_provider(app: &DaemonApp, provider_run_id: &str) -> bool {
    app.prompt_activity.read().contains_key(provider_run_id)
}

fn waiting(app: &mut DaemonApp, leased_agent_id: &str) -> bool {
    RemoteLeaseRuntime::new(app).leased_turn_is_waiting_for_capacity(leased_agent_id)
}

#[test]
fn turn_at_capacity_waits_and_starts_when_the_running_turn_completes() {
    let mut app = capacity_one_worker();
    let first = leased_agent(&mut app, "agent-first");
    let second = leased_agent(&mut app, "agent-second");

    let first_run = submit(&mut app, &first.id);
    assert!(!waiting(&mut app, &first.id));
    assert!(turn_reached_provider(&app, &first_run));

    let second_run = submit(&mut app, &second.id);
    assert!(waiting(&mut app, &second.id));
    assert!(!turn_reached_provider(&app, &second_run));
    assert!(app.relay_registration().accepting_remote_leases);

    let (_target, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&second.id, &second_run, true)
        .expect("held turn projection should drain")
        .expect("held turn should project its waiting notice");
    let RelayPeerEvent::LeasedRuntimeProjection {
        notices,
        completions,
        ..
    } = event;
    assert!(completions.is_empty(), "a held turn must not settle");
    assert!(notices
        .iter()
        .any(|notice| notice.starts_with("Waiting for worker capacity")));
    assert!(waiting(&mut app, &second.id));

    RemoteLeaseRuntime::new(&mut app)
        .complete_leased_prompt(&first.id)
        .expect("running turn should complete");
    assert!(!waiting(&mut app, &second.id));
    assert!(turn_reached_provider(&app, &second_run));
    assert!(app
        .prompt_owner_active_prompt_for_agent(&second.backing_session_id, &second.backing_agent_id)
        .expect("held prompt should load")
        .is_some());
}

#[test]
fn held_turn_rejects_steering_until_it_starts() {
    let mut app = capacity_one_worker();
    let first = leased_agent(&mut app, "agent-first");
    let second = leased_agent(&mut app, "agent-second");
    submit(&mut app, &first.id);
    let git_context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "session-agent-second".to_string(),
        home_agent_id: "agent-second".to_string(),
        home_prompt_id: "home-prompt-second".to_string(),
        home_turn_id: "home-prompt-second".to_string(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: Some(PromptOrigin::Chariox),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "held remote prompt".to_string(),
    };
    RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &second.id,
            "held remote prompt\n",
            Vec::new(),
            None,
            Some(git_context),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("held prompt should be accepted");
    assert!(waiting(&mut app, &second.id));

    let error = match RemoteLeaseRuntime::new(&mut app).prepare_leased_prompt_steer(
        &second.id,
        "steer-1",
        "home-prompt-second",
        "more context",
        "",
        Vec::new(),
        None,
    ) {
        Ok(_) => panic!("steering a held turn must not reach the provider"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("waiting for worker capacity"));
}

#[test]
fn cancelling_a_held_turn_settles_it_without_reaching_the_provider() {
    let mut app = capacity_one_worker();
    let first = leased_agent(&mut app, "agent-first");
    let second = leased_agent(&mut app, "agent-second");
    submit(&mut app, &first.id);
    let second_run = submit(&mut app, &second.id);
    assert!(waiting(&mut app, &second.id));

    let cancellation = RemoteLeaseRuntime::new(&mut app)
        .cancel_leased_prompt(&second.id)
        .expect("held turn should cancel");
    assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelled);
    assert!(!waiting(&mut app, &second.id));
    assert!(app
        .prompt_owner_active_prompt_for_agent(&second.backing_session_id, &second.backing_agent_id)
        .expect("backing prompt should load")
        .is_none());

    let (_target, event) = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&second.id, &second_run, false)
        .expect("cancelled turn projection should drain")
        .expect("cancelled turn should project its settlement");
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = event;
    assert_eq!(completions.len(), 1, "the home needs one settlement");

    RemoteLeaseRuntime::new(&mut app)
        .complete_leased_prompt(&first.id)
        .expect("running turn should complete");
    assert!(!turn_reached_provider(&app, &second_run));
}
