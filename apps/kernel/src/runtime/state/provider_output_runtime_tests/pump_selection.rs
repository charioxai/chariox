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
async fn provider_output_pump_treats_unregistered_starting_pty_as_launch_in_progress() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-starting-provider-pump",
            "worktree-starting-provider-pump",
        ))
        .expect("session should be created");
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let started = runtime
        .owned
        .start_provider_launch(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(agent.id()),
        )
        .expect("provider launch should enter starting state");

    let records = runtime
        .pump_owned_provider_output(session.id(), started.run.id(), Vec::new(), false)
        .await
        .expect("a detached launch may be pumped before its PTY is registered");

    assert!(records.is_empty());
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(started.run.id())
            .expect("starting provider run should remain available")
            .state(),
        crate::provider::ProviderRunState::Starting,
    );
}

#[tokio::test]
async fn idle_focus_sync_preserves_downstream_provider_launch_in_progress() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, focused_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-starting-provider-focus",
            "worktree-starting-provider-focus",
        ))
        .expect("session should be created");
    let downstream_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("downstream"),
        )
        .expect("downstream agent should spawn");
    crate::app::KernelSessionService::new(&mut app)
        .focus_agent(session.id(), focused_agent.id())
        .expect("first agent should remain the explicit focus target");
    let focused_run = app
        .providers
        .launch_run_detached(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(focused_agent.id()),
        )
        .expect("focused provider should launch");
    let parked_focused_run = app
        .providers
        .park_run_provider_only(session.id(), focused_run.id())
        .expect("focused provider should park")
        .into_run();
    app.sessions
        .set_active_provider_run(session.id(), None)
        .expect("parked provider should no longer be active");
    app.update_provider_run_projection(parked_focused_run);

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let downstream_run = runtime
        .owned
        .start_provider_launch(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(downstream_agent.id()),
        )
        .expect("downstream provider launch should start")
        .run;
    runtime
        .owned
        .provider_run_projection
        .update(downstream_run.clone());

    assert!(
        runtime
            .owned
            .should_defer_provider_run_sync_for_focus_change(session.id(), focused_agent.id())
            .expect("focus deferral should resolve"),
        "focus changes must not retire a provider launch in progress",
    );

    runtime
        .owned
        .sync_focused_provider_run_if_idle(session.id())
        .expect("idle focus sync should not disrupt launch handoff");

    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(downstream_run.id())
            .expect("downstream run should remain available")
            .state(),
        crate::provider::ProviderRunState::Starting,
        "stale focus reconciliation must not terminate a downstream launch in progress",
    );
    assert_eq!(
        runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available")
            .active_provider_run_id(),
        Some(downstream_run.id()),
        "the downstream launch must remain the session's active provider",
    );
}

#[tokio::test]
async fn local_provider_launch_preserves_and_restores_projected_remote_predecessor() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, remote_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-remote-predecessor",
            "worktree-remote-predecessor",
        ))
        .expect("session should be created");
    let local_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("local-popup"),
        )
        .expect("local agent should spawn");
    app.agents
        .bind_remote_execution(
            remote_agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel-1".to_string(),
                worker_machine_id: "worker-machine-1".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("worker-run-1".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("remote agent should bind");
    let projected_run_id = "leased:leased-agent-1:worker-run-1";
    let mut projected_run = crate::provider::RuntimeProviderRun::from_control_capability_inference(
        "worker-run-1",
        "worker-session-1".to_string(),
        Some("leased-agent-1".to_string()),
        "dev-stub".to_string(),
    );
    projected_run.mark_running();
    let projected_run = projected_run.projected_for_home_agent_with_id(
        projected_run_id,
        session.id(),
        remote_agent.id(),
    );
    app.update_provider_run_projection(projected_run.clone());
    app.sessions
        .set_active_provider_run(session.id(), Some(projected_run_id.to_string()))
        .expect("projected remote run should be active");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let started = runtime
        .owned
        .start_provider_launch(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "popup-model",
            )
            .with_agent_id(local_agent.id()),
        )
        .expect("local provider should launch beside a projected remote run");

    assert_eq!(
        started.previous_active_run_id.as_deref(),
        Some(projected_run_id)
    );
    assert_eq!(started.run.agent_instance_id(), Some(local_agent.id()));
    assert_eq!(
        runtime.owned.provider_run_projection.get(projected_run_id),
        Some(projected_run.clone()),
        "launching locally must not mutate the worker-owned projection"
    );

    let failed = runtime
        .owned
        .provider_store
        .terminate_run_provider_only(session.id(), started.run.id())
        .expect("simulated failed launch should terminate");
    runtime
        .owned
        .clear_active_provider_run_session_pointer(session.id(), failed.run().id())
        .expect("failed local launch should clear its pointer");
    let restored = runtime
        .owned
        .resume_provider_run_for_session(session.id(), projected_run_id)
        .expect("rollback should restore the projected remote predecessor");

    assert_eq!(restored, projected_run);
    assert_eq!(
        runtime
            .owned
            .session_store
            .get_session(session.id())
            .expect("session should remain available")
            .active_provider_run_id(),
        Some(projected_run_id)
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
        "stale Chariox runs without active prompts must not be pumped as background runs"
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
            prompt_origin: Some(crate::session::PromptOrigin::Chariox),
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
