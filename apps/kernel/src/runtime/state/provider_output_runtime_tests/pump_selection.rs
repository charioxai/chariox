use super::*;

#[tokio::test]
async fn owned_prompt_mirror_refreshes_projected_external_active_prompt() {
    let mut app =
        DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-owned-external-projection",
            "worktree-owned-external-projection",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let external_prompt = crate::session::PromptQueueItem::external_observed_running(
        "codex",
        "thread-owned",
        "user-1",
        agent_id.clone(),
        "external owned projection prompt",
    );

    {
        let mut app = app.lock().await;
        app.prompt_owner_sync_external_active_prompt(&session_id, &agent_id, Some(external_prompt))
            .expect("external prompt owner sync should refresh projections");
    }

    let projected = runtime
        .owned
        .session_projection
        .get(&session_id)
        .expect("session projection should refresh");
    assert_eq!(projected.agents().len(), 1);
    let active_prompt = projected
        .active_prompt_for_agent(&agent_id)
        .expect("external active prompt should project");
    assert_eq!(
        active_prompt.prompt_origin(),
        crate::session::PromptOrigin::External
    );
    assert_eq!(active_prompt.prompt(), "external owned projection prompt");
}

#[tokio::test]
async fn provider_output_pump_ignores_projected_remote_active_run() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    app.sessions
        .set_active_provider_run(
            session.id(),
            Some("remote-projected-provider-run-1".to_string()),
        )
        .expect("active provider run should be recorded");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let session_state = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist");
    let provider_run_ids = provider_run_ids_for_owned_output_pump(&runtime.owned, &session_state);

    assert!(
        provider_run_ids.is_empty(),
        "projected remote provider runs are drained through remote projection, not local PTY output"
    );
}

#[tokio::test]
async fn provider_switch_does_not_park_runs_with_active_prompts() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
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
    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("second"),
        )
        .expect("second agent should spawn");
    let idle_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("idle"),
        )
        .expect("idle agent should spawn");

    let first_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(first_agent.id()),
        )
        .expect("first provider should launch");
    app.update_provider_run_projection(first_run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(first_agent.id()),
        "first prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");

    let second_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(second_agent.id()),
        )
        .expect("second provider should launch");
    app.update_provider_run_projection(second_run.clone());

    assert_eq!(
        app.providers
            .get_run(first_run.id())
            .expect("first run should exist")
            .state(),
        crate::provider::ProviderRunState::Running,
        "launching another provider must not park a run that owns an active prompt",
    );

    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(second_agent.id()),
        "second prompt\n",
        Vec::new(),
    )
    .expect("second prompt should start");
    crate::app::KernelSessionService::new(&mut app)
        .focus_agent(session.id(), idle_agent.id())
        .expect("idle agent focus should succeed");

    assert_eq!(
        app.providers
            .get_run(second_run.id())
            .expect("second run should exist")
            .state(),
        crate::provider::ProviderRunState::Running,
        "focusing an idle agent while multiple prompts are active must not park active work",
    );
    assert_eq!(
        app.sessions
            .get_session(session.id())
            .expect("session should exist")
            .active_provider_run_id(),
        Some(second_run.id()),
        "ambiguous multi-agent prompt work should keep the active provider pointer stable",
    );
}

#[tokio::test]
async fn provider_output_pump_includes_unfocused_active_prompt_runs() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-output-pump",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("second"),
        )
        .expect("second agent should spawn");
    let idle_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "codex").with_alias("idle"),
        )
        .expect("idle agent should spawn");

    let first_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(first_agent.id()),
        )
        .expect("first provider should launch");
    app.update_provider_run_projection(first_run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(first_agent.id()),
        "first prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");

    let second_run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(second_agent.id()),
        )
        .expect("second provider should launch");
    app.update_provider_run_projection(second_run.clone());
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(second_agent.id()),
        "second prompt\n",
        Vec::new(),
    )
    .expect("second prompt should start");

    let stale_idle_run = app
        .providers
        .start_run_provider_only(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(idle_agent.id()),
        )
        .expect("stale idle provider should start without becoming active")
        .into_run();
    app.update_provider_run_projection(stale_idle_run.clone());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let session_state = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist");
    let provider_run_ids = provider_run_ids_for_owned_output_pump(&runtime.owned, &session_state);

    assert!(
        provider_run_ids.contains(first_run.id()),
        "output pump must keep draining an unfocused agent with active prompt"
    );
    assert!(
        provider_run_ids.contains(second_run.id()),
        "output pump must include the focused active prompt run"
    );
    assert!(
        !provider_run_ids.contains(stale_idle_run.id()),
        "stale Arroba runs without active prompts must not be pumped as background runs"
    );
}

#[tokio::test]
async fn provider_output_pump_includes_runs_with_pending_git_snapshots() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-1",
            "worktree-1",
        ))
        .expect("session should be created");

    let provider_run = app
        .providers
        .start_run_provider_only(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider should start without becoming active")
        .into_run();
    app.update_provider_run_projection(provider_run.clone());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .owned
        .git_turn_snapshots
        .insert(crate::git_observer::GitTurnSnapshot {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            provider: "dev-stub".to_string(),
            model: "sonnet".to_string(),
            provider_run_id: provider_run.id().to_string(),
            provider_session_id: None,
            prompt_id: "prompt-1".to_string(),
            turn_id: "prompt-1".to_string(),
            source_attachment_id: Some("attachment-1".to_string()),
            prompt_origin: Some(crate::session::PromptOrigin::Arroba),
            external_provider: None,
            external_provider_session_id: None,
            external_provider_turn_id: None,
            started_at_ms: None,
            machine_id: None,
            prompt_summary: "test prompt".to_string(),
            repo_root: "/tmp/repo".to_string(),
            worktree_path: "/tmp/repo".to_string(),
            branch: Some("main".to_string()),
            head_sha: Some("head".to_string()),
            upstream_ref: None,
            ahead_count: None,
            status_fingerprint: String::new(),
            is_dirty: false,
            workspace_live_sync_tracked: true,
            workspace_live_sync_file_snapshots: Default::default(),
        });

    let session_state = runtime
        .owned
        .session_store
        .get_session(session.id())
        .expect("session should exist");
    let provider_run_ids = provider_run_ids_for_owned_output_pump(&runtime.owned, &session_state);

    assert!(
        provider_run_ids.contains(provider_run.id()),
        "output pump must drain runs that still have pending git/WLS snapshots"
    );
}
