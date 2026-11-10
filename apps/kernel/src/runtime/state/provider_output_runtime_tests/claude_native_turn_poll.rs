//! MP-08 / MP-09 / MP-10 / MP-11: Claude's hooks and transcript are files, so a
//! running turn must wake its own pump while the terminal is quiet (an approval,
//! a question or a long tool call), or its cards and rows wait for the next
//! keystroke on a screen nobody types on.
use super::*;
use crate::app::startup_readiness_run;
use std::fs;

#[test]
fn a_running_claude_turn_wakes_its_own_pump_while_the_terminal_is_quiet() {
    let worktree = crate::test_support::TestWorktree::new("claude-native-turn-poll");
    let root = std::env::temp_dir().join(format!(
        "chariox-claude-turn-poll-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    fs::create_dir_all(&root).expect("test root should be created");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon should bootstrap");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-turn-poll",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let context_file = root.join("hidden-context.txt");
    let events_file = root.join("events.jsonl");
    fs::write(&context_file, "").expect("context file should be created");
    fs::write(&events_file, "").expect("events file should be created");
    let mut run = startup_readiness_run(
        session.id(),
        agent.id(),
        "provider-run-turn-poll",
        &context_file,
        &events_file,
        "exec sleep 300".to_string(),
    );
    run.mark_running();
    app.pty_mut()
        .spawn_for_run(&run)
        .expect("test provider PTY should start");
    app.providers_mut().insert_run_for_test(run.clone());
    app.sessions_mut()
        .set_active_provider_run(session.id(), Some(run.id().to_string()))
        .expect("active provider run should be set");
    app.record_native_prompt_started_with_attachments(
        session.id(),
        attachment.id(),
        attachment.id(),
        agent.id(),
        "Run a long command.",
        Vec::new(),
    )
    .expect("active prompt should start");
    let app = Arc::new(Mutex::new(app));
    let _cleanup = InertPtyCleanup {
        app: Arc::clone(&app),
        provider_run_id: run.id().into(),
    };
    let executor = tokio::runtime::Runtime::new().unwrap();
    let runtime = executor.block_on(owned_runtime_state(&app));

    executor
        .block_on(runtime.pump_owned_provider_output(
            session.id(),
            run.id(),
            vec![attachment.id().to_string()],
            false,
        ))
        .expect("the pump should run");

    // The terminal is silent, yet the pump is due again within a second.
    let now_ms = crate::session::unix_epoch_ms();
    let due_at_ms = runtime
        .owned
        .provider_output_deadlines
        .next_due_at_ms()
        .expect("a running turn schedules its next check");
    assert!(
        due_at_ms <= now_ms + 1_000,
        "the next check is {} ms away",
        due_at_ms.saturating_sub(now_ms)
    );
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    assert!(runtime
        .take_ready_provider_output_run_ids(crate::session::unix_epoch_ms())
        .contains(run.id()));
    let _ = fs::remove_dir_all(root);
}
