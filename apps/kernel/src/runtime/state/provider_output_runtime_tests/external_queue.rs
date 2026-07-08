use super::*;

#[tokio::test]
async fn external_active_prompt_blocks_queue_until_observer_settles_it() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-external-queue-settlement",
            "worktree-external-queue-settlement",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-external-queue-settlement",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run.clone());
    let (external_prompt_id, queued_prompt_id) =
        sync_external_active_prompt_and_queue_arroba_prompt(
            &mut app,
            session.id(),
            attachment.id(),
            agent.id(),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    assert_external_active_prompt_and_queued_arroba_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );

    let blocked = runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), run.id())
        .expect("active external prompt should not error while blocking queue dispatch");
    assert!(
        blocked.is_none(),
        "queued Arroba prompt must not dispatch while external prompt is active"
    );

    {
        let mut app = app.lock().await;
        let changed = app
            .prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
            .expect("observer settlement should clear external active prompt");
        assert!(changed);
    }

    let dispatch = runtime
        .owned
        .advance_next_queued_prompt_dispatch(session.id(), agent.id(), run.id())
        .expect("settled external prompt should release queued prompt")
        .expect("queued prompt should dispatch after external settlement");
    assert_eq!(dispatch.session_id, session.id());
    assert_eq!(dispatch.provider_run_id, run.id());
    assert_eq!(dispatch.agent_id, agent.id());
    assert_eq!(dispatch.prompt, "queued from Arroba\n");
    assert!(!dispatch.steering);

    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    let active_prompt = session_state
        .active_prompt_for_agent(agent.id())
        .expect("queued prompt should now be active");
    assert_eq!(active_prompt.id(), dispatch.prompt_id);
    assert_eq!(
        active_prompt.prompt_origin(),
        crate::session::PromptOrigin::Arroba
    );
    assert_eq!(active_prompt.prompt(), "queued from Arroba\n");
    assert!(
        session_state
            .queued_prompts_for_agent(agent.id())
            .map(|queued| queued.is_empty())
            .unwrap_or(true),
        "queue should be empty after deterministic promotion"
    );
}

#[tokio::test]
async fn external_active_prompt_rejects_queued_prompt_steering() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-external-steering",
            "worktree-external-steering",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-external-steering",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider run should launch");
    app.update_provider_run_projection(run);
    let (external_prompt_id, queued_prompt_id) =
        sync_external_active_prompt_and_queue_arroba_prompt(
            &mut app,
            session.id(),
            attachment.id(),
            agent.id(),
        );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    assert_external_active_prompt_and_queued_arroba_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );

    let error = match runtime.owned.steer_queued_prompt(
        session.id(),
        agent.id(),
        attachment.id(),
        &queued_prompt_id,
    ) {
        Ok(_) => panic!("external active prompt should reject steering"),
        Err(error) => error,
    };
    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "steer queued prompt");
            assert_eq!(
                message,
                "queued prompts cannot be steered into externally started provider turns"
            );
        }
        other => panic!("expected LocalTransport steering error, got {other:?}"),
    }

    assert_external_active_prompt_and_queued_arroba_prompt(
        &runtime,
        session.id(),
        agent.id(),
        &external_prompt_id,
        &queued_prompt_id,
    );
}
