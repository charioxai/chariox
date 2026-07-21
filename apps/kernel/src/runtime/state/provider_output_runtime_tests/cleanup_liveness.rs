use super::*;

#[tokio::test]
async fn owned_end_session_clears_stale_prompt_runtime_state_for_already_ended_session() {
    let mut app =
        DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "codex",
            "default",
            "gpt-5",
        ))
        .expect("provider should launch");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .end_session(session.id())
        .await
        .expect("session should end once");
    {
        let app = app.lock().await;
        app.prompt_activity_store().write().insert(
            run.id().to_string(),
            crate::app::ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: true,
                settlement_requested: true,
            },
        );
        app.active_turn_store().start(
            crate::app::ActiveTurnState::new(
                session.id().to_string(),
                agent.id().to_string(),
                "prompt-stale".to_string(),
                run.id().to_string(),
            )
            .with_phase(crate::app::ActiveTurnPhase::Settling),
        );
    }

    runtime
        .end_session(session.id())
        .await
        .expect("already ended session should clean stale runtime state");

    let app = app.lock().await;
    assert!(
        !app.prompt_activity_store().read().contains_key(run.id()),
        "prompt activity should not survive already-ended session cleanup"
    );
    assert!(
        !app.active_turn_store().snapshot().contains_key(run.id()),
        "active turn should not survive already-ended session cleanup"
    );
}

#[tokio::test]
async fn owned_liveness_reconciliation_settles_already_ended_active_prompt() {
    let mut app =
        DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    app.update_provider_run_projection(run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "do work\n",
        Vec::new(),
    )
    .expect("prompt should start");
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    let ended = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), run.id())
        .expect("provider run should be marked ended")
        .into_run();
    app.update_provider_run_projection(ended);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let already_ended = runtime
        .reconcile_provider_run_exit(session.id(), run.id())
        .await
        .expect("already-ended liveness reconciliation should succeed");

    assert!(already_ended);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_none(),
        "already-ended provider reconciliation should close the active prompt"
    );
    let app = app.lock().await;
    assert!(
        !app.prompt_activity_store().read().contains_key(run.id()),
        "already-ended provider reconciliation should clear prompt activity"
    );
    assert!(
        !app.active_turn_store().snapshot().contains_key(run.id()),
        "already-ended provider reconciliation should clear active turn state"
    );
}

#[tokio::test]
async fn stale_provider_exit_does_not_settle_prompt_on_replacement_run() {
    let mut app =
        DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-1",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let stale_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("initial provider should launch");
    let ended = app
        .providers_mut()
        .mark_run_ended_provider_only(session.id(), stale_run.id())
        .expect("initial provider should end")
        .into_run();
    app.update_provider_run_projection(ended);
    let replacement_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("replacement provider should launch");
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "continue on the replacement\n",
        Vec::new(),
    )
    .expect("replacement prompt should start");
    let prompt_id = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist")
        .active_prompt_for_agent(agent.id())
        .expect("replacement prompt should remain active")
        .id()
        .to_string();
    app.active_turn_store()
        .start(crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            prompt_id,
            replacement_run.id().to_string(),
        ));

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let already_ended = runtime
        .reconcile_provider_run_exit(session.id(), stale_run.id())
        .await
        .expect("stale provider reconciliation should succeed");

    assert!(already_ended);
    let session_state = runtime
        .owned
        .session_snapshot(session.id())
        .expect("session snapshot should exist");
    assert!(
        session_state.active_prompt_for_agent(agent.id()).is_some(),
        "stale provider reconciliation must not settle the replacement prompt"
    );
    assert!(
        runtime
            .owned
            .active_turns
            .snapshot()
            .contains_key(replacement_run.id()),
        "replacement active turn must remain tracked"
    );
}

#[tokio::test]
async fn owned_destroy_agent_clears_stale_prompt_runtime_state_for_ended_provider_runs() {
    let mut app =
        DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "codex",
                "default",
                "gpt-5",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should launch");
    let ended = app
        .providers_mut()
        .terminate_run_provider_only(session.id(), run.id())
        .expect("provider run should end")
        .into_run();
    app.update_provider_run_projection(ended);
    app.prompt_activity_store().write().insert(
        run.id().to_string(),
        crate::app::ActivePromptState {
            last_output_at: Some(Instant::now()),
            saw_response_content: true,
            completion_recorded: true,
            settlement_requested: true,
        },
    );
    app.active_turn_store().start(
        crate::app::ActiveTurnState::new(
            session.id().to_string(),
            agent.id().to_string(),
            "prompt-stale".to_string(),
            run.id().to_string(),
        )
        .with_phase(crate::app::ActiveTurnPhase::Settling),
    );

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .destroy_agent(agent.id(), crate::session::DEFAULT_LOCAL_USER_ID)
        .await
        .expect("agent should be destroyed");

    let app = app.lock().await;
    assert!(
        !app.prompt_activity_store().read().contains_key(run.id()),
        "destroying an agent should clear prompt activity for ended provider runs"
    );
    assert!(
        !app.active_turn_store().snapshot().contains_key(run.id()),
        "destroying an agent should clear active turns for ended provider runs"
    );
}
