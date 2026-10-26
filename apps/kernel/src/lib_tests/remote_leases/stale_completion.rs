use super::*;

fn submit_home_prompt(app: &mut DaemonApp, leased_agent_id: &str, home_prompt_id: &str) -> String {
    let context = crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "session-stale-completion".to_string(),
        home_agent_id: "agent-home-stale-completion".to_string(),
        home_prompt_id: home_prompt_id.to_string(),
        home_turn_id: format!("turn-{home_prompt_id}"),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: None,
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "stale completion".to_string(),
    };
    let (provider_run_id, outcome) = RemoteLeaseRuntime::new(app)
        .submit_leased_prompt_with_workflow_context(
            leased_agent_id,
            &format!("{home_prompt_id}\n"),
            Vec::new(),
            None,
            Some(context),
            Vec::new(),
            None,
            crate::extension::RemoteExtensionManifest::default(),
        )
        .expect("leased prompt should be accepted");
    assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
    provider_run_id
}

/// MP-08 / MP-09 / MP-10 / MP-11 G10: a completion names the exact home prompt and worker run it
/// completes, so a late or duplicated completion of an earlier prompt cannot end the successor.
#[test]
fn late_completion_of_an_earlier_leased_prompt_leaves_the_successor_turn_active() {
    let mut config = DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "home-kernel",
            "session-stale-completion",
            "agent-home-stale-completion",
            false,
            "user-home",
        )
        .expect("execution lease should be created");
    let leased_agent = RemoteLeaseRuntime::new(&mut app)
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
        .expect("leased agent should be created");
    let earlier_run = submit_home_prompt(&mut app, &leased_agent.id, "home-prompt-earlier");
    RemoteLeaseRuntime::new(&mut app)
        .complete_leased_prompt(&leased_agent.id, "home-prompt-earlier", &earlier_run)
        .expect("the home completes the earlier prompt once");
    let successor_run = submit_home_prompt(&mut app, &leased_agent.id, "home-prompt-successor");
    let active = |app: &mut DaemonApp| {
        app.prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("active prompt should load")
        .map(|prompt| prompt.id().to_string())
    };
    let successor = active(&mut app).expect("the successor turn is running");

    // The relay may deliver the earlier completion again, after the successor started.
    let late = RemoteLeaseRuntime::new(&mut app).complete_leased_prompt(
        &leased_agent.id,
        "home-prompt-earlier",
        &earlier_run,
    );
    assert!(
        matches!(late, Err(DaemonError::LeasedPromptNotCurrent { .. })),
        "a completion that belongs to the earlier prompt is refused with a typed reason: {late:?}"
    );
    // The successor's prompt id with another run is still running there: it is
    // refused as a run change, which the home must not read as a settled prompt.
    let wrong_run = RemoteLeaseRuntime::new(&mut app).complete_leased_prompt(
        &leased_agent.id,
        "home-prompt-successor",
        "provider-run-other",
    );
    assert!(
        matches!(wrong_run, Err(DaemonError::LeasedPromptRunChanged { .. })),
        "the same prompt on another run is a live run mismatch: {wrong_run:?}"
    );
    assert_eq!(active(&mut app), Some(successor));

    RemoteLeaseRuntime::new(&mut app)
        .complete_leased_prompt(&leased_agent.id, "home-prompt-successor", &successor_run)
        .expect("the exact successor completion is accepted");
    assert_eq!(active(&mut app), None);
}
