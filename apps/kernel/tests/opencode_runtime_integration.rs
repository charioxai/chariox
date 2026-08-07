use std::env;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use arroba_kernel::attachment::{AttachRequest, ClientCapabilityLevel};
use arroba_kernel::provider::{LaunchProviderRequest, ProviderRunState};
use arroba_kernel::session::CreateSessionRequest;
use arroba_kernel::terminal::TerminalOutputKind;
use arroba_kernel::{DaemonApp, DaemonConfig};

mod support;
use support::runtime_integration::{
    collect_provider_output_until, collect_provider_records_until, create_opencode_fixture_script,
    opencode_env_guard, output_timeout_ms, render_terminal_output,
    wait_for_mock_opencode_event_subscription, MockOpenCodeServer,
};

#[test]
fn shared_opencode_endpoint_keeps_prompt_queue_running_without_managed_process() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::remove_var("ARROBA_OPENCODE_BIN");
    env::remove_var("ARROBA_OPENCODE_PORT");
    let endpoint = format!("http://127.0.0.1:{}", mock_server.port());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_structured_endpoint(endpoint),
        )
        .expect("provider run should launch");
    wait_for_mock_opencode_event_subscription(&mock_server);

    let _ = arroba_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "first exit prompt\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let _ = arroba_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "second exit prompt\n",
        Vec::new(),
    )
    .expect("second prompt should queue");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: first exit prompt")
                && output.contains("fixture response: second exit prompt")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
        },
    );

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(output.contains("fixture response: first exit prompt"));
    assert!(output.contains("fixture response: second exit prompt"));
    assert_eq!(session_state.active_provider_run_id(), Some(run.id()));
    assert!(session_state.active_prompt().is_none());
    assert!(session_state.queued_prompts().is_empty());
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should remain queryable")
            .state(),
        ProviderRunState::Running
    );
    assert!(!app.pty().has_process(run.id()));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
}

#[test]
fn shared_opencode_idle_status_completes_the_prompt_without_hot_polling() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(100));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::remove_var("ARROBA_OPENCODE_BIN");
    env::remove_var("ARROBA_OPENCODE_PORT");
    let endpoint = format!("http://127.0.0.1:{}", mock_server.port());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_structured_endpoint(endpoint),
        )
        .expect("provider run should launch");
    wait_for_mock_opencode_event_subscription(&mock_server);

    let _ = arroba_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "completes without a settle window\n",
        Vec::new(),
    )
    .expect("prompt should start");

    thread::sleep(Duration::from_millis(120));
    let mut output = Vec::new();
    let completion_deadline =
        Instant::now() + Duration::from_millis(output_timeout_ms().max(2_000));
    loop {
        let recipients = app.attachments().list_session_attachment_ids(session.id());
        output.extend(
            arroba_kernel::transport::TransportService::pump_provider_output(
                &mut app,
                session.id(),
                run.id(),
                recipients,
            )
            .expect("pump after OpenCode idle should succeed"),
        );
        let session_after_pump = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist after completion");
        if session_after_pump.active_prompt().is_none() {
            break;
        }
        assert!(
            Instant::now() < completion_deadline,
            "OpenCode idle should complete the active prompt within bounded structured-poll latency"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        output
            .iter()
            .any(|record| matches!(record.kind, TerminalOutputKind::ProviderOutput)),
        "the completion pump should include the assistant output"
    );
    let terminal_text = app
        .terminal()
        .output_records()
        .iter()
        .filter(|record| record.session_id == session.id())
        .filter_map(|record| String::from_utf8(record.bytes.clone()).ok())
        .collect::<String>();
    assert!(terminal_text.contains("fixture response: completes without a settle window"));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
}

#[test]
fn event_stream_disconnect_reconnects_without_restarting_the_provider_run() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.disconnect_next_event_stream();
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "prompt after reconnect\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: prompt after reconnect")
                && app
                    .providers()
                    .get_run(run.id())
                    .expect("run should remain queryable")
                    .state()
                    == ProviderRunState::Running
        },
    );

    assert!(output.contains("fixture response: prompt after reconnect"));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn event_stream_reconnect_retries_temporary_http_failures_without_restarting_the_provider_run() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.disconnect_next_event_stream();
    mock_server.fail_next_event_stream_attempts(2);
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "prompt after transient reconnect failures\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: prompt after transient reconnect failures")
                && app
                    .providers()
                    .get_run(run.id())
                    .expect("run should remain queryable")
                    .state()
                    == ProviderRunState::Running
        },
    );

    assert!(output.contains("fixture response: prompt after transient reconnect failures"));
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should remain available")
            .state(),
        ProviderRunState::Running
    );

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn external_opencode_endpoint_accepts_prompts_and_streams_output() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::remove_var("ARROBA_OPENCODE_BIN");
    env::remove_var("ARROBA_OPENCODE_PORT");
    let endpoint = format!("http://127.0.0.1:{}", mock_server.port());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_structured_endpoint(endpoint),
        )
        .expect("provider run should launch against external endpoint");

    app.resize_terminal(session.id(), 120, 40)
        .expect("external endpoint resize should be a no-op");

    let _ = arroba_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "prompt through external endpoint\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: prompt through external endpoint")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should remain available")
                    .active_prompt()
                    .is_none()
        },
    );

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(output.contains("fixture response: prompt through external endpoint"));
    assert!(session_state.active_prompt().is_none());
    assert_eq!(session_state.active_provider_run_id(), Some(run.id()));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
}

#[test]
fn launch_retries_temporary_event_subscription_failures() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.fail_next_event_stream_attempts(2);
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch after retrying event subscribe failures");

    assert_eq!(run.state(), ProviderRunState::Running);

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn shared_opencode_endpoint_routes_multi_agent_prompts_without_pty_exit() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::remove_var("ARROBA_OPENCODE_BIN");
    env::remove_var("ARROBA_OPENCODE_PORT");
    let endpoint = format!("http://127.0.0.1:{}", mock_server.port());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, default_agent) = app
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let default_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(default_agent.id())
                .with_structured_endpoint(endpoint.clone()),
        )
        .expect("default provider run should launch");
    let reviewer = app
        .spawn_agent(
            arroba_kernel::agent::CreateAgentRequest::new(session.id(), "opencode")
                .with_alias("reviewer"),
        )
        .expect("reviewer agent should spawn");
    let reviewer_run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_agent_id(reviewer.id())
                .with_structured_endpoint(endpoint),
        )
        .expect("reviewer provider run should launch");

    app.focus_agent(session.id(), default_agent.id())
        .expect("default agent should focus");
    let _ = arroba_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "first exit prompt on default\n",
        Vec::new(),
    )
    .expect("first prompt should start");

    app.focus_agent(session.id(), reviewer.id())
        .expect("reviewer agent should focus");
    let _ = arroba_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "reviewer prompt after default exit\n",
        Vec::new(),
    )
    .expect("reviewer prompt should queue");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let timeout_ms = output_timeout_ms().max(6_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut default_output = Vec::new();
    let mut reviewer_output = Vec::new();

    loop {
        for record in arroba_kernel::transport::TransportService::pump_provider_output(
            &mut app,
            session.id(),
            default_run.id(),
            recipients.clone(),
        )
        .expect("default provider output should pump")
        {
            default_output.extend(record.bytes);
        }
        for record in arroba_kernel::transport::TransportService::pump_provider_output(
            &mut app,
            session.id(),
            reviewer_run.id(),
            recipients.clone(),
        )
        .expect("reviewer provider output should pump")
        {
            reviewer_output.extend(record.bytes);
        }

        let default_text = String::from_utf8_lossy(&default_output).into_owned();
        let reviewer_text = String::from_utf8_lossy(&reviewer_output).into_owned();
        let session_state = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");
        let default_run_state = app
            .providers()
            .get_run(default_run.id())
            .expect("default run should remain queryable")
            .state();

        if default_text.contains("fixture response: first exit prompt on default")
            && reviewer_text.contains("fixture response: reviewer prompt after default exit")
            && default_run_state == ProviderRunState::Parked
            && session_state.active_prompt().is_none()
        {
            break;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for multi-agent provider exit recovery after {timeout_ms}ms: default={default_text:?} reviewer={reviewer_text:?} active_run={:?} active_prompt={:?} default_active_prompt={:?} reviewer_active_prompt={:?} default_provider_session={:?} reviewer_provider_session={:?} prompt_requests={:?} session_responses={:?}",
            session_state.active_provider_run_id(),
            session_state.active_prompt().map(|prompt| prompt.id().to_string()),
            session_state.active_prompt_for_agent(default_agent.id()).map(|prompt| prompt.id().to_string()),
            session_state.active_prompt_for_agent(reviewer.id()).map(|prompt| prompt.id().to_string()),
            app.providers().get_run(default_run.id()).ok().and_then(|run| run.provider_session_id().map(str::to_string)),
            app.providers().get_run(reviewer_run.id()).ok().and_then(|run| run.provider_session_id().map(str::to_string)),
            mock_server.prompt_requests(),
            mock_server.session_response_text(),
        );
        thread::sleep(Duration::from_millis(25));
    }

    let default_output = String::from_utf8_lossy(&default_output).into_owned();
    let reviewer_output = String::from_utf8_lossy(&reviewer_output).into_owned();
    assert!(default_output.contains("fixture response: first exit prompt on default"));
    assert!(reviewer_output.contains("fixture response: reviewer prompt after default exit"));

    let settled_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(settled_state.focused_agent_id(), Some(reviewer.id()));
    assert!(settled_state.queued_prompts().is_empty());
    assert_eq!(
        app.providers()
            .get_run(default_run.id())
            .expect("default run should remain queryable")
            .state(),
        ProviderRunState::Parked
    );
    assert_eq!(
        app.providers()
            .get_run(reviewer_run.id())
            .expect("reviewer run should remain queryable")
            .state(),
        ProviderRunState::Running
    );
    assert!(!app.pty().has_process(default_run.id()));
    assert!(!app.pty().has_process(reviewer_run.id()));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
}

#[test]
fn managed_opencode_fixture_without_target_port_fails_health_check() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script(10);
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::remove_var("ARROBA_OPENCODE_PORT");

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let error = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect_err("missing OpenCode port override should fail");

    match error {
        arroba_kernel::DaemonError::ProviderProtocol { operation, .. } => {
            assert_eq!(operation, "health");
        }
        other => panic!("unexpected error: {other}"),
    }

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(session_state.active_provider_run_id().is_none());

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn opencode_event_stream_does_not_depend_on_session_status_polling() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script(10);
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.set_omit_session_status(true);
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::set_var("ARROBA_OPENCODE_BIN", &fixture_path);
    env::set_var("ARROBA_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(LaunchProviderRequest::new(
            session.id(),
            "opencode",
            "opencode",
            "default",
            "default",
        ))
        .expect("provider run should launch");

    let _ = arroba_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "prompt without session status polling\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: prompt without session status polling")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
        },
    );

    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist");
    assert!(output.contains("fixture response: prompt without session status polling"));
    assert_eq!(session_state.active_provider_run_id(), Some(run.id()));
    assert!(session_state.active_prompt().is_none());
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should remain queryable")
            .state(),
        ProviderRunState::Running
    );

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
}

#[test]
fn shared_opencode_tool_activity_keeps_prompt_alive_until_explicit_idle_after_followup_output() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(1_000));
    mock_server.set_emit_tool_call_before_completion(true);
    let previous_bin = env::var_os("ARROBA_OPENCODE_BIN");
    let previous_port = env::var_os("ARROBA_OPENCODE_PORT");
    env::remove_var("ARROBA_OPENCODE_BIN");
    env::remove_var("ARROBA_OPENCODE_PORT");
    let endpoint = format!("http://127.0.0.1:{}", mock_server.port());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = app
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let run = app
        .launch_provider(
            LaunchProviderRequest::new(session.id(), "opencode", "opencode", "default", "default")
                .with_structured_endpoint(endpoint),
        )
        .expect("provider run should launch");

    let _ = arroba_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "tool activity should keep prompt alive\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let interim = collect_provider_records_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |records, _app| {
            records
                .iter()
                .any(|record| record.kind == TerminalOutputKind::ProviderTool)
        },
    );
    assert!(
        interim
            .iter()
            .any(|record| record.kind == TerminalOutputKind::ProviderTool),
        "mock tool activity should be observed"
    );

    let session_after_tool = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist after tool activity");
    assert!(
        session_after_tool.active_prompt().is_some(),
        "tool activity must keep the active prompt alive"
    );

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let records_after_tool_only_completion =
        arroba_kernel::transport::TransportService::pump_provider_output(
            &mut app,
            session.id(),
            run.id(),
            recipients,
        )
        .expect("pump before followup output should succeed");
    let output_after_tool_only_completion =
        render_terminal_output(&records_after_tool_only_completion);
    let session_after_tool_only_completion = app
        .sessions()
        .get_session(session.id())
        .expect("session should still exist after tool-only completion");
    let saw_followup_output = output_after_tool_only_completion
        .contains("fixture response: tool activity should keep prompt alive");
    if !saw_followup_output {
        assert!(
            session_after_tool_only_completion.active_prompt().is_some(),
            "tool-call-only completion must not settle the prompt before OpenCode reports idle"
        );
    }

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = if saw_followup_output {
        collect_provider_output_until(
            &mut app,
            session.id(),
            run.id(),
            recipients,
            |_output, app| {
                app.sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
            },
        )
    } else {
        collect_provider_output_until(
            &mut app,
            session.id(),
            run.id(),
            recipients,
            |output, app| {
                output.contains("fixture response: tool activity should keep prompt alive")
                    && app
                        .sessions()
                        .get_session(session.id())
                        .expect("session should still exist")
                        .active_prompt()
                        .is_none()
            },
        )
    };
    let combined_output = format!("{output_after_tool_only_completion}{output}");
    assert!(combined_output.contains("fixture response: tool activity should keep prompt alive"));

    if let Some(previous_bin) = previous_bin {
        env::set_var("ARROBA_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("ARROBA_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("ARROBA_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("ARROBA_OPENCODE_PORT");
    }
    mock_server.stop();
}
