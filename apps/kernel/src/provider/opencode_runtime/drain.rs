use std::sync::mpsc::TryRecvError;

use crate::error::DaemonError;
use crate::provider::run_actor::ProviderNativeInteractionBridge;
use crate::terminal::TerminalOutputKind;

use super::parts::{handle_message_part_delta, handle_message_part_updated};
use super::permission::handle_permission_request;
use super::snapshot::{
    collect_new_completed_assistant_messages, latest_assistant_usage_tokens,
    opencode_message_completes_active_prompt, opencode_messages_active_prompt_failure,
    opencode_messages_complete_active_prompt, opencode_messages_have_empty_active_assistant,
    record_snapshot_message_metadata, render_snapshot_output_chunks,
};
use super::state::OpenCodeEventDrainResult;
use super::transcript::{format_session_status, render_session_error_transcript_update};
use super::{OpenCodeAssistantCompletion, OpenCodeOutputChunk, OpenCodeRuntimeState};
use crate::provider::{OpenCodeClient, OpenCodeEvent, RuntimeProviderRun};

const OPENCODE_EVENT_RESUBSCRIBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const OPENCODE_EVENT_RESUBSCRIBE_RETRY_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);
const OPENCODE_EVENT_DRAIN_MAX_EVENTS: usize = 256;
const OPENCODE_EMPTY_IDLE_ASSISTANT_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
const OPENCODE_EMPTY_IDLE_ASSISTANT_FAILURE: &str =
    "OpenCode became idle without producing assistant output. Chariox closed this turn so the agent can be retried with a fresh provider session.";

pub(in crate::provider) fn drain_opencode_events(
    run: &RuntimeProviderRun,
    state: &mut OpenCodeRuntimeState,
    native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
) -> Result<OpenCodeEventDrainResult, DaemonError> {
    let provider_run_id = run.id();
    let mut chunks = Vec::new();
    let mut completions = Vec::new();
    let mut prompt_completed = false;
    let mut terminal_failure = None;
    let mut notices = Vec::new();
    let mut resolved_model = None;
    let mut resolved_model_source = None;
    let mut resolved_variant = None;
    let mut resolved_usage_tokens_total = None;
    let drain_active_user_message_id = state.active_user_message_id.clone();

    for _ in 0..OPENCODE_EVENT_DRAIN_MAX_EVENTS {
        match state.event_subscription.receiver.try_recv() {
            Ok(OpenCodeEvent::MessageUpdated { info }) => {
                if info.session_id != state.session_id {
                    continue;
                }
                if resolved_model.is_none() {
                    resolved_model = info.resolved_model();
                    if resolved_model.is_some() {
                        resolved_model_source = Some("message.updated");
                    }
                }
                if resolved_variant.is_none() {
                    resolved_variant = info.resolved_variant();
                }
                if info.role == "assistant" {
                    let total_tokens = info.total_tokens();
                    if total_tokens > 0 {
                        resolved_usage_tokens_total = Some(total_tokens);
                    }
                }
                state
                    .message_roles
                    .insert(info.id.clone(), info.role.clone());
                state
                    .message_parent_ids
                    .insert(info.id.clone(), info.parent_id.clone());
                if info.session_id == state.session_id
                    && info.role == "assistant"
                    && info.parent_id.as_deref() == drain_active_user_message_id.as_deref()
                {
                    if let Some(message) = info.terminal_error_message() {
                        record_terminal_failure(
                            state,
                            message,
                            &mut chunks,
                            &mut completions,
                            &mut notices,
                            &mut terminal_failure,
                            &mut prompt_completed,
                            drain_active_user_message_id.as_deref(),
                        );
                        continue;
                    }
                }
                if info.session_id == state.session_id
                    && info.role == "assistant"
                    && state.message_belongs_to_active_prompt(&info.id)
                    && info.time.completed.is_some()
                    && info.is_terminal_assistant_completion()
                    && state
                        .completed_assistant_message_ids
                        .insert(info.id.clone())
                {
                    if state.active_user_message_id.is_some() {
                        state.active_terminal_assistant_message_id = Some(info.id.clone());
                    }
                    completions.push(OpenCodeAssistantCompletion {
                        message_id: info.id.clone(),
                        completed_at_ms: info.time.completed.unwrap_or_default(),
                    });
                }
                if opencode_message_completes_active_prompt(state, &info) {
                    let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
                    if let Ok(messages) = client.messages(&state.session_id) {
                        if let Some(total_tokens) = latest_assistant_usage_tokens(&messages) {
                            resolved_usage_tokens_total = Some(total_tokens);
                        }
                        record_snapshot_message_metadata(state, &messages);
                        chunks.extend(
                            render_snapshot_output_chunks(
                                state,
                                run.remote_extension_manifest(),
                                &messages,
                            )
                            .chunks,
                        );
                        let snapshot_completions =
                            collect_new_completed_assistant_messages(state, &messages);
                        completions.extend(snapshot_completions);
                    }
                }
            }
            Ok(OpenCodeEvent::MessagePartDelta {
                session_id,
                message_id,
                part_id,
                field,
                delta,
                ..
            }) => {
                handle_message_part_delta(
                    state,
                    provider_run_id,
                    session_id,
                    message_id,
                    part_id,
                    field,
                    delta,
                    &mut chunks,
                )?;
            }
            Ok(OpenCodeEvent::MessagePartUpdated { part }) => {
                handle_message_part_updated(
                    state,
                    provider_run_id,
                    run.remote_extension_manifest(),
                    *part,
                    &mut chunks,
                )?;
            }
            Ok(OpenCodeEvent::SessionError {
                session_id,
                message,
            }) => {
                if session_id == state.session_id {
                    record_terminal_failure(
                        state,
                        message,
                        &mut chunks,
                        &mut completions,
                        &mut notices,
                        &mut terminal_failure,
                        &mut prompt_completed,
                        drain_active_user_message_id.as_deref(),
                    );
                }
            }
            Ok(OpenCodeEvent::SessionStatus { session_id, kind }) => {
                if session_id == state.session_id {
                    if state.last_status_kind.as_deref() != Some(kind.as_str()) {
                        state.last_status_kind = Some(kind.clone());
                        chunks.push(OpenCodeOutputChunk {
                            kind: TerminalOutputKind::ProviderStatus,
                            merge_key: Some("__provider_status__".to_string()),
                            bytes: format_session_status(&kind).into_bytes(),
                        });
                    }
                    if state.active_user_message_id.is_some() {
                        let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
                        if let Ok(messages) = client.messages(&state.session_id) {
                            if let Some(total_tokens) = latest_assistant_usage_tokens(&messages) {
                                resolved_usage_tokens_total = Some(total_tokens);
                            }
                            record_snapshot_message_metadata(state, &messages);
                            let snapshot_chunks = render_snapshot_output_chunks(
                                state,
                                run.remote_extension_manifest(),
                                &messages,
                            );
                            chunks.extend(snapshot_chunks.chunks);
                            if let Some(message) = opencode_messages_active_prompt_failure(
                                state,
                                &messages,
                                drain_active_user_message_id.as_deref(),
                            ) {
                                record_terminal_failure(
                                    state,
                                    message,
                                    &mut chunks,
                                    &mut completions,
                                    &mut notices,
                                    &mut terminal_failure,
                                    &mut prompt_completed,
                                    drain_active_user_message_id.as_deref(),
                                );
                            } else {
                                let status_completions =
                                    collect_new_completed_assistant_messages(state, &messages);
                                if !status_completions.is_empty() {
                                    completions.extend(status_completions);
                                }
                                if kind == "idle"
                                    && opencode_messages_complete_active_prompt(state, &messages)
                                {
                                    prompt_completed = true;
                                    state.active_terminal_assistant_message_id = None;
                                    state.active_user_message_id = None;
                                }
                            }
                        }
                    }
                    if !prompt_completed
                        && kind == "idle"
                        && state.active_terminal_assistant_message_id.is_some()
                    {
                        prompt_completed = true;
                        state.active_terminal_assistant_message_id = None;
                        state.active_user_message_id = None;
                    }
                }
            }
            Ok(OpenCodeEvent::PermissionAsked { request }) => {
                if request.session_id != state.session_id {
                    continue;
                }
                handle_permission_request(
                    run,
                    state,
                    provider_run_id,
                    native_interaction_bridge.clone(),
                    &request,
                )?;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
                state.event_subscription = client.subscribe_events_with_retry(
                    OPENCODE_EVENT_RESUBSCRIBE_TIMEOUT,
                    OPENCODE_EVENT_RESUBSCRIBE_RETRY_INTERVAL,
                )?;
                if let Ok(snapshot) = client.snapshot(&state.session_id) {
                    if resolved_model.is_none() {
                        resolved_model = snapshot
                            .messages
                            .iter()
                            .rev()
                            .find_map(|message| message.info.resolved_model());
                        if resolved_model.is_some() {
                            resolved_model_source = Some("snapshot");
                        }
                    }
                    if resolved_variant.is_none() {
                        resolved_variant = snapshot
                            .messages
                            .iter()
                            .rev()
                            .find_map(|message| message.info.resolved_variant());
                    }
                    if let Some(total_tokens) = latest_assistant_usage_tokens(&snapshot.messages) {
                        resolved_usage_tokens_total = Some(total_tokens);
                    }
                    record_snapshot_message_metadata(state, &snapshot.messages);
                    let snapshot_chunks = render_snapshot_output_chunks(
                        state,
                        run.remote_extension_manifest(),
                        &snapshot.messages,
                    );
                    chunks.extend(snapshot_chunks.chunks);
                    if state.last_status_kind.as_deref() != Some(snapshot.status.as_str()) {
                        state.last_status_kind = Some(snapshot.status.clone());
                        chunks.push(OpenCodeOutputChunk {
                            kind: TerminalOutputKind::ProviderStatus,
                            merge_key: Some("__provider_status__".to_string()),
                            bytes: format_session_status(&snapshot.status).into_bytes(),
                        });
                    }
                    let snapshot_failure = opencode_messages_active_prompt_failure(
                        state,
                        &snapshot.messages,
                        drain_active_user_message_id.as_deref(),
                    );
                    if let Some(message) = snapshot_failure {
                        record_terminal_failure(
                            state,
                            message,
                            &mut chunks,
                            &mut completions,
                            &mut notices,
                            &mut terminal_failure,
                            &mut prompt_completed,
                            drain_active_user_message_id.as_deref(),
                        );
                    } else {
                        let snapshot_completions =
                            collect_new_completed_assistant_messages(state, &snapshot.messages);
                        if !snapshot_completions.is_empty() {
                            completions.extend(snapshot_completions);
                        }
                        if snapshot.status == "idle"
                            && opencode_messages_complete_active_prompt(state, &snapshot.messages)
                        {
                            prompt_completed = true;
                            state.active_terminal_assistant_message_id = None;
                            state.active_user_message_id = None;
                        } else if snapshot.status == "idle"
                            && state.active_terminal_assistant_message_id.is_some()
                        {
                            prompt_completed = true;
                            state.active_terminal_assistant_message_id = None;
                            state.active_user_message_id = None;
                        }
                    }
                }
            }
        }
    }

    if state.active_user_message_id.is_some() && !prompt_completed {
        let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
        if let Ok(status) = client.session_status(&state.session_id) {
            if status != "idle" {
                if state.last_status_kind.as_deref() != Some(status.as_str()) {
                    state.last_status_kind = Some(status.clone());
                    chunks.push(OpenCodeOutputChunk {
                        kind: TerminalOutputKind::ProviderStatus,
                        merge_key: Some("__provider_status__".to_string()),
                        bytes: format_session_status(&status).into_bytes(),
                    });
                }
            } else {
                let mut completion_confirmed = state.active_terminal_assistant_message_id.is_some();
                let mut empty_active_assistant = false;
                if let Ok(messages) = client.messages(&state.session_id) {
                    empty_active_assistant =
                        opencode_messages_have_empty_active_assistant(state, &messages);
                    completion_confirmed |=
                        opencode_messages_complete_active_prompt(state, &messages);
                    if resolved_model.is_none() {
                        resolved_model = messages
                            .iter()
                            .rev()
                            .find_map(|message| message.info.resolved_model());
                        if resolved_model.is_some() {
                            resolved_model_source = Some("snapshot");
                        }
                    }
                    if resolved_variant.is_none() {
                        resolved_variant = messages
                            .iter()
                            .rev()
                            .find_map(|message| message.info.resolved_variant());
                    }
                    if let Some(total_tokens) = latest_assistant_usage_tokens(&messages) {
                        resolved_usage_tokens_total = Some(total_tokens);
                    }
                    record_snapshot_message_metadata(state, &messages);
                    chunks.extend(
                        render_snapshot_output_chunks(
                            state,
                            run.remote_extension_manifest(),
                            &messages,
                        )
                        .chunks,
                    );
                    if let Some(message) = opencode_messages_active_prompt_failure(
                        state,
                        &messages,
                        drain_active_user_message_id.as_deref(),
                    ) {
                        record_terminal_failure(
                            state,
                            message,
                            &mut chunks,
                            &mut completions,
                            &mut notices,
                            &mut terminal_failure,
                            &mut prompt_completed,
                            drain_active_user_message_id.as_deref(),
                        );
                    } else {
                        completions
                            .extend(collect_new_completed_assistant_messages(state, &messages));
                        completion_confirmed |=
                            state.active_terminal_assistant_message_id.is_some();
                    }
                }
                if !prompt_completed
                    && !completion_confirmed
                    && empty_active_assistant
                    && state.active_prompt_has_elapsed(OPENCODE_EMPTY_IDLE_ASSISTANT_GRACE)
                {
                    record_terminal_failure(
                        state,
                        OPENCODE_EMPTY_IDLE_ASSISTANT_FAILURE.to_string(),
                        &mut chunks,
                        &mut completions,
                        &mut notices,
                        &mut terminal_failure,
                        &mut prompt_completed,
                        drain_active_user_message_id.as_deref(),
                    );
                }
                if !prompt_completed && completion_confirmed {
                    if state.last_status_kind.as_deref() != Some(status.as_str()) {
                        state.last_status_kind = Some(status.clone());
                        chunks.push(OpenCodeOutputChunk {
                            kind: TerminalOutputKind::ProviderStatus,
                            merge_key: Some("__provider_status__".to_string()),
                            bytes: format_session_status(&status).into_bytes(),
                        });
                    }
                    prompt_completed = true;
                    state.active_terminal_assistant_message_id = None;
                    state.active_user_message_id = None;
                }
            }
        }
    }

    Ok(OpenCodeEventDrainResult {
        chunks,
        completions,
        prompt_completed,
        terminal_failure,
        notices,
        resolved_model,
        resolved_model_source,
        resolved_variant,
        resolved_usage_tokens_total,
    })
}

fn record_terminal_failure(
    state: &mut OpenCodeRuntimeState,
    message: String,
    chunks: &mut Vec<OpenCodeOutputChunk>,
    completions: &mut Vec<OpenCodeAssistantCompletion>,
    notices: &mut Vec<String>,
    terminal_failure: &mut Option<String>,
    prompt_completed: &mut bool,
    drain_active_user_message_id: Option<&str>,
) {
    if terminal_failure.is_some() || drain_active_user_message_id.is_none() {
        return;
    }
    completions.clear();
    chunks.push(OpenCodeOutputChunk {
        kind: TerminalOutputKind::ProviderError,
        merge_key: None,
        bytes: render_session_error_transcript_update(&message).into_bytes(),
    });
    notices.push(message.clone());
    *terminal_failure = Some(message);
    *prompt_completed = true;
    state.active_terminal_assistant_message_id = None;
    state.active_user_message_id = None;
}
