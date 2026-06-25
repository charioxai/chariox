use std::thread;
use std::time::Duration;

use crate::error::DaemonError;
use crate::prompt_assembly::PromptEnvelope;
use crate::provider::{
    ProviderAssistantCompletion, ProviderPromptChunk, ProviderPromptSignalBatch,
    ProviderResumeState, RuntimeProviderRun,
};

use super::super::{
    claude_runtime::{abort_claude_turn, drain_claude_events, submit_claude_prompt},
    codex_runtime::CodexPollResult,
    codex_runtime::{abort_codex_turn, drain_codex_events, submit_codex_prompt},
    opencode_binding::{
        abort_opencode_session, submit_opencode_prompt, sync_opencode_run_selection_for_session,
        OpenCodeRunSelection,
    },
    opencode_runtime::drain_opencode_events,
    pi_runtime::{abort_pi_turn, drain_pi_events, submit_pi_prompt},
};
use super::native_interaction::ProviderNativeInteractionBridgeStore;
use super::runtime_slots::ProviderRunRuntimeRegistry;

pub(super) fn execute_submit_command(
    runtime_registry: &ProviderRunRuntimeRegistry,
    run: RuntimeProviderRun,
    envelope: PromptEnvelope,
) -> Result<(), DaemonError> {
    let run_id = run.id().to_string();
    if run.adapter_key() == "dev-stub" && run.provider() == "slow-structured" {
        thread::sleep(Duration::from_millis(750));
        return Ok(());
    }
    if run.adapter_key() == "codex" {
        let (slot, mut state) = runtime_registry.take_codex_runtime(&run_id)?;
        let result = submit_codex_prompt(&run, &mut state, &envelope);
        runtime_registry.restore_codex_runtime_if_live(&run_id, &slot, state);
        return result;
    }
    if run.adapter_key() == "claude" {
        if !run.client_interface().is_arroba() {
            return Ok(());
        }
        let (slot, mut state) = runtime_registry.take_claude_runtime(&run_id)?;
        let result = submit_claude_prompt(&run, &mut state, &envelope);
        runtime_registry.restore_claude_runtime_if_live(&run_id, &slot, state);
        return result;
    }
    if run.adapter_key() == "pi" {
        let (slot, mut state) = runtime_registry.take_pi_runtime(&run_id)?;
        let result = submit_pi_prompt(&run, &mut state, &envelope);
        runtime_registry.restore_pi_runtime_if_live(&run_id, &slot, state);
        return result;
    }
    if run.adapter_key() != "opencode" {
        return Ok(());
    }

    let (slot, mut state) = runtime_registry.take_opencode_runtime(&run_id)?;
    let result = submit_opencode_prompt(&run, &mut state, &envelope);
    runtime_registry.restore_opencode_runtime_if_live(&run_id, &slot, state);
    result
}

pub(super) fn execute_abort_command(
    runtime_registry: &ProviderRunRuntimeRegistry,
    run: RuntimeProviderRun,
) -> Result<(), DaemonError> {
    let run_id = run.id().to_string();
    if run.adapter_key() == "dev-stub" && run.provider() == "slow-structured" {
        thread::sleep(Duration::from_millis(750));
        return Ok(());
    }
    if run.adapter_key() == "codex" {
        let (slot, mut state) = runtime_registry.take_codex_runtime(&run_id)?;
        let result = abort_codex_turn(&run_id, &mut state);
        runtime_registry.restore_codex_runtime_if_live(&run_id, &slot, state);
        return result;
    }
    if run.adapter_key() == "claude" {
        if !run.client_interface().is_arroba() {
            return Ok(());
        }
        let (slot, mut state) = runtime_registry.take_claude_runtime(&run_id)?;
        let result = abort_claude_turn(&run, &mut state);
        runtime_registry.restore_claude_runtime_if_live(&run_id, &slot, state);
        return result;
    }
    if run.adapter_key() == "pi" {
        let (slot, mut state) = runtime_registry.take_pi_runtime(&run_id)?;
        let result = abort_pi_turn(&run, &mut state);
        runtime_registry.restore_pi_runtime_if_live(&run_id, &slot, state);
        return result;
    }
    if run.adapter_key() != "opencode" {
        return Ok(());
    }

    let (slot, state) = runtime_registry.take_opencode_runtime(&run_id)?;
    let result = abort_opencode_session(&run_id, &state);
    runtime_registry.restore_opencode_runtime_if_live(&run_id, &slot, state);
    result
}

pub(super) fn execute_utility_command(
    runtime_registry: &ProviderRunRuntimeRegistry,
    run: RuntimeProviderRun,
    envelope: PromptEnvelope,
    timeout: Duration,
) -> Result<String, DaemonError> {
    let run_id = run.id().to_string();
    if run.adapter_key() != "claude" || !run.client_interface().is_arroba() {
        return Err(DaemonError::LocalTransport {
            operation: "run structured provider utility prompt",
            message: format!(
                "structured utility command is not supported for adapter `{}`",
                run.adapter_key()
            ),
        });
    }
    let (slot, mut state) = runtime_registry.take_claude_runtime(&run_id)?;
    let result = run_claude_utility_prompt_on_runtime(&run, &mut state, &envelope, timeout);
    runtime_registry.restore_claude_runtime_if_live(&run_id, &slot, state);
    result
}

fn run_claude_utility_prompt_on_runtime(
    run: &RuntimeProviderRun,
    state: &mut super::super::ClaudeRuntimeState,
    envelope: &PromptEnvelope,
    timeout: Duration,
) -> Result<String, DaemonError> {
    submit_claude_prompt(run, state, envelope)?;
    let deadline = std::time::Instant::now() + timeout;
    let mut output = String::new();
    while std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        let batch = drain_claude_events(run, state)?;
        for chunk in batch.chunks {
            if chunk.kind == crate::terminal::TerminalOutputKind::ProviderOutput {
                output.push_str(&String::from_utf8_lossy(&chunk.bytes));
            }
        }
        if let Some(error) = batch.terminal_failure {
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "claude_utility_failed",
                message: error,
            });
        }
        if batch.prompt_completed {
            let output = output.trim().to_string();
            if output.is_empty() {
                return Err(DaemonError::ProviderProtocol {
                    provider_run_id: run.id().to_string(),
                    operation: "claude_utility_empty_output",
                    message: "Claude utility returned no assistant text".to_string(),
                });
            }
            return Ok(output);
        }
    }
    Err(DaemonError::ProviderProtocol {
        provider_run_id: run.id().to_string(),
        operation: "claude_utility_timeout",
        message: format!(
            "Claude utility did not complete within {} ms",
            timeout.as_millis()
        ),
    })
}

pub(super) fn execute_terminate_command(
    runtime_registry: &ProviderRunRuntimeRegistry,
    run: RuntimeProviderRun,
) -> Result<(), DaemonError> {
    let run_id = run.id().to_string();
    if run.adapter_key() == "codex" && runtime_registry.runtime_slot_missing_or_empty_codex(&run_id)
    {
        return Ok(());
    }
    if run.adapter_key() == "claude"
        && runtime_registry.runtime_slot_missing_or_empty_claude(&run_id)
    {
        return Ok(());
    }
    if run.adapter_key() == "opencode"
        && runtime_registry.runtime_slot_missing_or_empty_opencode(&run_id)
    {
        return Ok(());
    }
    if run.adapter_key() == "pi" && runtime_registry.runtime_slot_missing_or_empty_pi(&run_id) {
        return Ok(());
    }
    execute_abort_command(runtime_registry, run)
}

pub(super) fn execute_selection_sync_command(
    runtime_registry: &ProviderRunRuntimeRegistry,
    run_id: &str,
    run: &RuntimeProviderRun,
) -> Result<OpenCodeRunSelection, DaemonError> {
    let slot = runtime_registry.opencode_slot(run_id)?;
    let (base_url, session_id) = {
        let guard = slot.lock().expect("opencode runtime slot poisoned");
        let state = guard
            .as_ref()
            .ok_or_else(|| DaemonError::ProviderProtocol {
                provider_run_id: run_id.to_string(),
                operation: "opencode_session_missing",
                message: "no OpenCode session is bound to this provider run".to_string(),
            })?;
        (state.base_url().to_string(), state.session_id().to_string())
    };
    sync_opencode_run_selection_for_session(
        run_id,
        &base_url,
        &session_id,
        run.model(),
        run.variant(),
    )
}

pub(super) fn execute_output_poll_command(
    native_interaction_bridge: &ProviderNativeInteractionBridgeStore,
    runtime_registry: &ProviderRunRuntimeRegistry,
    run: &RuntimeProviderRun,
    output_poll_delay: Duration,
) -> Result<Option<ProviderPromptSignalBatch>, DaemonError> {
    let run_id = run.id();
    if run.adapter_key() == "dev-stub" && run.provider() == "slow-structured" {
        thread::sleep(Duration::from_millis(750));
        return Ok(None);
    }
    if run.adapter_key() == "codex" {
        let (slot, mut state) = match runtime_registry.take_codex_runtime(run_id) {
            Ok((slot, state)) => (slot, state),
            Err(_) => return Ok(None),
        };
        if !output_poll_delay.is_zero() {
            thread::sleep(output_poll_delay);
        }
        let poll = drain_codex_events(run, &mut state, native_interaction_bridge.read());
        let resolved_resume_state =
            completed_codex_turn_resume_state(state.thread_id(), poll.as_ref().ok());
        runtime_registry.restore_codex_runtime_if_live(run_id, &slot, state);
        let poll = poll?;
        crate::logging::debug_with_fields(
            "daemon.provider_run_actor",
            "codex output poll result trace",
            serde_json::json!({
                "provider_run_id": run_id,
                "chunks": poll.chunks.len(),
                "completions": poll.completions.len(),
                "prompt_completed": poll.prompt_completed,
                "terminal_failure": poll.terminal_failure,
                "notices": poll.notices.len(),
            }),
        );
        return Ok(Some(ProviderPromptSignalBatch {
            chunks: poll
                .chunks
                .into_iter()
                .map(|chunk| ProviderPromptChunk {
                    kind: chunk.kind,
                    merge_key: chunk.merge_key,
                    bytes: chunk.bytes,
                })
                .collect(),
            completions: poll
                .completions
                .into_iter()
                .map(|completion| ProviderAssistantCompletion {
                    message_id: completion.message_id,
                    completed_at_ms: completion.completed_at_ms,
                })
                .collect(),
            prompt_completed: poll.prompt_completed,
            terminal_failure: poll.terminal_failure,
            notices: poll.notices,
            resolved_model: None,
            resolved_model_source: None,
            resolved_variant: None,
            resolved_usage_tokens_total: poll.resolved_usage.and_then(|usage| usage.total_tokens),
            resolved_usage: poll.resolved_usage,
            resolved_resume_state,
        }));
    }
    if run.adapter_key() == "claude" {
        let (slot, mut state) = match runtime_registry.take_claude_runtime(run_id) {
            Ok((slot, state)) => (slot, state),
            Err(_) => return Ok(None),
        };
        if !output_poll_delay.is_zero() {
            thread::sleep(output_poll_delay);
        }
        let drain = drain_claude_events(run, &mut state);
        runtime_registry.restore_claude_runtime_if_live(run_id, &slot, state);
        return drain.map(Some);
    }
    if run.adapter_key() == "pi" {
        let (slot, mut state) = match runtime_registry.take_pi_runtime(run_id) {
            Ok((slot, state)) => (slot, state),
            Err(_) => return Ok(None),
        };
        if !output_poll_delay.is_zero() {
            thread::sleep(output_poll_delay);
        }
        let drain = drain_pi_events(run, &mut state);
        runtime_registry.restore_pi_runtime_if_live(run_id, &slot, state);
        return drain.map(Some);
    }
    if run.adapter_key() != "opencode" {
        return Ok(None);
    }
    let (slot, mut state) = match runtime_registry.take_opencode_runtime(run_id) {
        Ok((slot, state)) => (slot, state),
        Err(_) => return Ok(None),
    };
    if !output_poll_delay.is_zero() {
        thread::sleep(output_poll_delay);
    }
    let drain = drain_opencode_events(run, &mut state, native_interaction_bridge.read());
    runtime_registry.restore_opencode_runtime_if_live(run_id, &slot, state);
    let drain = drain?;
    Ok(Some(ProviderPromptSignalBatch {
        chunks: drain
            .chunks
            .into_iter()
            .map(|chunk| ProviderPromptChunk {
                kind: chunk.kind,
                merge_key: chunk.merge_key,
                bytes: chunk.bytes,
            })
            .collect(),
        completions: drain
            .completions
            .into_iter()
            .map(|completion| ProviderAssistantCompletion {
                message_id: completion.message_id,
                completed_at_ms: completion.completed_at_ms,
            })
            .collect(),
        prompt_completed: drain.prompt_completed,
        terminal_failure: drain.terminal_failure,
        notices: drain.notices,
        resolved_model: drain.resolved_model,
        resolved_model_source: drain.resolved_model_source,
        resolved_variant: drain.resolved_variant,
        resolved_usage_tokens_total: drain.resolved_usage_tokens_total,
        resolved_usage: None,
        resolved_resume_state: None,
    }))
}

fn completed_codex_turn_resume_state(
    thread_id: &str,
    poll: Option<&CodexPollResult>,
) -> Option<ProviderResumeState> {
    let poll = poll?;
    if !poll.prompt_completed {
        return None;
    }
    Some(ProviderResumeState::from_codex_thread_id(thread_id))
}

#[cfg(test)]
mod tests {
    use super::super::super::codex_runtime::{CodexAssistantCompletion, CodexPollResult};

    use super::completed_codex_turn_resume_state;

    fn codex_poll(prompt_completed: bool) -> CodexPollResult {
        CodexPollResult {
            chunks: Vec::new(),
            completions: prompt_completed
                .then(|| CodexAssistantCompletion {
                    message_id: "msg-1".to_string(),
                    completed_at_ms: 1,
                })
                .into_iter()
                .collect(),
            prompt_completed,
            terminal_failure: None,
            notices: Vec::new(),
            resolved_usage: None,
        }
    }

    #[test]
    fn codex_resume_state_is_durable_only_after_prompt_completion() {
        assert_eq!(
            completed_codex_turn_resume_state("thread-1", Some(&codex_poll(false)))
                .and_then(|state| state.codex_thread_id().map(str::to_string)),
            None
        );

        assert_eq!(
            completed_codex_turn_resume_state("thread-1", Some(&codex_poll(true)))
                .and_then(|state| state.codex_thread_id().map(str::to_string)),
            Some("thread-1".to_string())
        );
    }
}
