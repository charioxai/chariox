//! OpenCode snapshot reconciliation and rendering.

use crate::error::DaemonError;
use crate::provider::{OpenCodeClient, OpenCodeMessage, OpenCodeMessageInfo};
use crate::terminal::TerminalOutputKind;

use super::transcript::render_tool_transcript_update;
use super::{OpenCodeAssistantCompletion, OpenCodeOutputChunk, OpenCodeRuntimeState};

pub(super) struct SnapshotRenderResult {
    pub(super) chunks: Vec<OpenCodeOutputChunk>,
}

pub(super) fn opencode_message_completes_active_prompt(
    state: &OpenCodeRuntimeState,
    info: &OpenCodeMessageInfo,
) -> bool {
    let Some(active_user_message_id) = state.active_user_message_id.as_deref() else {
        return false;
    };
    info.session_id == state.session_id
        && info.role == "assistant"
        && info.parent_id.as_deref() == Some(active_user_message_id)
        && info.is_terminal_assistant_completion()
}

pub(super) fn opencode_messages_complete_active_prompt(
    state: &OpenCodeRuntimeState,
    messages: &[OpenCodeMessage],
) -> bool {
    messages
        .iter()
        .any(|message| opencode_message_completes_active_prompt(state, &message.info))
}

pub(super) fn refresh_opencode_message_metadata(
    state: &mut OpenCodeRuntimeState,
    provider_run_id: &str,
) -> Result<(), DaemonError> {
    let client = OpenCodeClient::new(provider_run_id, &state.base_url)?;
    if let Ok(messages) = client.messages(&state.session_id) {
        record_snapshot_message_metadata(state, &messages);
    }
    Ok(())
}

pub(super) fn record_snapshot_message_metadata(
    state: &mut OpenCodeRuntimeState,
    messages: &[OpenCodeMessage],
) {
    for message in messages {
        state
            .message_roles
            .insert(message.info.id.clone(), message.info.role.clone());
        for part in &message.parts {
            state
                .part_message_ids
                .insert(part.id.clone(), part.message_id.clone());
            state.part_kinds.insert(part.id.clone(), part.kind.clone());
        }
    }
}

pub(super) fn collect_new_completed_assistant_messages(
    state: &mut OpenCodeRuntimeState,
    messages: &[OpenCodeMessage],
) -> Vec<OpenCodeAssistantCompletion> {
    let mut completions = Vec::new();
    for message in messages {
        let is_new_completed = message.info.session_id == state.session_id
            && message.info.role == "assistant"
            && message.info.time.completed.is_some()
            && !message.info.is_tool_call_only_completion()
            && state.last_completed_assistant_message_id.as_deref()
                != Some(message.info.id.as_str());
        if !is_new_completed {
            continue;
        }
        state.last_completed_assistant_message_id = Some(message.info.id.clone());
        completions.push(OpenCodeAssistantCompletion {
            message_id: message.info.id.clone(),
            completed_at_ms: message.info.time.completed.unwrap_or_default(),
        });
    }
    completions
}

pub(super) fn latest_assistant_usage_tokens(messages: &[OpenCodeMessage]) -> Option<u64> {
    messages.iter().rev().find_map(|message| {
        (message.info.role == "assistant")
            .then(|| message.info.total_tokens())
            .filter(|total| *total > 0)
    })
}

pub(super) fn render_snapshot_output_chunks(
    state: &mut OpenCodeRuntimeState,
    messages: &[OpenCodeMessage],
) -> SnapshotRenderResult {
    let mut chunks = Vec::new();
    for message in messages
        .iter()
        .filter(|message| message.info.role == "assistant")
    {
        for part in &message.parts {
            match part.kind.as_str() {
                "text" | "reasoning" => {
                    if part.text.is_empty() {
                        continue;
                    }
                    let emitted = state
                        .emitted_text_offsets
                        .entry(part.id.clone())
                        .or_insert(0);
                    let start = (*emitted).min(part.text.len());
                    if start == part.text.len() {
                        continue;
                    }
                    chunks.push(OpenCodeOutputChunk {
                        kind: if part.kind == "reasoning" {
                            TerminalOutputKind::ProviderReasoning
                        } else {
                            TerminalOutputKind::ProviderOutput
                        },
                        merge_key: Some(part.id.clone()),
                        bytes: part.text.as_bytes()[start..].to_vec(),
                    });
                    *emitted = part.text.len();
                }
                "tool" => {
                    let summary = render_tool_transcript_update(part);
                    let previous = state.emitted_tool_summaries.get(&part.id);
                    if previous.map(String::as_str) != Some(summary.as_str()) {
                        state
                            .emitted_tool_summaries
                            .insert(part.id.clone(), summary.clone());
                        chunks.push(OpenCodeOutputChunk {
                            kind: TerminalOutputKind::ProviderTool,
                            merge_key: Some(part.id.clone()),
                            bytes: summary.into_bytes(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    SnapshotRenderResult { chunks }
}
