use std::env;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use chariox_kernel::attachment::{AttachRequest, ClientCapabilityLevel};
use chariox_kernel::provider::{LaunchProviderRequest, ProviderRunState};
use chariox_kernel::session::{CreateSessionRequest, PromptStatus, SessionStatus};
use chariox_kernel::{DaemonApp, DaemonConfig};

mod support;
use support::runtime_integration::{
    collect_provider_output_for_agent_until, collect_provider_output_until,
    collect_provider_records_until, create_opencode_fixture_script, opencode_env_guard,
    output_timeout_ms, wait_for_mock_opencode_event_subscription, wait_for_provider_runtime_state,
    MockOpenCodeServer,
};

#[test]
fn end_session_aborts_active_opencode_session_before_cleanup() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("CHARIOX_OPENCODE_BIN");
    let previous_port = env::var_os("CHARIOX_OPENCODE_PORT");
    env::set_var("CHARIOX_OPENCODE_BIN", &fixture_path);
    env::set_var("CHARIOX_OPENCODE_PORT", mock_server.port().to_string());

    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");

    let _attachment = app
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

    let ended = app
        .end_session(session.id())
        .expect("session should end cleanly");

    assert_eq!(ended.status(), SessionStatus::Ended);
    let deadline = Instant::now() + Duration::from_millis(output_timeout_ms().max(4_000));
    while mock_server.abort_count() == 0 {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for run actor to abort OpenCode session during cleanup"
        );
        thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(mock_server.abort_count(), 1);
    assert_eq!(
        app.providers()
            .get_run(run.id())
            .expect("run should remain queryable")
            .state(),
        ProviderRunState::Ended
    );

    if let Some(previous_bin) = previous_bin {
        env::set_var("CHARIOX_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("CHARIOX_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("CHARIOX_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("CHARIOX_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn clearing_runtime_during_slow_opencode_submit_does_not_restore_state() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.set_prompt_async_response_delay(Duration::from_millis(500));
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

    assert!(app
        .providers()
        .structured_runtime_state_bound_for_tests(run.id()));
    let outcome = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "submit cleanup race\n",
        Vec::new(),
    )
    .expect("prompt should start");
    assert!(
        matches!(
            outcome,
            chariox_kernel::session::PromptSubmissionOutcome::Started { .. }
        ),
        "prompt should dispatch immediately: {outcome:?}"
    );
    wait_for_provider_runtime_state(&app, run.id(), false, "submit I/O is in flight");

    app.providers_mut().clear_runtime(run.id());
    thread::sleep(Duration::from_millis(700));

    assert!(!app
        .providers()
        .structured_runtime_state_bound_for_tests(run.id()));

    mock_server.stop();
}

#[test]
fn clearing_runtime_during_slow_opencode_abort_does_not_restore_state() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(1_000));
    mock_server.set_prompt_async_response_delay(Duration::from_millis(100));
    mock_server.set_abort_response_delay(Duration::from_millis(500));
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

    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "abort cleanup race\n",
        Vec::new(),
    )
    .expect("prompt should start");
    wait_for_provider_runtime_state(&app, run.id(), false, "submit I/O is in flight");
    wait_for_provider_runtime_state(&app, run.id(), true, "submit has restored runtime state");
    let cancellation = chariox_kernel::transport::TransportService::cancel_active_prompt(
        &mut app,
        session.id(),
        attachment.id(),
    )
    .expect("active prompt should cancel");
    assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
    wait_for_provider_runtime_state(&app, run.id(), false, "abort I/O is in flight");

    app.providers_mut().clear_runtime(run.id());
    thread::sleep(Duration::from_millis(700));

    assert!(!app
        .providers()
        .structured_runtime_state_bound_for_tests(run.id()));

    mock_server.stop();
}

#[test]
fn clearing_runtime_during_slow_opencode_output_poll_does_not_restore_state() {
    let _guard = opencode_env_guard();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.set_prompt_async_response_delay(Duration::from_millis(100));
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
    app.providers()
        .set_output_poll_delay_for_tests(run.id(), Duration::from_millis(500));

    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "poll cleanup race\n",
        Vec::new(),
    )
    .expect("prompt should start");
    wait_for_provider_runtime_state(&app, run.id(), false, "submit I/O is in flight");
    wait_for_provider_runtime_state(&app, run.id(), true, "submit has restored runtime state");

    let poll_deadline = Instant::now() + Duration::from_millis(output_timeout_ms().max(4_000));
    while app
        .providers()
        .structured_runtime_state_bound_for_tests(run.id())
    {
        let recipients = app.attachments().list_session_attachment_ids(session.id());
        let _ = chariox_kernel::transport::TransportService::pump_provider_output(
            &mut app,
            session.id(),
            run.id(),
            recipients,
        )
        .expect("poll should enqueue once the structured-output backoff expires");
        assert!(
            Instant::now() < poll_deadline,
            "timed out waiting for slow output poll I/O to start"
        );
        thread::sleep(Duration::from_millis(10));
    }

    app.providers_mut().clear_runtime(run.id());
    thread::sleep(Duration::from_millis(700));

    assert!(!app
        .providers()
        .structured_runtime_state_bound_for_tests(run.id()));

    mock_server.stop();
}

#[test]
fn session_error_completes_the_active_prompt_and_advances_the_queue() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    mock_server.fail_next_prompt("fixture prompt failure");
    let previous_bin = env::var_os("CHARIOX_OPENCODE_BIN");
    let previous_port = env::var_os("CHARIOX_OPENCODE_PORT");
    env::set_var("CHARIOX_OPENCODE_BIN", &fixture_path);
    env::set_var("CHARIOX_OPENCODE_PORT", mock_server.port().to_string());

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

    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "first prompt should fail\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "second prompt should run\n",
        Vec::new(),
    )
    .expect("second prompt should queue");

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let agent_id = run
        .agent_instance_id()
        .expect("provider run should be attached to an agent")
        .to_string();
    let output = collect_provider_output_for_agent_until(
        &mut app,
        session.id(),
        &agent_id,
        run.id(),
        recipients,
        |output, app| {
            output.contains("fixture response: second prompt should run")
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .active_prompt()
                    .is_none()
                && app
                    .sessions()
                    .get_session(session.id())
                    .expect("session should still exist")
                    .queued_prompts()
                    .is_empty()
        },
    );

    assert!(output.contains("fixture response: second prompt should run"));
    let replacement_run = app
        .providers()
        .get_run_for_agent(session.id(), &agent_id)
        .expect("queued prompt should use a replacement provider run");
    assert_ne!(replacement_run.id(), run.id());
    assert_eq!(replacement_run.state(), ProviderRunState::Running);
    assert!(app
        .terminal()
        .notice_records()
        .iter()
        .any(|record| record.message.contains("fixture prompt failure")));

    if let Some(previous_bin) = previous_bin {
        env::set_var("CHARIOX_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("CHARIOX_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("CHARIOX_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("CHARIOX_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn cancelling_active_opencode_prompt_waits_for_provider_confirmation_before_advancing_queue() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("CHARIOX_OPENCODE_BIN");
    let previous_port = env::var_os("CHARIOX_OPENCODE_PORT");
    env::set_var("CHARIOX_OPENCODE_BIN", &fixture_path);
    env::set_var("CHARIOX_OPENCODE_PORT", mock_server.port().to_string());

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

    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "first prompt should cancel\n",
        Vec::new(),
    )
    .expect("first prompt should start");
    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "second prompt after cancel\n",
        Vec::new(),
    )
    .expect("second prompt should queue");

    let cancellation = chariox_kernel::transport::TransportService::cancel_active_prompt(
        &mut app,
        session.id(),
        attachment.id(),
    )
    .expect("active prompt should cancel");
    assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
    assert!(cancellation.started_next.is_none());
    assert_eq!(
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
            .active_prompt()
            .expect("active prompt should still exist")
            .status(),
        PromptStatus::Cancelling
    );

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let output = collect_provider_output_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |output, _app| output.contains("fixture response: second prompt after cancel"),
    );
    assert!(output.contains("fixture response: second prompt after cancel"));
    assert_eq!(mock_server.abort_count(), 1);

    if let Some(previous_bin) = previous_bin {
        env::set_var("CHARIOX_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("CHARIOX_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("CHARIOX_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("CHARIOX_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}

#[test]
fn cancelling_active_opencode_prompt_without_queue_clears_the_active_prompt() {
    let _guard = opencode_env_guard();
    let fixture_path = create_opencode_fixture_script();
    let mock_server = MockOpenCodeServer::start(Duration::from_millis(50));
    let previous_bin = env::var_os("CHARIOX_OPENCODE_BIN");
    let previous_port = env::var_os("CHARIOX_OPENCODE_PORT");
    env::set_var("CHARIOX_OPENCODE_BIN", &fixture_path);
    env::set_var("CHARIOX_OPENCODE_PORT", mock_server.port().to_string());

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

    let _ = chariox_kernel::transport::TransportService::schedule_direct_prompt(
        &mut app,
        session.id(),
        attachment.id(),
        "cancel just this prompt\n",
        Vec::new(),
    )
    .expect("prompt should start");

    let cancellation = chariox_kernel::transport::TransportService::cancel_active_prompt(
        &mut app,
        session.id(),
        attachment.id(),
    )
    .expect("active prompt should cancel");
    assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
    assert!(cancellation.started_next.is_none());

    let recipients = app.attachments().list_session_attachment_ids(session.id());
    let _records = collect_provider_records_until(
        &mut app,
        session.id(),
        run.id(),
        recipients,
        |_records, app| {
            mock_server.abort_count() == 1
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
        .expect("session should still exist after cancellation");
    assert!(session_state.active_prompt().is_none());
    assert_eq!(
        session_state.scheduler_state(),
        chariox_kernel::session::SchedulerState::Idle
    );

    if let Some(previous_bin) = previous_bin {
        env::set_var("CHARIOX_OPENCODE_BIN", previous_bin);
    } else {
        env::remove_var("CHARIOX_OPENCODE_BIN");
    }
    if let Some(previous_port) = previous_port {
        env::set_var("CHARIOX_OPENCODE_PORT", previous_port);
    } else {
        env::remove_var("CHARIOX_OPENCODE_PORT");
    }
    mock_server.stop();
    let _ = fs::remove_file(&fixture_path);
}
