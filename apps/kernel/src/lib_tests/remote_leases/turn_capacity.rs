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

/// Release F cancels a leased turn only by its exact home prompt and worker run,
/// so this submission carries the home identity a real home kernel sends.
fn submit_with_home_prompt(
    app: &mut DaemonApp,
    leased_agent_id: &str,
    home_agent_id: &str,
    home_prompt_id: &str,
) -> String {
    let context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: format!("session-{home_agent_id}"),
        home_agent_id: home_agent_id.to_string(),
        home_prompt_id: home_prompt_id.to_string(),
        home_turn_id: format!("turn-{home_prompt_id}"),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: None,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "held turn cancellation".to_string(),
    };
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(app)
        .submit_leased_prompt_with_workflow_context(
            leased_agent_id,
            "remote leased prompt\n",
            Vec::new(),
            None,
            Some(context),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
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
fn steering_a_held_turn_is_delivered_when_the_turn_starts() {
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

    let (_run, dispatch) = RemoteLeaseRuntime::new(&mut app)
        .prepare_leased_prompt_steer(
            &second.id,
            "steer-1",
            "home-prompt-second",
            "more context",
            "",
            Vec::new(),
            None,
        )
        .unwrap_or_else(|error| panic!("steering a held turn should be accepted: {error}"));
    assert!(
        dispatch.is_none(),
        "a held turn's steer must not reach the provider early"
    );
    assert_eq!(
        RemoteLeaseRuntime::new(&mut app)
            .held_turn_prompt(&second.id)
            .as_deref(),
        Some("held remote prompt\n\nmore context")
    );

    RemoteLeaseRuntime::new(&mut app)
        .complete_leased_prompt(&first.id)
        .expect("running turn should complete");
    assert!(!waiting(&mut app, &second.id));
}

#[test]
fn cancelling_a_held_turn_settles_it_without_reaching_the_provider() {
    let mut app = capacity_one_worker();
    let first = leased_agent(&mut app, "agent-first");
    let second = leased_agent(&mut app, "agent-second");
    submit(&mut app, &first.id);
    let second_run =
        submit_with_home_prompt(&mut app, &second.id, "agent-second", "home-prompt-second");
    assert!(waiting(&mut app, &second.id));

    let cancellation = RemoteLeaseRuntime::new(&mut app)
        .cancel_leased_prompt(&second.id, "home-prompt-second", &second_run)
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

fn settle_on_worker(app: &mut DaemonApp, leased_agent: &crate::execution_lease::LeasedAgent) {
    app.complete_active_prompt(
        &leased_agent.backing_session_id,
        &leased_agent.backing_agent_id,
        None,
    )
    .expect("running turn should settle on the worker");
}

#[test]
fn home_drain_starts_a_held_turn_after_the_running_turn_settles() {
    let mut app = capacity_one_worker();
    let first = leased_agent(&mut app, "agent-first");
    let second = leased_agent(&mut app, "agent-second");
    submit(&mut app, &first.id);
    let second_run = submit(&mut app, &second.id);

    settle_on_worker(&mut app, &first);
    assert!(waiting(&mut app, &second.id));
    RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&second.id, &second_run, false)
        .expect("held turn projection should drain");
    assert!(!waiting(&mut app, &second.id));
    assert!(turn_reached_provider(&app, &second_run));
}

#[test]
fn held_turn_that_fails_to_start_settles_the_home_turn() {
    let mut app = capacity_one_worker();
    let first = leased_agent(&mut app, "agent-first");
    let second = leased_agent(&mut app, "agent-second");
    submit(&mut app, &first.id);
    let second_run = submit(&mut app, &second.id);
    app.providers_mut()
        .park_run_provider_only(&second.backing_session_id, &second_run)
        .expect("held turn's provider run should park");

    settle_on_worker(&mut app, &first);
    let events = RemoteLeaseRuntime::new(&mut app)
        .pump_leased_runtime_projections()
        .expect("worker pump should run");
    assert!(!waiting(&mut app, &second.id));
    assert!(app
        .prompt_owner_active_prompt_for_agent(&second.backing_session_id, &second.backing_agent_id)
        .expect("backing prompt should load")
        .is_none());
    let completions = events
        .into_iter()
        .map(
            |(
                _target,
                RelayPeerEvent::LeasedRuntimeProjection {
                    home_agent_id,
                    completions,
                    ..
                },
            )| { (home_agent_id, completions) },
        )
        .filter(|(home_agent_id, _)| home_agent_id == "agent-second")
        .flat_map(|(_, completions)| completions)
        .count();
    assert_eq!(completions, 1, "the home turn must not stay running");
}
