//! Publish a native proposal while Claude is blocked on its permission callback.
use super::*;

pub(crate) fn publish_claude_plan(
    run: &RuntimeProviderRun,
    state: &mut ClaudeRuntimeState,
    tool_use_id: Option<&str>,
    plan: &str,
) -> (ProviderPromptSignalBatch, Result<(), DaemonError>) {
    let mut batch = ProviderPromptSignalBatch::default();
    let result = (|| {
        let invalid = |message: &str| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "claude_publish_plan",
            message: message.to_string(),
        };
        if run.execution_mode() != AgentExecutionMode::Plan
            || state.active_execution_mode != AgentExecutionMode::Plan
            || state.active_turn_id.is_none()
        {
            return Err(invalid(
                "The native plan proposal belongs to an ended planning turn.",
            ));
        }
        // The MCP callback can arrive before the stdout reader has queued its tool
        // invocation. Keep the callback unanswered until that exact root call arrives.
        let deadline = Instant::now() + Duration::from_secs(5);
        let tool_use_id = loop {
            if let Some(id) = state.tool_transcript.plan_proposal_id(tool_use_id) {
                break id;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(invalid(
                    "The native plan invocation was not received; retry the planning turn.",
                ));
            }
            let message = state.output.receiver.recv_timeout(remaining).map_err(|_| {
                invalid("The native plan invocation was not received; retry the planning turn.")
            })?;
            match message {
                ClaudeRuntimeMessage::Stdout(value) => {
                    handle_claude_tool_uses(run.id(), state, &value, &mut batch)?;
                    apply_claude_message(run.id(), state, value, &mut batch);
                }
                ClaudeRuntimeMessage::StdoutParseError(error) => {
                    batch
                        .notices
                        .push(format!("Claude stdout parse warning: {error}"));
                }
                ClaudeRuntimeMessage::Stderr(line) => {
                    apply_claude_stderr(run, state, &line, &mut batch)
                }
            }
            if state.active_turn_id.is_none() {
                return Err(invalid(
                    "The native plan proposal belongs to an ended planning turn.",
                ));
            }
        };
        let payload = json!({
            "tool": "ExitPlanMode", "status": "completed", "id": tool_use_id,
            "input": {"plan": plan},
        });
        batch.chunks.push(super::super::ProviderPromptChunk {
            kind: TerminalOutputKind::ProviderTool,
            merge_key: Some(tool_use_id),
            bytes: serde_json::to_vec(&payload).map_err(|error| invalid(&error.to_string()))?,
        });
        events::complete_claude_turn(state, &mut batch);
        // Never grant ExitPlanMode. Restart kills the process while it is still
        // waiting for permission, before the proposal can reach clients or history.
        if let Err(error) = restart_claude_runtime(run, state, "claude_restart_after_plan_proposal")
        {
            batch.terminal_failure = Some(error.to_string());
            return Err(error);
        }
        Ok(())
    })();
    // A rejected late/nested callback can still have drained valid output. Send
    // that output through the usual history path without failing another turn.
    (batch, result)
}
