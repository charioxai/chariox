//! OpenCode snapshot reconciliation and rendering.

use crate::error::DaemonError;
use crate::extension::RemoteExtensionManifest;
use crate::provider::{OpenCodeClient, OpenCodeMessage, OpenCodeMessageInfo};
use crate::terminal::TerminalOutputKind;

use super::parts::emit_authoritative_part_text;
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

pub(super) fn opencode_messages_active_prompt_failure(
    state: &OpenCodeRuntimeState,
    messages: &[OpenCodeMessage],
) -> Option<String> {
    messages.iter().rev().find_map(|message| {
        (message.info.role == "assistant"
            && state.message_belongs_to_active_prompt(&message.info.id))
        .then(|| message.info.terminal_error_message())
        .flatten()
    })
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
        state
            .message_parent_ids
            .insert(message.info.id.clone(), message.info.parent_id.clone());
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
    record_snapshot_message_metadata(state, messages);
    let mut completions = Vec::new();
    for message in messages {
        let is_new_completed = message.info.session_id == state.session_id
            && message.info.role == "assistant"
            && state.message_belongs_to_active_prompt(&message.info.id)
            && message.info.time.completed.is_some()
            && message.info.is_terminal_assistant_completion()
            && !state
                .completed_assistant_message_ids
                .contains(message.info.id.as_str());
        if !is_new_completed {
            continue;
        }
        state
            .completed_assistant_message_ids
            .insert(message.info.id.clone());
        if state.active_user_message_id.is_some() {
            state.active_terminal_assistant_message_id = Some(message.info.id.clone());
        }
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
    remote_extension_manifest: &RemoteExtensionManifest,
    messages: &[OpenCodeMessage],
) -> SnapshotRenderResult {
    record_snapshot_message_metadata(state, messages);
    let mut chunks = Vec::new();
    for message in messages {
        if message.info.role != "assistant"
            || !state.message_belongs_to_active_prompt(&message.info.id)
        {
            continue;
        }
        for part in &message.parts {
            match part.kind.as_str() {
                "text" | "reasoning" => {
                    emit_authoritative_part_text(
                        state,
                        &part.id,
                        &part.text,
                        if part.kind == "reasoning" {
                            TerminalOutputKind::ProviderReasoning
                        } else {
                            TerminalOutputKind::ProviderOutput
                        },
                        &mut chunks,
                    );
                }
                "tool" => {
                    let summary = render_tool_transcript_update(part, remote_extension_manifest);
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
