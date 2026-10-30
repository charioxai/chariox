use super::*;
use crate::provider::{ProviderPromptChunk, ProviderPromptSignalBatch, ProviderRunTermination};
use crate::terminal::TerminalOutputKind;

// MP-08 / MP-09 / MP-10 / MP-11: exercise the common failure entry after the
// provider's invocation has already reached a client and durable history.
async fn process_end_settles_tools(provider: &str, memory_limit: bool, large_output: bool) {
    let worktree = crate::test_support::TestWorktree::new("process-tool-settlement");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "process-tool-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "test",
            )
            .with_agent_id(agent.id()),
        )
        .unwrap();
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "run a command",
        Vec::new(),
    )
    .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let tool = if provider == "claude" {
        "Bash"
    } else {
        "commandExecution"
    };
    let output = if large_output {
        format!(
            "old-leading-output{}latest-diagnostic-tail",
            "x".repeat(20 * 1024)
        )
    } else {
        "partial output".to_string()
    };
    let chunk = |id: &str, status: &str| ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderTool,
        merge_key: Some(format!("merge-{id}")),
        bytes: serde_json::to_vec(&serde_json::json!({
            "id": id, "tool": tool, "status": status,
            "input": {"command":"sleep 30"}, "output":output,
        }))
        .unwrap(),
    };
    runtime
        .apply_owned_structured_output_batch(
            session.id(),
            run.id(),
            vec![attachment.id().into()],
            ProviderPromptSignalBatch {
                chunks: vec![chunk("pending", "running"), chunk("finished", "completed")],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let live = runtime
        .owned
        .terminal_stream
        .drain_output_records(session.id(), attachment.id());
    let live_tool = live
        .iter()
        .find(|r| r.merge_key.as_deref() == Some("merge-pending"))
        .expect("retention must not suppress the live tool update");
    let live_payload: serde_json::Value = serde_json::from_slice(&live_tool.bytes).unwrap();
    if large_output {
        let tail = live_payload["output"].as_str().unwrap();
        assert!(tail.ends_with("latest-diagnostic-tail"));
        assert!(!tail.contains("old-leading-output"));
    }
    let termination = if memory_limit {
        ProviderRunTermination::memory_limit(33554432, crate::session::unix_epoch_ms())
    } else {
        ProviderRunTermination::outside_end(crate::session::unix_epoch_ms())
    };
    runtime
        .fail_owned_provider_prompt_with_termination(
            session.id(),
            run.id(),
            &termination.reason,
            true,
            Some(termination.clone()),
        )
        .await
        .unwrap();
    let records = runtime
        .owned
        .terminal_stream
        .drain_output_records(session.id(), attachment.id());
    let updates = records
        .iter()
        .filter(|r| r.kind == TerminalOutputKind::ProviderTool)
        .collect::<Vec<_>>();
    assert_eq!(
        updates.len(),
        1,
        "{provider}: process end must settle exactly the unfinished tool"
    );
    assert_eq!(updates[0].merge_key.as_deref(), Some("merge-pending"));
    let payload: serde_json::Value = serde_json::from_slice(&updates[0].bytes).unwrap();
    assert_eq!(payload["id"], "pending");
    assert_eq!(payload["tool"], tool);
    assert_eq!(payload["status"], "error");
    assert_eq!(payload["error"], termination.reason);
    assert_eq!(
        payload["output"], live_payload["output"],
        "failure must retain the displayed tail"
    );
    assert_eq!(payload["input"]["command"], "sleep 30");
    runtime.flush_provider_history(run.id()).await.unwrap();
    let entries = runtime
        .owned
        .operational_history_store
        .load_session_history_entries(session.id(), Some(agent.id()))
        .unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry.merge_key.as_deref() == Some("merge-pending")
                && entry.text.contains(&termination.reason)),
        "the terminal update must survive reconnect"
    );
    if large_output {
        let failure = entries
            .iter()
            .find(|entry| {
                entry.merge_key.as_deref() == Some("merge-pending")
                    && entry.text.contains(&termination.reason)
            })
            .expect("failure snapshot must be durable");
        let persisted: serde_json::Value = serde_json::from_str(&failure.text).unwrap();
        assert_eq!(
            persisted["output"], live_payload["output"],
            "replay must retain the displayed tail"
        );
    }
    assert!(runtime
        .owned
        .session_store
        .get_session(session.id())
        .unwrap()
        .active_prompt_for_agent(agent.id())
        .is_none());
}

#[tokio::test]
async fn claude_memory_limit_settles_pending_tool() {
    process_end_settles_tools("claude", true, false).await;
}
#[tokio::test]
async fn claude_outside_end_settles_pending_tool() {
    process_end_settles_tools("claude", false, false).await;
}
#[tokio::test]
async fn codex_memory_limit_settles_pending_tool() {
    process_end_settles_tools("codex", true, false).await;
}
#[tokio::test]
async fn codex_outside_end_settles_pending_tool() {
    process_end_settles_tools("codex", false, false).await;
}

#[tokio::test]
async fn process_tool_settlement_keeps_live_tail_after_memory_limit() {
    process_end_settles_tools("codex", true, true).await;
}

#[tokio::test]
async fn process_tool_settlement_keeps_live_tail_after_outside_end() {
    process_end_settles_tools("claude", false, true).await;
}

// MP-08 / MP-09 / MP-10 / MP-11: retirement may wait for the app while
// another observer sees an ended run with its original prompt still active.
#[tokio::test]
async fn concurrent_process_exit_reports_one_diagnostic() {
    let worktree = crate::test_support::TestWorktree::new("concurrent-process-exit");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "exit-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "test",
            )
            .with_agent_id(agent.id()),
        )
        .unwrap();
    app.submit_prompt(
        session.id(),
        attachment.id(),
        Some(agent.id()),
        "run a command",
        Vec::new(),
    )
    .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let app_guard = app.lock().await;
    let release_app = async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        drop(app_guard);
    };
    let original = ProviderRunTermination::outside_end(crate::session::unix_epoch_ms());
    let repeated = ProviderRunTermination::runtime_failure(
        "provider run was already ended during liveness reconciliation",
        crate::session::unix_epoch_ms(),
    );
    let (first, second, ()) = tokio::join!(
        runtime.settle_unexpected_provider_run_exit(session.id(), run.id(), agent.id(), original),
        runtime.settle_unexpected_provider_run_exit(session.id(), run.id(), agent.id(), repeated),
        release_app,
    );
    first.unwrap();
    second.unwrap();
    let records = runtime
        .owned
        .terminal_stream
        .drain_output_records(session.id(), attachment.id());
    let diagnostics = records
        .iter()
        .filter(|record| {
            record.kind == TerminalOutputKind::ProviderError
                && !record.bytes.starts_with(b"Request not carried out:")
        })
        .count();
    assert_eq!(
        diagnostics, 1,
        "one ended process must produce one diagnostic"
    );
    let history = runtime
        .owned
        .operational_history_store
        .load_session_events(session.id(), Some(agent.id()))
        .unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|event| {
                event.kind == crate::history::HistoryEventKind::ProviderError
                    && !event
                        .content
                        .as_deref()
                        .unwrap_or("")
                        .starts_with("Request not carried out:")
            })
            .count(),
        1,
        "replay must retain only the original exit diagnostic"
    );
    let state = runtime.owned.session_snapshot(session.id()).unwrap();
    let activity = runtime.agent_activity_for_session(&state);
    assert_eq!(
        activity[agent.id()]
            .last_completed_turn
            .as_ref()
            .unwrap()
            .provider_termination
            .as_ref()
            .unwrap()
            .category,
        crate::provider::ProviderRunTerminationCategory::OutsideEnd
    );
}
