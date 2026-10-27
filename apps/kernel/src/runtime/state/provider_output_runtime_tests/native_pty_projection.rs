//! MP-08 / MP-10 / MP-11: native PTY regressions use the kernel-owned policy-backed pump.
use super::*;

#[tokio::test]
async fn exited_pty_is_drained_before_liveness_settlement() {
    let worktree = crate::test_support::TestWorktree::new("provider-output-exited-pty");
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-exited-pty",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude-headless",
        "default",
        "haiku",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-exited-pty",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-exited-pty".to_string(),
            pty_target: Some("test-exited-pty".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                "printf '%s\\n' 'Error: --dangerously-skip-permissions cannot be used with root/sudo privileges'; exit 1".to_string(),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    crate::app::ProviderProcessTracker::new(&mut app)
        .register_managed_run(&run)
        .expect("native fixture must register its managed process");
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "validate provider startup",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");

    for _ in 0..50 {
        if matches!(
            app.pty_mut().poll_process_state(run.id()),
            Ok(crate::pty::PtyProcessState::Exited { .. })
        ) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    runtime
        .pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            false,
        )
        .await
        .expect("provider output pump should preserve the terminal failure");

    let run = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("provider run should still exist");
    let diagnostic = run
        .terminal_diagnostic()
        .expect("provider terminal diagnostic should be recorded");
    assert!(diagnostic.contains("terminal permission error"));
    assert!(diagnostic.contains("--dangerously-skip-permissions"));
}

#[tokio::test]
async fn raw_provider_output_does_not_promote_framed_reviewer_prose_to_a_terminal_error() {
    let worktree = crate::test_support::TestWorktree::new("provider-output-reviewer-prose");
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-reviewer-prose",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-reviewer-prose",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-reviewer-prose".to_string(),
            pty_target: Some("test-reviewer-prose".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                "printf '%s\\n' 'Error: this classifier is under review.' 'The phrase unsupported model is reviewer prose.'; sleep 5".to_string(),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    crate::app::ProviderProcessTracker::new(&mut app)
        .register_managed_run(&run)
        .expect("native fixture must register its managed process");
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "review the classifier",
        crate::session::PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should start");

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut output = Vec::new();
    while std::time::Instant::now() < deadline {
        let records = runtime
            .pump_owned_provider_output(
                session.id(),
                run.id(),
                vec![attachment.id().to_string()],
                false,
            )
            .await
            .expect("reviewer output should remain ordinary provider output");
        output.extend(
            records
                .iter()
                .flat_map(|record| record.bytes.iter().copied()),
        );
        if String::from_utf8_lossy(&output).contains("unsupported model is reviewer prose.") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    runtime
        .with_app_side_effect(|app| app.pty_mut().remove_process(run.id()))
        .await
        .expect("test provider PTY should stop");
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("Error: this classifier is under review."));
    assert!(output.contains("unsupported model is reviewer prose."));

    let run = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .expect("provider run should remain available");
    assert!(
        run.terminal_diagnostic().is_none(),
        "ordinary reviewer output gained a diagnostic: {:?}",
        run.terminal_diagnostic()
    );
}

#[tokio::test]
async fn idle_claude_native_tui_projects_startup_terminal_output() {
    let worktree = crate::test_support::TestWorktree::new("provider-output-idle-claude");
    let mut app = crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-idle-claude-native",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id())
    .with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    let mut run = crate::provider::RuntimeProviderRun::new(
        "provider-run-idle-claude-native",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "test-idle-claude-native".to_string(),
            pty_target: Some("test-idle-claude-native".to_string()),
            pty_program: Some("/bin/sh".to_string()),
            pty_args: vec![
                "-lc".to_string(),
                "printf '\\033[?2004hClaude Code\\n'; sleep 5".to_string(),
            ],
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        },
    );
    run.mark_running();
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    crate::app::ProviderProcessTracker::new(&mut app)
        .register_managed_run(&run)
        .expect("native fixture must register its managed process");
    app.sessions
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.update_provider_run_projection(run.clone());

    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let records = loop {
        let records = runtime
            .pump_owned_provider_output(
                session.id(),
                run.id(),
                vec![attachment.id().to_string()],
                false,
            )
            .await
            .expect("provider output pump should preserve the startup frame");
        if !records.is_empty() {
            break records;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for the Claude native startup frame"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    runtime
        .with_app_side_effect(|app| app.pty_mut().remove_process(run.id()))
        .await
        .expect("test provider PTY should stop");

    assert!(records
        .iter()
        .all(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderTerminal));
    let output = records
        .iter()
        .flat_map(|record| record.bytes.iter().copied())
        .collect::<Vec<_>>();
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("Claude Code"));
    assert!(output.contains("\u{1b}[?2004h"));
}
