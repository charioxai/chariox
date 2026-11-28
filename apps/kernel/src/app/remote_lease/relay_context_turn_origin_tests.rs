use super::*;

fn home_turn(id: &str) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: "origin-home-room".into(),
        home_agent_id: "origin-home-agent".into(),
        home_prompt_id: id.into(),
        home_turn_id: id.into(),
        source_attachment_id: None,
        workspace_live_sync_mode: None,
        prompt_origin: Some(crate::session::PromptOrigin::Chariox),
        external_provider: None,
        external_provider_session_id: None,
        external_provider_turn_id: None,
        prompt_summary: "inspect metadata".into(),
    }
}

#[test]
fn forwarding_origin_follows_a_promoted_plain_leased_prompt() {
    let mut config = crate::config::DaemonConfig::for_tests();
    config.accept_remote_leases = true;
    let mut app = crate::app::DaemonApp::bootstrap(config).unwrap();
    let lease = RemoteLeaseRuntime::new(&mut app)
        .create_execution_lease(
            "origin-home-kernel",
            "origin-home-room",
            "origin-home-agent",
            false,
            "origin-owner",
        )
        .unwrap();
    let agent = RemoteLeaseRuntime::new(&mut app)
        .create_leased_agent(
            &lease.id,
            "managed-dev-stub",
            "default",
            Some("default".into()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let (_, first) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &agent.id,
            "first home turn",
            vec![],
            None,
            Some(home_turn("first-home-origin")),
            vec![],
            Default::default(),
        )
        .unwrap();
    assert!(matches!(
        first,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));
    let (_, second) = RemoteLeaseRuntime::new(&mut app)
        .submit_leased_prompt_with_workflow_context(
            &agent.id,
            "queued home turn",
            vec![],
            None,
            Some(home_turn("second-home-origin")),
            vec![],
            Default::default(),
        )
        .unwrap();
    assert!(matches!(
        second,
        crate::session::PromptSubmissionOutcome::Queued { .. }
    ));
    RemoteLeaseRuntime::new(&mut app)
        .settle_leased_prompt(&agent.id)
        .unwrap();
    let active = app
        .prompt_owner_active_prompt_for_agent(&agent.backing_session_id, &agent.backing_agent_id)
        .unwrap()
        .expect("the queued backing turn should be promoted");
    let admitted_home = active
        .leased_skill_context()
        .and_then(|context| context.home_prompt_id.as_deref());
    assert_eq!(admitted_home, Some("second-home-origin"));
    let run = app
        .providers()
        .get_run_for_agent(&agent.backing_session_id, &agent.backing_agent_id)
        .unwrap();
    let session = app
        .sessions()
        .get_session(&agent.backing_session_id)
        .unwrap();
    let forwarded = RemoteLeaseRuntime::new(&mut app)
        .leased_workspace_live_sync_context_for_provider_run(
            run.id(),
            crate::io::WorkspaceIdentity::local(session.worktree_id()),
        )
        .expect("the current leased worker should have its home context");
    assert_eq!(
        forwarded.home_prompt_id.as_deref(),
        Some("second-home-origin"),
        "forwarding must use the same authoritative origin as the promoted envelope"
    );
}
