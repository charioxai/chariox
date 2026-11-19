use super::*;
use crate::provider::ContextCompaction;

#[test]
fn native_stop_failure_settles_only_its_compaction_row() {
    let worktree = crate::test_support::TestWorktree::new("claude-native-stop-failure");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "native-compaction-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let root = crate::test_temp_dir::TestTempDir::new("claude-native-stop-failure");
    let context_file = root.path().join("context.txt");
    let events_file = root.path().join("events.jsonl");
    let mut run = startup_readiness_run(
        session.id(),
        agent.id(),
        "run-native-stop-failure",
        &context_file,
        &events_file,
        "cat >/dev/null".to_string(),
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());

    let process_event = |app: &mut DaemonApp, event: &str| {
        fs::write(&events_file, event).unwrap();
        ProviderOutputClaudeNativeBridge::new(app)
            .process(session.id(), run.id(), &run, None)
            .unwrap()
    };
    ProviderOutputFanout::new(&app).fan_out(
        session.id(),
        run.id(),
        TerminalOutputKind::ProviderOutput,
        None,
        vec![attachment.id().to_string()],
        b"An earlier reply.",
    );

    // These hooks came from Claude Code 2.1.327 running /compact in a PTY.
    let (pre_compact, stop_failure) = include_str!("fixtures/native-stopfailure-hooks.jsonl")
        .trim()
        .split_once('\n')
        .unwrap();
    process_event(&mut app, pre_compact);
    let before = app.terminal().output_records();
    let running = before.last().unwrap();
    assert_eq!(running.kind, TerminalOutputKind::ContextCompacted);
    assert_eq!(
        serde_json::from_slice::<Value>(&running.bytes).unwrap()["state"],
        "running"
    );

    let unrelated_stop = stop_failure.replace("fixture-prompt", "earlier-prompt");
    process_event(&mut app, &unrelated_stop);
    assert_eq!(app.terminal().output_records(), before);

    let outcome = process_event(&mut app, stop_failure);
    assert!(outcome.terminal_failure.is_some());
    let after = app.terminal().output_records();
    assert_eq!(
        after.len(),
        before.len() + 1,
        "StopFailure must replace the running compaction in the transcript with one failed update"
    );
    assert_eq!(&after[..before.len()], before);
    let failed = after.last().unwrap();
    assert_eq!(failed.kind, TerminalOutputKind::ContextCompacted);
    assert_eq!(failed.merge_key, running.merge_key);
    assert_eq!(
        failed.bytes,
        ContextCompaction::Failed {
            reason: "Claude encountered an error before it finished compacting context.".into()
        }
        .to_text()
        .as_bytes()
    );

    process_event(&mut app, stop_failure);
    assert_eq!(app.terminal().output_records(), after, "duplicate failure");

    process_event(
        &mut app,
        &pre_compact.replace("fixture-prompt", "next-prompt"),
    );
    let later_running = app.terminal().output_records().pop().unwrap();
    assert_ne!(later_running.merge_key, failed.merge_key);
    let transcript_path = root.path().join("session.jsonl");
    fs::write(
        &transcript_path,
        serde_json::json!({
            "type": "user", "uuid": "later-summary", "isCompactSummary": true,
            "message": {"role": "user", "content": "A later summary."}
        })
        .to_string(),
    )
    .unwrap();
    process_event(
        &mut app,
        &serde_json::json!({
            "hook_event_name": "PostCompact",
            "prompt_id": "next-prompt",
            "transcript_path": transcript_path,
            "compact_summary": "A later summary."
        })
        .to_string(),
    );
    let finished = app.terminal().output_records();
    assert_eq!(&finished[..after.len()], after);
    assert_eq!(finished.len(), after.len() + 2);
    let done = finished.last().unwrap();
    assert_eq!(done.merge_key, later_running.merge_key);
    assert_eq!(
        done.bytes,
        ContextCompaction::Done {
            summary: "A later summary.".into()
        }
        .to_text()
        .as_bytes()
    );

    process_event(
        &mut app,
        &stop_failure.replace("fixture-prompt", "next-prompt"),
    );
    assert_eq!(app.terminal().output_records(), finished);
}
