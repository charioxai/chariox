use super::*;
use crate::provider::{ProviderPromptChunk, ProviderPromptSignalBatch, ProviderRunTermination};
use crate::terminal::TerminalOutputKind;

// MP-08 / MP-09 / MP-10 / MP-11: exercise the common failure entry after the
// provider's invocation has already reached a client and durable history.
async fn process_end_settles_tools(provider: &str, memory_limit: bool) {
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
    let chunk = |id: &str, status: &str| ProviderPromptChunk {
        kind: TerminalOutputKind::ProviderTool,
        merge_key: Some(format!("merge-{id}")),
        bytes: serde_json::to_vec(&serde_json::json!({
            "id": id, "tool": tool, "status": status,
            "input": {"command":"sleep 30"}, "output":"partial output",
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
    runtime
        .owned
        .terminal_stream
        .drain_output_records(session.id(), attachment.id());
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
    assert_eq!(payload["output"], "partial output");
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
    process_end_settles_tools("claude", true).await;
}
#[tokio::test]
async fn claude_outside_end_settles_pending_tool() {
    process_end_settles_tools("claude", false).await;
}
#[tokio::test]
async fn codex_memory_limit_settles_pending_tool() {
    process_end_settles_tools("codex", true).await;
}
#[tokio::test]
async fn codex_outside_end_settles_pending_tool() {
    process_end_settles_tools("codex", false).await;
}
