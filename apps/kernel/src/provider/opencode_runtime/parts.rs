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
    if !state.message_belongs_to_active_prompt(&message_id) {
        return Ok(());
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
            emit_part_delta(
                state,
                part_id,
                delta,
                TerminalOutputKind::ProviderReasoning,
                chunks,
            );
        }
        Some("text") => {
            emit_part_delta(
                state,
                part_id,
                delta,
                TerminalOutputKind::ProviderOutput,
                chunks,
            );
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
    if !state.message_belongs_to_active_prompt(&part.message_id) {
        return Ok(());
    }
    let is_assistant = state
        .message_roles
        .get(&part.message_id)
        .is_some_and(|role| role == "assistant");
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
    if !is_assistant {
        return;
    }
    emit_authoritative_part_text(state, &part.id, &part.text, kind, chunks);
}

pub(super) fn emit_authoritative_part_text(
    state: &mut OpenCodeRuntimeState,
    part_id: &str,
    text: &str,
    kind: TerminalOutputKind,
    chunks: &mut Vec<OpenCodeOutputChunk>,
) {
    // A subscriber can join after a part already has text and receive only the
    // next delta. Until a full part update or snapshot establishes the prefix,
    // those deltas must stay buffered; treating their byte count as an offset
    // skips the real prefix and duplicates the final suffix on reconciliation.
    let Some(emitted) = state.emitted_text_by_part.get_mut(part_id) else {
        // `message.part.updated` first announces a newly-created empty text
        // part. That is metadata, not proof that streaming began at offset
        // zero for this subscriber. Keep subsequent deltas buffered until a
        // non-empty cumulative part or snapshot supplies the real prefix.
        if text.is_empty() {
            return;
        }
        state.buffered_text_deltas.remove(part_id);
        state
            .emitted_text_by_part
            .insert(part_id.to_string(), text.to_string());
        chunks.push(OpenCodeOutputChunk {
            kind,
            merge_key: Some(part_id.to_string()),
            bytes: text.as_bytes().to_vec(),
        });
        return;
    };

    state.buffered_text_deltas.remove(part_id);

    if text == emitted || emitted.starts_with(text) {
        return;
    }
    let Some(suffix) = text.strip_prefix(emitted.as_str()) else {
        // OpenCode text parts are append-only. A non-prefix snapshot is stale
        // or a provider-side rewrite and cannot safely be appended to an
        // already delivered transcript.
        return;
    };
    if !suffix.is_empty() {
        chunks.push(OpenCodeOutputChunk {
            kind,
            merge_key: Some(part_id.to_string()),
            bytes: suffix.as_bytes().to_vec(),
        });
        *emitted = text.to_string();
    }
}

fn emit_part_delta(
    state: &mut OpenCodeRuntimeState,
    part_id: String,
    delta: String,
    kind: TerminalOutputKind,
    chunks: &mut Vec<OpenCodeOutputChunk>,
) {
    let Some(emitted) = state.emitted_text_by_part.get_mut(&part_id) else {
        state
            .buffered_text_deltas
            .entry(part_id)
            .or_default()
            .push(delta);
        return;
    };
    emitted.push_str(&delta);
    chunks.push(OpenCodeOutputChunk {
        kind,
        merge_key: Some(part_id),
        bytes: delta.into_bytes(),
    });
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
