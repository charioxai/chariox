//! Codex utility-prompt execution.

use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::error::DaemonError;
use crate::provider::RuntimeProviderRun;
use crate::terminal::TerminalOutputKind;

use super::input::codex_input;
use super::prompt::{abort_codex_turn, codex_turn_id_from_start_response};
use super::run_config::{codex_client_for_run, normalize_codex_model, normalize_variant};
use super::{drain_codex_events, CodexRuntimeState};

const CODEX_UTILITY_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub fn run_codex_utility_prompt(
    run: &RuntimeProviderRun,
    prompt: &str,
    hidden_system_context: &str,
    timeout: Duration,
) -> Result<String, DaemonError> {
    let endpoint = run
        .structured_endpoint()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "codex_utility_endpoint_missing",
            message: "codex utility requires a structured provider endpoint".to_string(),
        })?
        .to_string();
    let client = codex_client_for_run(run, &endpoint, None)?;
    let mut socket = client.connect_initialized()?;
    let mut next_request_id = 1;
    let cwd = run
        .working_directory()
        .map(|path| path.to_string_lossy().to_string());
    let model = normalize_codex_model(run.model());
    let effort = normalize_variant(run.variant());
    let thread = client.thread_start(
        &mut socket,
        &mut next_request_id,
        cwd.as_deref(),
        model.as_deref(),
        run.write_access_mode(),
        run.execution_mode(),
        run.permission_level(),
    )?;
    let mut state = CodexRuntimeState::new(endpoint, thread.thread.id, socket, next_request_id);
    let input = codex_input(prompt, &[]);
    let thread_id = state.thread_id().to_string();
    let response = client.turn_start(
        &mut state.socket,
        &mut state.next_request_id,
        &thread_id,
        cwd.as_deref(),
        model.as_deref(),
        effort.as_deref(),
        run.write_access_mode(),
        run.execution_mode(),
        run.permission_level(),
        hidden_context_for_provider(hidden_system_context),
        input,
        &mut state.buffered_notifications,
    )?;
    if let Some(turn_id) = codex_turn_id_from_start_response(&response) {
        state.active_turn_id = Some(turn_id);
    }

    let deadline = Instant::now() + timeout;
    let mut output = String::new();
    let mut completed = false;
    while Instant::now() < deadline {
        let poll = drain_codex_events(run, &mut state, None)?;
        for chunk in poll.chunks {
            if chunk.kind == TerminalOutputKind::ProviderOutput {
                output.push_str(&String::from_utf8_lossy(&chunk.bytes));
            }
        }
        if let Some(failure) = poll.terminal_failure {
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "codex_utility_failed",
                message: failure,
            });
        }
        if poll.prompt_completed {
            completed = true;
            break;
        }
        sleep(CODEX_UTILITY_POLL_INTERVAL);
    }
    if !completed {
        let _ = abort_codex_turn(run.id(), &mut state);
        return Err(DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "codex_utility_timeout",
            message: format!(
                "codex utility did not complete within {} ms",
                timeout.as_millis()
            ),
        });
    }
    let output = clean_codex_utility_output(&output);
    if output.is_empty() {
        return Err(DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "codex_utility_empty_output",
            message: "codex utility returned no assistant text".to_string(),
        });
    }
    Ok(output)
}

fn hidden_context_for_provider(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn clean_codex_utility_output(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}
