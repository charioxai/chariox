use super::*;

/// MP-08 / MP-09 / MP-10 / MP-11 G10: `CompleteLeasedPrompt` names only the leased agent, so a late
/// or duplicated completion of an earlier prompt completes whichever prompt is active on the worker.
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
    for prompt in ["earlier prompt\n", "successor prompt\n"] {
        let (_run, outcome) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt(&leased_agent.id, prompt, Vec::new())
            .expect("leased prompt should be accepted");
        assert!(matches!(outcome, PromptSubmissionOutcome::Started { .. }));
        if prompt.starts_with("earlier") {
            RemoteLeaseRuntime::new(&mut app)
                .complete_leased_prompt(&leased_agent.id)
                .expect("the home completes the earlier prompt once");
        }
    }
    let active = |app: &mut DaemonApp| {
        app.prompt_owner_active_prompt_for_agent(
            &leased_agent.backing_session_id,
            &leased_agent.backing_agent_id,
        )
        .expect("active prompt should load")
    };
    let successor = active(&mut app).expect("the successor turn is running");

    // The relay may deliver the earlier completion again, after the successor started.
    let _late = RemoteLeaseRuntime::new(&mut app).complete_leased_prompt(&leased_agent.id);

    assert_eq!(
        active(&mut app).map(|prompt| prompt.id().to_string()),
        Some(successor.id().to_string()),
        "a completion that belongs to the earlier prompt must not complete the successor turn"
    );
}
