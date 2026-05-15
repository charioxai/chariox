use std::thread::sleep;
use std::time::Duration;
use std::time::Instant;

use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::session::PromptAttachment;
use crate::terminal::TerminalOutputKind;

use super::{
    codex_client::codex_endpoint_is_healthy, AgentEndpointMode, CodexClient, CodexNotification,
    CodexRunSelection, ProviderNativeInteractionBridge, ProviderResumeState, RuntimeProviderRun,
};

mod events;
mod input;
mod run_config;
mod state;
mod transcript;
mod turn;

use events::{apply_notification, backfill_external_completed_turn};
use input::codex_input;
use run_config::{codex_client_for_run, normalize_codex_model, normalize_variant};
pub use state::{
    CodexAssistantCompletion, CodexOutputChunk, CodexPollResult, CodexRuntimeBinding,
    CodexRuntimeState,
};
use turn::maybe_finalize_terminal_signal;

#[cfg(test)]
use transcript::{render_codex_tool_transcript_update, CodexToolTranscriptState};
#[cfg(test)]
use turn::CodexTurnTracker;

const CODEX_EVENT_DRAIN_READ_TIMEOUT: Duration = Duration::from_millis(1);
const CODEX_EVENT_DRAIN_MAX_LIVE_NOTIFICATIONS: usize = 64;
const CODEX_UTILITY_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub fn initialize_codex_runtime(
    run: &RuntimeProviderRun,
) -> Result<CodexRuntimeBinding, DaemonError> {
    let endpoint = run
        .structured_endpoint()
        .ok_or_else(|| DaemonError::ProviderProtocol {
            provider_run_id: run.id().to_string(),
            operation: "codex_endpoint_missing",
            message: "codex run did not expose a structured endpoint".to_string(),
        })?
        .to_string();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !codex_endpoint_is_healthy(&endpoint) {
        if Instant::now() >= deadline {
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: run.id().to_string(),
                operation: "codex_endpoint_unhealthy",
                message: format!(
                    "timed out waiting for Codex app-server to become healthy at `{endpoint}`"
                ),
            });
        }
        sleep(Duration::from_millis(100));
    }
    let client = codex_client_for_run(run, &endpoint, None)?;
    let mut socket = client.connect_initialized()?;
    let mut next_request_id = 1;
    let cwd = run
        .working_directory()
        .map(|path| path.to_string_lossy().to_string());
    let model = normalize_codex_model(run.model());
    let resumable_thread_id = run.resume_state().codex_thread_id().map(str::to_string);
    let (thread_id, selection) = match resumable_thread_id {
        Some(thread_id) if run.endpoint_mode() == AgentEndpointMode::External => {
            crate::logging::info_with_fields(
                "daemon.provider.codex",
                "binding native codex thread without resume",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "thread_id": thread_id,
                }),
            );
            (
                thread_id,
                CodexRunSelection {
                    model: Some(format!("codex/{}", run.model())),
                    variant: run.variant().map(str::to_string),
                },
            )
        }
        Some(thread_id) => {
            crate::logging::info_with_fields(
                "daemon.provider.codex",
                "reusing codex thread",
                serde_json::json!({
                    "provider_run_id": run.id(),
                    "thread_id": thread_id,
                }),
            );
            let resume = client.thread_resume(
                &mut socket,
                &mut next_request_id,
                &thread_id,
                cwd.as_deref(),
                model.as_deref(),
                run.write_access_mode(),
                run.execution_mode(),
                run.permission_level(),
            );
            match resume {
                Ok(thread) => (
                    thread.thread.id,
                    CodexRunSelection {
                        model: Some(format!("codex/{}", thread.model)),
                        variant: thread.reasoning_effort,
                    },
                ),
                Err(error) => {
                    crate::logging::error_with_fields(
                        "daemon.provider.codex",
                        "codex thread resume failed",
                        serde_json::json!({
                            "provider_run_id": run.id(),
                            "thread_id": thread_id,
                            "error": error.to_string(),
                        }),
                    );
                    return Err(DaemonError::ProviderProtocol {
                        provider_run_id: run.id().to_string(),
                        operation: "codex_thread_resume",
                        message: format!(
                            "Codex could not resume thread `{thread_id}`: {error}. Refusing to start a blank replacement thread."
                        ),
                    });
                }
            }
        }
        None => {
            let thread = client.thread_start(
                &mut socket,
                &mut next_request_id,
                cwd.as_deref(),
                model.as_deref(),
                run.write_access_mode(),
                run.execution_mode(),
                run.permission_level(),
            )?;
            (
                thread.thread.id,
                CodexRunSelection {
                    model: Some(format!("codex/{}", thread.model)),
                    variant: thread.reasoning_effort,
                },
            )
        }
    };
    Ok(CodexRuntimeBinding {
        state: CodexRuntimeState::new(endpoint, thread_id.clone(), socket, next_request_id),
        selection,
        resume_state: ProviderResumeState::from_codex_thread_id(thread_id),
    })
}

pub fn submit_codex_prompt(
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    prompt: &str,
    attachments: &[PromptAttachment],
) -> Result<(), DaemonError> {
    let client = codex_client_for_run(run, state.endpoint(), None)?;
    let cwd = run
        .working_directory()
        .map(|path| path.to_string_lossy().to_string());
    let model = normalize_codex_model(run.model());
    let effort = normalize_variant(run.variant());
    let input = codex_input(prompt, attachments);
    let thread_id = state.thread_id().to_string();
    let response = match client.turn_start(
        &mut state.socket,
        &mut state.next_request_id,
        &thread_id,
        cwd.as_deref(),
        model.as_deref(),
        effort.as_deref(),
        run.write_access_mode(),
        run.execution_mode(),
        run.permission_level(),
        input,
        &mut state.buffered_notifications,
    ) {
        Ok(response) => response,
        Err(error) => {
            state.buffered_notifications.push(CodexNotification::Error {
                message: error.to_string(),
            });
            return Ok(());
        }
    };
    if let Some(turn_id) = codex_turn_id_from_start_response(&response) {
        state.active_turn_id = Some(turn_id);
    }
    state.turn_tracker = CodexTurnTracker::default();
    crate::logging::debug_with_fields(
        "daemon.provider.codex",
        "codex turn start response trace",
        json!({
            "provider_run_id": run.id(),
            "active_turn_id": state.active_turn_id,
            "response": response,
        }),
    );
    Ok(())
}

pub fn run_codex_utility_prompt(
    run: &RuntimeProviderRun,
    prompt: &str,
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

pub fn abort_codex_turn(
    provider_run_id: &str,
    state: &mut CodexRuntimeState,
) -> Result<(), DaemonError> {
    let Some(turn_id) = state.active_turn_id.clone() else {
        return Ok(());
    };
    let thread_id = state.thread_id().to_string();
    let client = CodexClient::new(provider_run_id, state.endpoint())?;
    client.turn_interrupt(
        &mut state.socket,
        &mut state.next_request_id,
        &thread_id,
        &turn_id,
    )
}

pub fn drain_codex_events(
    run: &RuntimeProviderRun,
    state: &mut CodexRuntimeState,
    native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
) -> Result<CodexPollResult, DaemonError> {
    let client = codex_client_for_run(run, state.endpoint(), native_interaction_bridge)?;
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut notices = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut resolved_usage = None;

    for notification in std::mem::take(&mut state.buffered_notifications) {
        apply_notification(
            notification,
            &mut state.active_turn_id,
            &mut state.turn_tracker,
            &mut state.text_items,
            &mut state.tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
    }

    let mut drained_to_quiet = true;
    for _ in 0..CODEX_EVENT_DRAIN_MAX_LIVE_NOTIFICATIONS {
        let Some(notification) =
            client.read_notification(&mut state.socket, CODEX_EVENT_DRAIN_READ_TIMEOUT)?
        else {
            break;
        };
        drained_to_quiet = false;
        apply_notification(
            notification,
            &mut state.active_turn_id,
            &mut state.turn_tracker,
            &mut state.text_items,
            &mut state.tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
    }
    if !drained_to_quiet {
        drained_to_quiet = client
            .read_notification(&mut state.socket, CODEX_EVENT_DRAIN_READ_TIMEOUT)?
            .map(|notification| {
                state.buffered_notifications.push(notification);
            })
            .is_none();
    }
    if run.endpoint_mode() == AgentEndpointMode::External
        && state.active_turn_id.is_some()
        && !state.turn_tracker.has_pending_terminal()
    {
        backfill_external_completed_turn(
            &client,
            state,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        )?;
    }
    if drained_to_quiet {
        maybe_finalize_terminal_signal(
            &mut state.active_turn_id,
            &mut state.turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );
    }

    Ok(CodexPollResult {
        chunks,
        completions,
        prompt_completed,
        terminal_failure,
        notices,
        resolved_usage,
    })
}

fn codex_turn_id_from_start_response(response: &Value) -> Option<String> {
    response
        .get("turn")
        .and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .or_else(|| response.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{json, Value};

    use crate::provider::ProviderRunTokenUsage;
    use crate::terminal::TerminalOutputKind;

    use super::{
        apply_notification, maybe_finalize_terminal_signal, render_codex_tool_transcript_update,
        CodexAssistantCompletion, CodexNotification, CodexOutputChunk, CodexToolTranscriptState,
        CodexTurnTracker,
    };

    fn flush_quiet_terminal_for_test(
        active_turn_id: &mut Option<String>,
        turn_tracker: &mut CodexTurnTracker,
        completions: &mut Vec<CodexAssistantCompletion>,
        notices: &mut Vec<String>,
        prompt_completed: &mut bool,
        terminal_failure: &mut Option<String>,
    ) {
        turn_tracker.force_pending_terminal_quiet_for_tests();
        maybe_finalize_terminal_signal(
            active_turn_id,
            turn_tracker,
            completions,
            notices,
            prompt_completed,
            terminal_failure,
        );
    }

    #[test]
    fn reasoning_and_agent_deltas_preserve_item_merge_keys() {
        let mut active_turn_id = None;
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ReasoningTextDelta {
                item_id: "reason-1".to_string(),
                delta: "thinking".to_string(),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::AgentMessageDelta {
                item_id: "msg-1".to_string(),
                delta: "answer".to_string(),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| (
                    chunk.kind.clone(),
                    chunk.merge_key.clone().unwrap_or_default(),
                    String::from_utf8_lossy(&chunk.bytes).into_owned()
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    TerminalOutputKind::ProviderReasoning,
                    "reason-1".to_string(),
                    "thinking".to_string()
                ),
                (
                    TerminalOutputKind::ProviderOutput,
                    "msg-1".to_string(),
                    "answer".to_string()
                ),
            ]
        );
    }

    #[test]
    fn completed_agent_message_snapshot_is_rendered_without_delta() {
        let mut active_turn_id = None;
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "agentMessage",
                    "id": "msg-1",
                    "text": "final answer",
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert_eq!(
            chunks,
            vec![CodexOutputChunk {
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("msg-1".to_string()),
                bytes: b"final answer".to_vec(),
            }]
        );
        assert!(completions.is_empty());
        assert!(!prompt_completed);
    }

    #[test]
    fn completed_agent_message_snapshot_only_emits_missing_suffix_after_delta() {
        let mut active_turn_id = None;
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::AgentMessageDelta {
                item_id: "msg-1".to_string(),
                delta: "hello".to_string(),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "agentMessage",
                    "id": "msg-1",
                    "text": "hello world",
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert_eq!(
            chunks
                .iter()
                .map(|chunk| (
                    chunk.kind.clone(),
                    chunk.merge_key.clone().unwrap_or_default(),
                    String::from_utf8_lossy(&chunk.bytes).into_owned()
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    TerminalOutputKind::ProviderOutput,
                    "msg-1".to_string(),
                    "hello".to_string()
                ),
                (
                    TerminalOutputKind::ProviderOutput,
                    "msg-1".to_string(),
                    " world".to_string()
                ),
            ]
        );
    }

    #[test]
    fn completed_reasoning_snapshot_is_rendered_without_delta() {
        let mut active_turn_id = None;
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "reasoning",
                    "id": "reason-1",
                    "summary": ["first", "second"],
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert_eq!(
            chunks,
            vec![CodexOutputChunk {
                kind: TerminalOutputKind::ProviderReasoning,
                merge_key: Some("reason-1".to_string()),
                bytes: b"first\nsecond".to_vec(),
            }]
        );
    }

    #[test]
    fn token_usage_notification_is_projected() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::TokenUsageUpdated {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                usage: ProviderRunTokenUsage {
                    total_tokens: Some(42_100),
                    last_tokens: Some(8_900),
                    context_tokens: Some(8_900),
                    context_window: Some(128_000),
                },
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert_eq!(
            resolved_usage,
            Some(ProviderRunTokenUsage {
                total_tokens: Some(42_100),
                last_tokens: Some(8_900),
                context_tokens: Some(8_900),
                context_window: Some(128_000),
            })
        );
        assert!(chunks.is_empty());
        assert!(completions.is_empty());
        assert!(notices.is_empty());
        assert!(!prompt_completed);
        assert!(terminal_failure.is_none());
    }

    #[test]
    fn command_execution_updates_are_rendered_cumulatively() {
        let mut active_turn_id = None;
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ItemStarted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "ls -la",
                    "cwd": "/tmp",
                    "status": "inProgress",
                    "commandActions": [],
                    "aggregatedOutput": null,
                    "exitCode": null,
                    "durationMs": null,
                    "processId": "pty-1",
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::CommandExecutionOutputDelta {
                item_id: "cmd-1".to_string(),
                delta: "alpha\n".to_string(),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::CommandExecutionOutputDelta {
                item_id: "cmd-1".to_string(),
                delta: "beta\n".to_string(),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "ls -la",
                    "cwd": "/tmp",
                    "status": "completed",
                    "commandActions": [],
                    "aggregatedOutput": "alpha\nbeta\n",
                    "exitCode": 0,
                    "durationMs": 42,
                    "processId": "pty-1",
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        let tool_chunks = chunks
            .into_iter()
            .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderTool)
            .collect::<Vec<CodexOutputChunk>>();
        assert_eq!(tool_chunks.len(), 4);

        let second = parse_tool_chunk(&tool_chunks[1]);
        assert_eq!(second["tool"], "bash");
        assert_eq!(second["status"], "running");
        assert_eq!(second["output"], "alpha");

        let third = parse_tool_chunk(&tool_chunks[2]);
        assert_eq!(third["output"], "alpha\nbeta");

        let fourth = parse_tool_chunk(&tool_chunks[3]);
        assert_eq!(fourth["status"], "completed");
        assert_eq!(fourth["output"], "alpha\nbeta");
    }

    #[test]
    fn codex_exec_command_events_render_as_command_execution_tool_updates() {
        let mut active_turn_id = None;
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ExecCommandStarted {
                call_id: "cmd-event-1".to_string(),
                command: json!("/bin/zsh -lc 'pwd'"),
                cwd: Some("/tmp".to_string()),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::ExecCommandOutputDelta {
                call_id: "cmd-event-1".to_string(),
                chunk: "b2sK".to_string(),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::ExecCommandCompleted {
                call_id: "cmd-event-1".to_string(),
                command: json!("/bin/zsh -lc 'pwd'"),
                cwd: Some("/tmp".to_string()),
                output: Some("ok\n".to_string()),
                exit_code: Some(0),
                success: Some(true),
                stderr: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        let tool_chunks = chunks
            .into_iter()
            .filter(|chunk| chunk.kind == TerminalOutputKind::ProviderTool)
            .collect::<Vec<CodexOutputChunk>>();
        assert_eq!(tool_chunks.len(), 3);

        let first = parse_tool_chunk(&tool_chunks[0]);
        assert_eq!(first["tool"], "bash");
        assert_eq!(first["status"], "running");
        assert_eq!(first["input"]["command"], "/bin/zsh -lc 'pwd'");

        let second = parse_tool_chunk(&tool_chunks[1]);
        assert_eq!(second["output"], "ok");

        let third = parse_tool_chunk(&tool_chunks[2]);
        assert_eq!(third["status"], "completed");
        assert_eq!(third["output"], "ok");
        assert!(!prompt_completed);
        assert!(completions.is_empty());
    }

    #[test]
    fn mcp_tool_progress_is_projected_into_tool_text() {
        let rendered = render_codex_tool_transcript_update(&CodexToolTranscriptState {
            item: json!({
                "type": "mcpToolCall",
                "id": "tool-1",
                "server": "arroba-runtime",
                "tool": "validate_workflow_output",
                "status": "inProgress",
                "arguments": { "value": 1 },
                "result": null,
                "error": null,
                "durationMs": null
            }),
            streamed_output: String::new(),
            progress_messages: vec!["checking schema".to_string()],
            last_emitted: None,
        })
        .expect("payload should render");

        let parsed = serde_json::from_str::<Value>(&rendered).expect("payload should deserialize");
        assert_eq!(parsed["tool"], "validate_workflow_output");
        assert_eq!(parsed["title"], "arroba-runtime");
        assert_eq!(parsed["text"], "checking schema");
    }

    #[test]
    fn only_turn_completed_marks_the_prompt_as_complete() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "ls",
                    "status": "completed",
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));

        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "turn-1".to_string(),
                status: "completed".to_string(),
                error_message: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );
        assert!(prompt_completed);
        assert_eq!(active_turn_id, None);
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].message_id, "codex-turn:turn-1");
    }

    #[test]
    fn turn_completion_waits_for_socket_quiet_before_prompt_completion() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "turn-1".to_string(),
                status: "completed".to_string(),
                error_message: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
        assert!(completions.is_empty());

        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );
        assert!(prompt_completed);
        assert_eq!(active_turn_id, None);
    }

    #[test]
    fn terminal_completion_waits_for_late_tool_output_before_prompt_completion() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "turn-1".to_string(),
                status: "completed".to_string(),
                error_message: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
        assert!(completions.is_empty());

        apply_notification(
            CodexNotification::ItemStarted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "echo still running",
                    "status": "inProgress",
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );
        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
        assert!(completions.is_empty());

        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "echo still running",
                    "status": "completed",
                    "aggregatedOutput": "ok",
                    "exitCode": 0,
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
        assert!(completions.is_empty());

        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );

        assert!(prompt_completed);
        assert_eq!(active_turn_id, None);
        assert_eq!(completions.len(), 1);
    }

    #[test]
    fn turn_completion_waits_for_running_command_execution() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ItemStarted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "pnpm test",
                    "status": "inProgress",
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "turn-1".to_string(),
                status: "completed".to_string(),
                error_message: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
        assert!(completions.is_empty());

        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "pnpm test",
                    "status": "completed",
                    "aggregatedOutput": "ok",
                    "exitCode": 0,
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );
        assert!(prompt_completed);
        assert_eq!(active_turn_id, None);
        assert_eq!(completions.len(), 1);

        apply_notification(
            CodexNotification::AgentMessageDelta {
                item_id: "msg-1".to_string(),
                delta: "done".to_string(),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "turn-1".to_string(),
                status: "completed".to_string(),
                error_message: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );

        assert!(prompt_completed);
        assert_eq!(active_turn_id, None);
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].message_id, "codex-turn:turn-1");
    }

    #[test]
    fn stale_turn_completion_before_tool_finish_does_not_settle_after_tool_finishes() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ExecCommandStarted {
                call_id: "cmd-1".to_string(),
                command: json!("pnpm test"),
                cwd: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "stale-turn".to_string(),
                status: "completed".to_string(),
                error_message: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::ExecCommandCompleted {
                call_id: "cmd-1".to_string(),
                command: json!("pnpm test"),
                cwd: None,
                output: Some("ok\n".to_string()),
                exit_code: Some(0),
                success: Some(true),
                stderr: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );

        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
        assert!(completions.is_empty());
    }

    #[test]
    fn completed_assistant_item_after_tools_does_not_infer_prompt_completion() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ExecCommandStarted {
                call_id: "cmd-1".to_string(),
                command: json!("echo ok"),
                cwd: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::ExecCommandCompleted {
                call_id: "cmd-1".to_string(),
                command: json!("echo ok"),
                cwd: None,
                output: Some("ok\n".to_string()),
                exit_code: Some(0),
                success: Some(true),
                stderr: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "agentMessage",
                    "id": "msg-1",
                    "text": "done",
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));

        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );

        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
        assert!(completions.is_empty());
    }

    #[test]
    fn streamed_assistant_content_after_tools_does_not_infer_prompt_completion() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ExecCommandStarted {
                call_id: "cmd-1".to_string(),
                command: json!("echo ok"),
                cwd: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::ExecCommandCompleted {
                call_id: "cmd-1".to_string(),
                command: json!("echo ok"),
                cwd: None,
                output: Some("ok\n".to_string()),
                exit_code: Some(0),
                success: Some(true),
                stderr: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::AgentMessageDelta {
                item_id: "msg-1".to_string(),
                delta: "done".to_string(),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(!prompt_completed);
        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );

        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
        assert!(completions.is_empty());
    }

    #[test]
    fn tool_start_after_assistant_content_still_requires_terminal_completion() {
        let mut active_turn_id = Some("turn-1".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::ItemCompleted {
                item: json!({
                    "type": "agentMessage",
                    "id": "msg-1",
                    "text": "I will inspect that.",
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        apply_notification(
            CodexNotification::ItemStarted {
                item: json!({
                    "type": "commandExecution",
                    "id": "cmd-1",
                    "command": "ls",
                    "status": "inProgress",
                }),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );

        assert!(!prompt_completed);
        assert_eq!(active_turn_id.as_deref(), Some("turn-1"));
        assert!(completions.is_empty());
    }

    #[test]
    fn stale_turn_completion_does_not_complete_prompt() {
        let mut active_turn_id = Some("current-turn".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "stale-turn".to_string(),
                status: "completed".to_string(),
                error_message: None,
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(!prompt_completed);
        assert!(completions.is_empty());
        assert_eq!(active_turn_id.as_deref(), Some("current-turn"));
    }

    #[test]
    fn interrupted_turn_is_treated_as_terminal_cancellation() {
        let mut active_turn_id = Some("turn-2".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "turn-2".to_string(),
                status: "interrupted".to_string(),
                error_message: Some("Aborted".to_string()),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );

        assert!(prompt_completed);
        assert_eq!(active_turn_id, None);
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].message_id, "codex-turn:turn-2");
        assert_eq!(notices, vec!["Aborted".to_string()]);
    }

    #[test]
    fn failed_turn_records_terminal_failure() {
        let mut active_turn_id = Some("turn-3".to_string());
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::TurnCompleted {
                turn_id: "turn-3".to_string(),
                status: "failed".to_string(),
                error_message: Some("model rejected".to_string()),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );
        flush_quiet_terminal_for_test(
            &mut active_turn_id,
            &mut turn_tracker,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
        );

        assert!(prompt_completed);
        assert_eq!(terminal_failure.as_deref(), Some("model rejected"));
        assert_eq!(notices, vec!["model rejected".to_string()]);
    }

    #[test]
    fn error_notification_without_active_turn_records_terminal_failure() {
        let mut active_turn_id = None;
        let mut turn_tracker = CodexTurnTracker::default();
        let mut text_items = BTreeMap::new();
        let mut tool_items = BTreeMap::new();
        let mut chunks = Vec::new();
        let mut completions = Vec::new();
        let mut notices = Vec::new();
        let mut prompt_completed = false;
        let mut terminal_failure = None;
        let mut resolved_usage = None;

        apply_notification(
            CodexNotification::Error {
                message: "unsupported model gpt-5.2-codex".to_string(),
            },
            &mut active_turn_id,
            &mut turn_tracker,
            &mut text_items,
            &mut tool_items,
            &mut chunks,
            &mut completions,
            &mut notices,
            &mut prompt_completed,
            &mut terminal_failure,
            &mut resolved_usage,
        );

        assert!(prompt_completed);
        assert_eq!(
            terminal_failure.as_deref(),
            Some("unsupported model gpt-5.2-codex")
        );
        assert_eq!(notices, vec!["unsupported model gpt-5.2-codex".to_string()]);
    }

    fn parse_tool_chunk(chunk: &CodexOutputChunk) -> Value {
        serde_json::from_slice(&chunk.bytes).expect("tool chunk should be JSON")
    }
}
