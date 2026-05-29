//! OpenCode streamed message part rendering.

use crate::error::DaemonError;
use crate::extension::RemoteExtensionManifest;
use crate::provider::OpenCodePart;
use crate::terminal::TerminalOutputKind;

use super::snapshot::refresh_opencode_message_metadata;
use super::transcript::render_tool_transcript_update;
use super::{OpenCodeOutputChunk, OpenCodeRuntimeState};

pub(super) fn handle_message_part_delta(
    state: &mut OpenCodeRuntimeState,
    provider_run_id: &str,
    session_id: String,
    message_id: String,
    part_id: String,
    field: String,
    delta: String,
    chunks: &mut Vec<OpenCodeOutputChunk>,
) -> Result<(), DaemonError> {
    if session_id != state.session_id || field != "text" || delta.is_empty() {
        return Ok(());
    }
    state
        .part_message_ids
        .insert(part_id.clone(), message_id.clone());
    if !state.message_roles.contains_key(&message_id) {
        refresh_opencode_message_metadata(state, provider_run_id)?;
    }
    let Some(role) = state.message_roles.get(&message_id).map(String::as_str) else {
        state
            .buffered_text_deltas
            .entry(part_id)
            .or_default()
            .push(delta);
        return Ok(());
    };
    if role != "assistant" {
        return Ok(());
    }
    match state.part_kinds.get(&part_id).map(String::as_str) {
        Some("reasoning") => {
            let emitted = state
                .emitted_text_offsets
                .entry(part_id.clone())
                .or_insert(0);
            *emitted += delta.len();
            chunks.push(OpenCodeOutputChunk {
                kind: TerminalOutputKind::ProviderReasoning,
                merge_key: Some(part_id.clone()),
                bytes: delta.into_bytes(),
            });
        }
        Some("text") => {
            let emitted = state
                .emitted_text_offsets
                .entry(part_id.clone())
                .or_insert(0);
            *emitted += delta.len();
            chunks.push(OpenCodeOutputChunk {
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some(part_id.clone()),
                bytes: delta.into_bytes(),
            });
        }
        Some(_) => {}
        None => {
            state
                .buffered_text_deltas
                .entry(part_id)
                .or_default()
                .push(delta);
        }
    }
    Ok(())
}

pub(super) fn handle_message_part_updated(
    state: &mut OpenCodeRuntimeState,
    provider_run_id: &str,
    remote_extension_manifest: &RemoteExtensionManifest,
    part: OpenCodePart,
    chunks: &mut Vec<OpenCodeOutputChunk>,
) -> Result<(), DaemonError> {
    if part.session_id != state.session_id {
        return Ok(());
    }
    state
        .part_message_ids
        .insert(part.id.clone(), part.message_id.clone());
    state.part_kinds.insert(part.id.clone(), part.kind.clone());
    if !state.message_roles.contains_key(&part.message_id) {
        refresh_opencode_message_metadata(state, provider_run_id)?;
    }
    let is_assistant = state
        .message_roles
        .get(&part.message_id)
        .is_some_and(|role| role == "assistant");
    if let Some(buffered_deltas) = state.buffered_text_deltas.remove(&part.id) {
        for delta in buffered_deltas {
            if !is_assistant {
                continue;
            }
            let emitted = state
                .emitted_text_offsets
                .entry(part.id.clone())
                .or_insert(0);
            *emitted += delta.len();
            match part.kind.as_str() {
                "reasoning" => chunks.push(OpenCodeOutputChunk {
                    kind: TerminalOutputKind::ProviderReasoning,
                    merge_key: Some(part.id.clone()),
                    bytes: delta.into_bytes(),
                }),
                "text" => {
                    chunks.push(OpenCodeOutputChunk {
                        kind: TerminalOutputKind::ProviderOutput,
                        merge_key: Some(part.id.clone()),
                        bytes: delta.into_bytes(),
                    });
                }
                _ => {}
            }
        }
    }
    match part.kind.as_str() {
        "text" => {
            emit_part_text(
                state,
                &part,
                is_assistant,
                TerminalOutputKind::ProviderOutput,
                chunks,
            );
        }
        "reasoning" => {
            emit_part_text(
                state,
                &part,
                is_assistant,
                TerminalOutputKind::ProviderReasoning,
                chunks,
            );
        }
        "tool" => {
            if is_assistant {
                emit_tool_summary(state, remote_extension_manifest, &part, chunks);
            }
        }
        _ => {}
    }
    Ok(())
}

fn emit_part_text(
    state: &mut OpenCodeRuntimeState,
    part: &OpenCodePart,
    is_assistant: bool,
    kind: TerminalOutputKind,
    chunks: &mut Vec<OpenCodeOutputChunk>,
) {
    if !is_assistant || part.text.is_empty() {
        return;
    }
    let emitted = state
        .emitted_text_offsets
        .entry(part.id.clone())
        .or_insert(0);
    let start = (*emitted).min(part.text.len());
    if start == part.text.len() {
        return;
    }
    chunks.push(OpenCodeOutputChunk {
        kind,
        merge_key: Some(part.id.clone()),
        bytes: part.text.as_bytes()[start..].to_vec(),
    });
    *emitted = part.text.len();
}

fn emit_tool_summary(
    state: &mut OpenCodeRuntimeState,
    remote_extension_manifest: &RemoteExtensionManifest,
    part: &OpenCodePart,
    chunks: &mut Vec<OpenCodeOutputChunk>,
) {
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
