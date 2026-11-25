use super::*;

// A run ended in the middle of a compaction owes its history row a settlement: the run's actor
// queues it while terminating, and the pump of the ended run must still apply it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_ended_in_the_middle_of_a_compaction_settles_its_history_row() {
    use crate::provider::{AgentEndpointMode, ContextCompaction};
    let worktree = crate::test_support::TestWorktree::new("ended-run-compaction");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "compaction-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id());
    let mut run = crate::provider::RuntimeProviderRun::new(
        "ended-compaction-run",
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "ended-compaction-run".into(),
            pty_target: None,
            pty_program: Some("/usr/bin/python3".into()),
            pty_args: vec![
                "-c".into(),
                r#"
import json, sys, time
print(json.dumps({'type': 'system', 'subtype': 'status', 'status': 'compacting', 'compact_output_chars': 0}), flush=True)
sys.stdin.read()
"#
                .into(),
            ],
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: Some("stdio://claude".into()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    let binding = crate::provider::initialize_claude_runtime(&run).unwrap();
    app.providers()
        .apply_runtime_binding(
            run.id(),
            crate::provider::ProviderRuntimeBinding::Claude(binding),
        )
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let compaction_rows = |records: &[crate::terminal::TerminalOutputRecord]| {
        records
            .iter()
            .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ContextCompacted)
            .map(|record| {
                (
                    record.merge_key.clone().unwrap_or_default(),
                    serde_json::from_slice::<ContextCompaction>(&record.bytes).unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };

    // The compaction starts and its running row reaches the pump.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let records = runtime
            .pump_owned_provider_output(session.id(), run.id(), vec![attachment.id().into()], true)
            .await
            .unwrap();
        if compaction_rows(&records)
            .iter()
            .any(|(_, row)| matches!(row, ContextCompaction::Running { .. }))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the compaction's running row should reach the pump"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    // The run is ended while it is still compacting; its pump then runs once more.
    let ended = app
        .lock()
        .await
        .providers_mut()
        .terminate_run_provider_only(session.id(), run.id())
        .unwrap()
        .into_run();
    app.lock().await.update_provider_run_projection(ended);
    let records = runtime
        .pump_owned_provider_output(session.id(), run.id(), vec![attachment.id().into()], true)
        .await
        .unwrap();

    let rows = compaction_rows(&records);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, "claude:ended-compaction-run:compaction:0");
    assert!(
        matches!(rows[0].1, ContextCompaction::Failed { .. }),
        "{rows:?}"
    );
    let history = app
        .lock()
        .await
        .load_session_history_entries(&session, Some(agent.id()))
        .unwrap();
    let last = history
        .iter()
        .rfind(|entry| {
            entry.merge_key.as_deref() == Some("claude:ended-compaction-run:compaction:0")
        })
        .expect("the compaction row is in history");
    assert!(
        matches!(
            serde_json::from_str::<ContextCompaction>(&last.text).unwrap(),
            ContextCompaction::Failed { .. }
        ),
        "history ends with the failure, not the running row: {}",
        last.text
    );
}
