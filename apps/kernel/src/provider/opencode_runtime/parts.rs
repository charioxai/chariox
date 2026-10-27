//! OpenCode streamed message part rendering.

use crate::error::DaemonError;
use crate::extension::RemoteExtensionManifest;
use crate::provider::OpenCodePart;
use crate::terminal::TerminalOutputKind;

use super::snapshot::refresh_opencode_message_metadata;
use super::transcript::render_tool_transcript_update;
use super::{OpenCodeOutputChunk, OpenCodeRuntimeState};

#[allow(
    clippy::too_many_arguments,
    reason = "Preserve the existing handle_message_part_delta operation signature and explicit context arguments"
)]
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
    // A completed HTTP snapshot already includes every byte of this part.
    // SSE delivery may lag behind that snapshot; appending those deltas replays
    // the answer. Unfinished parts still accept identical, legitimate deltas.
    if session_id != state.session_id
        || field != "text"
        || delta.is_empty()
        || state.finalized_text_part_ids.contains(&part_id)
    {
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
        None => {}
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
    // Only the SSE cumulative update establishes the event stream's position.
    // A snapshot may already have published a later prefix of this same block.
    let streamed = state
        .streamed_text_by_part
        .entry(part.id.clone())
        .or_default();
    if !streamed.starts_with(&part.text) {
        *streamed = part.text.clone();
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
    // MP-08 / MP-10: a live subscription established before a new prompt
    // sees its empty part announcement from offset zero. Reconnected/late
    // subscriptions still need a non-empty authoritative prefix.
    let Some(emitted) = state.emitted_text_by_part.get_mut(part_id) else {
        if text.is_empty() && !state.live_text_from_start {
            return;
        }
        state
            .emitted_text_by_part
            .insert(part_id.to_string(), text.to_string());
        if !text.is_empty() {
            chunks.push(OpenCodeOutputChunk {
                kind,
                merge_key: Some(part_id.to_string()),
                bytes: text.as_bytes().to_vec(),
            });
        }
        return;
    };

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
    if state.live_text_from_start
        && state
            .emitted_text_by_part
            .get(&part_id)
            .is_some_and(String::is_empty)
    {
        state
            .streamed_text_by_part
            .entry(part_id.clone())
            .or_default();
    }
    let Some(streamed) = state.streamed_text_by_part.get_mut(&part_id) else {
        return;
    };
    if !state.live_text_from_start && streamed.is_empty() {
        return;
    }
    let caught_up = state
        .emitted_text_by_part
        .get(&part_id)
        .is_some_and(|emitted| emitted.as_str() == streamed.as_str());
    streamed.push_str(&delta);
    if caught_up {
        state
            .emitted_text_by_part
            .get_mut(&part_id)
            .unwrap()
            .push_str(&delta);
        chunks.push(OpenCodeOutputChunk {
            kind,
            merge_key: Some(part_id),
            bytes: delta.into_bytes(),
        });
        return;
    }
    if !state.emitted_text_by_part.contains_key(&part_id) {
        return;
    }
    let cumulative = streamed.clone();
    emit_authoritative_part_text(state, &part_id, &cumulative, kind, chunks);
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
