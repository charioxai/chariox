use super::*;

#[tokio::test]
async fn forwarded_provider_failure_cannot_settle_a_different_active_turn() {
    for mismatch in ["run", "node", "token", "home"] {
        assert_forwarded_failure_scope(Some(mismatch)).await;
    }
}

#[tokio::test]
async fn forwarded_provider_failure_settles_its_matching_active_turn() {
    assert_forwarded_failure_scope(None).await;
}

async fn assert_forwarded_failure_scope(mismatch: Option<&str>) {
    let config = crate::config::DaemonConfig::for_tests();
    let home_kernel_id = config.daemon_id.clone();
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "forwarded-failure-workspace",
            "forwarded-failure-worktree",
        ))
        .unwrap();
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), None)
        .unwrap();
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .unwrap();
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(session.id(), workflow.id(), node.id(), None)
        .unwrap();
    let run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("current invocation".into()),
        )
        .unwrap();
    let node_run_id = run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            run.id(),
            &node_run_id,
            "current-token".into(),
            "current invocation".into(),
            None,
            None,
        )
        .unwrap();
    app.sessions_mut()
        .start_workflow_node_run(session.id(), run.id(), &node_run_id)
        .unwrap();
    let prompt = crate::session::PromptQueueItem::new(
        "current-prompt",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(run.id()),
        agent.id(),
        "current invocation",
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(run.id(), &node_run_id);
    let active_id = match app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .unwrap()
    {
        crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
        _ => panic!("the current invocation must start"),
    };
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "forwarded-failure-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let queued = crate::session::PromptQueueItem::new(
        "next-prompt",
        attachment.id(),
        agent.id(),
        "next invocation",
        crate::session::PromptStatus::Queued,
    );
    let queued_id = match app
        .prompt_owner_submit_prepared_prompt(session.id(), queued, true)
        .unwrap()
    {
        crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
        _ => panic!("the next invocation must queue"),
    };
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let mut context = crate::execution_lease::RemoteWorkflowTurnContext {
        home_kernel_id,
        home_session_id: session.id().to_string(),
        home_agent_id: agent.id().to_string(),
        workflow_run_id: run.id().to_string(),
        workflow_node_run_id: node_run_id,
        delivery_token: "current-token".into(),
        event_reply_enabled: false,
        event_context_enabled: false,
        event_actions_enabled: false,
    };
    match mismatch {
        Some("run") => context.workflow_run_id = "previous-run".into(),
        Some("node") => context.workflow_node_run_id = "previous-node".into(),
        Some("token") => context.delivery_token = "previous-token".into(),
        Some("home") => context.home_kernel_id = "different-home".into(),
        None => {}
        _ => unreachable!(),
    }
    let result = runtime
        .dispatch_forwarded_workflow_provider_failure(
            context,
            "You've hit your session limit".into(),
        )
        .await;
    let current = runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap();
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&current, agent.id());
    if let Some(mismatch) = mismatch {
        assert_eq!(
            active.as_ref().map(|prompt| prompt.id()),
            Some(active_id.as_str()),
            "a forwarded failure with mismatched {mismatch} must not settle current work"
        );
        assert_eq!(
            current.workflow_run(run.id()).unwrap().status(),
            crate::session::WorkflowRunStatus::Running
        );
        assert_eq!(
            queued.iter().map(|prompt| prompt.id()).collect::<Vec<_>>(),
            vec![queued_id.as_str()]
        );
        if matches!(mismatch, "token" | "home") {
            assert!(result.is_err(), "invalid authority must be rejected");
        } else {
            result.expect("a stale turn is acknowledged without affecting the current turn");
        }
    } else {
        result.expect("matching failure should settle");
        assert!(
            active.is_none(),
            "matching failed prompt must settle without replay"
        );
        assert_eq!(
            queued.iter().map(|prompt| prompt.id()).collect::<Vec<_>>(),
            vec![queued_id.as_str()]
        );
    }
}
