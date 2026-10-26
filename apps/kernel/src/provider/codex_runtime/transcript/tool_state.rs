use std::collections::BTreeMap;

use base64::Engine as _;
use serde_json::{json, Value};

use crate::terminal::TerminalOutputKind;

use super::item::is_codex_tool_item;
use super::tool_update::render_codex_tool_transcript_update;
use crate::extension::RemoteExtensionManifest;
use crate::provider::codex_runtime::CodexOutputChunk;

#[derive(Debug, Clone)]
pub(in crate::provider::codex_runtime) struct CodexToolTranscriptState {
    pub(in crate::provider::codex_runtime) item: Value,
    pub(in crate::provider::codex_runtime) streamed_output: String,
    pub(in crate::provider::codex_runtime) pending_output_bytes: Vec<u8>,
    pub(in crate::provider::codex_runtime) output_stream: Option<ToolOutputStream>,
    pub(in crate::provider::codex_runtime) progress_messages: Vec<String>,
    pub(in crate::provider::codex_runtime) last_emitted: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::provider::codex_runtime) enum ToolOutputStream {
    Legacy,
    Modern,
}

pub(in crate::provider::codex_runtime) fn codex_exec_command_item(
    call_id: &str,
    command: Value,
    cwd: Option<String>,
    exit_code: Option<i64>,
) -> Value {
    let mut item = json!({
        "id": call_id,
        "type": "commandExecution",
        "status": "inProgress",
        "command": normalize_codex_command_value(&command),
    });
    if let Some(cwd) = cwd {
        item["cwd"] = json!(cwd);
    }
    if let Some(exit_code) = exit_code {
        item["exitCode"] = json!(exit_code);
    }
    item
}

fn normalize_codex_command_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_string(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub(in crate::provider::codex_runtime) fn decode_codex_output_delta_chunk(chunk: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(chunk)
        .unwrap_or_else(|_| chunk.as_bytes().to_vec())
}

pub(in crate::provider::codex_runtime) fn sync_tool_item_with_manifest(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item: &Value,
    remote_extension_manifest: &RemoteExtensionManifest,
) -> Option<CodexOutputChunk> {
    if !is_codex_tool_item(item) {
        return None;
    }
    let item_id = item.get("id").and_then(Value::as_str)?.to_string();
    let entry = tool_items
        .entry(item_id.clone())
        .or_insert_with(|| CodexToolTranscriptState {
            item: item.clone(),
            streamed_output: String::new(),
            pending_output_bytes: Vec::new(),
            output_stream: None,
            progress_messages: Vec::new(),
            last_emitted: None,
        });
    entry.item = item.clone();
    if matches!(
        item.get("status").and_then(Value::as_str),
        Some("completed" | "failed" | "cancelled" | "canceled" | "interrupted")
    ) && !entry.pending_output_bytes.is_empty()
    {
        entry
            .streamed_output
            .push_str(&String::from_utf8_lossy(&entry.pending_output_bytes));
        entry.pending_output_bytes.clear();
    }
    render_tool_chunk_if_changed(&item_id, entry, remote_extension_manifest)
}

pub(in crate::provider::codex_runtime) fn append_tool_output_delta(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
    item_type: &str,
    delta: &str,
) -> Option<CodexOutputChunk> {
    append_output_delta_from_stream(
        tool_items,
        item_id,
        item_type,
        delta.as_bytes(),
        ToolOutputStream::Modern,
    )
}

pub(in crate::provider::codex_runtime) fn append_legacy_tool_output_delta(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
    item_type: &str,
    delta: &[u8],
) -> Option<CodexOutputChunk> {
    append_output_delta_from_stream(
        tool_items,
        item_id,
        item_type,
        delta,
        ToolOutputStream::Legacy,
    )
}

fn append_output_delta_from_stream(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
    item_type: &str,
    delta: &[u8],
    stream: ToolOutputStream,
) -> Option<CodexOutputChunk> {
    if delta.is_empty() {
        return None;
    }
    let entry = tool_items
        .entry(item_id.to_string())
        .or_insert_with(|| CodexToolTranscriptState {
            item: placeholder_tool_item(item_id, item_type),
            streamed_output: String::new(),
            pending_output_bytes: Vec::new(),
            output_stream: None,
            progress_messages: Vec::new(),
            last_emitted: None,
        });
    // MP-08 / MP-10: app-server can emit both legacy and modern deltas for
    // one call. Keep the first live stream, avoiding duplicated bytes while
    // preserving the prefix delivered before the modern stream starts.
    if entry
        .output_stream
        .is_some_and(|selected| selected != stream)
    {
        return None;
    }
    entry.output_stream = Some(stream);
    entry.pending_output_bytes.extend_from_slice(delta);
    let mut consumed = 0;
    while consumed < entry.pending_output_bytes.len() {
        let remaining = &entry.pending_output_bytes[consumed..];
        match std::str::from_utf8(remaining) {
            Ok(text) => {
                entry.streamed_output.push_str(text);
                consumed = entry.pending_output_bytes.len();
            }
            Err(error) => {
                let valid = error.valid_up_to();
                entry
                    .streamed_output
                    .push_str(std::str::from_utf8(&remaining[..valid]).unwrap());
                consumed += valid;
                if let Some(invalid) = error.error_len() {
                    entry.streamed_output.push('\u{fffd}');
                    consumed += invalid;
                } else {
                    // Retain only an incomplete trailing code point for this
                    // call; another tool's chunks cannot complete it.
                    break;
                }
            }
        }
    }
    entry.pending_output_bytes.drain(..consumed);
    render_tool_chunk_if_changed(item_id, entry, &RemoteExtensionManifest::default())
}

pub(in crate::provider::codex_runtime) fn append_tool_progress(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
    message: &str,
) -> Option<CodexOutputChunk> {
    if message.trim().is_empty() {
        return None;
    }
    let entry = tool_items
        .entry(item_id.to_string())
        .or_insert_with(|| CodexToolTranscriptState {
            item: placeholder_tool_item(item_id, "mcpToolCall"),
            streamed_output: String::new(),
            pending_output_bytes: Vec::new(),
            output_stream: None,
            progress_messages: Vec::new(),
            last_emitted: None,
        });
    entry.progress_messages.push(message.trim().to_string());
    render_tool_chunk_if_changed(item_id, entry, &RemoteExtensionManifest::default())
}

fn render_tool_chunk_if_changed(
    item_id: &str,
    state: &mut CodexToolTranscriptState,
    remote_extension_manifest: &RemoteExtensionManifest,
) -> Option<CodexOutputChunk> {
    let rendered = render_codex_tool_transcript_update(state, remote_extension_manifest)?;
    if state.last_emitted.as_deref() == Some(rendered.as_str()) {
        return None;
    }
    state.last_emitted = Some(rendered.clone());
    Some(CodexOutputChunk {
        kind: TerminalOutputKind::ProviderTool,
        merge_key: Some(item_id.to_string()),
        bytes: rendered.into_bytes(),
    })
}

fn placeholder_tool_item(item_id: &str, item_type: &str) -> Value {
    json!({
        "id": item_id,
        "type": item_type,
        "status": "inProgress",
    })
}

pub(super) fn normalize_merge_key(item_id: &str, fallback: &str) -> String {
    non_empty(item_id).unwrap_or(fallback).to_string()
}

pub(super) fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[cfg(test)]
mod output_stream_tests {
    use super::*;
    // MP-08 / MP-10: reviewer #985 finding 2. The transport splits a
    // three-byte character, so decoding each chunk as a String is invalid.
    #[test]
    fn panes_stream_review_legacy_split_utf8_stays_text() {
        let mut states = BTreeMap::new();
        for bytes in [&"✓".as_bytes()[..1], &"✓".as_bytes()[1..]] {
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            let decoded = decode_codex_output_delta_chunk(&encoded);
            append_legacy_tool_output_delta(&mut states, "unicode", "commandExecution", &decoded);
        }
        assert_eq!(states["unicode"].streamed_output, "✓");
    }

    #[test]
    fn legacy_utf8_carry_is_per_call_and_invalid_bytes_are_lossy() {
        let mut states = BTreeMap::new();
        append_legacy_tool_output_delta(
            &mut states,
            "one",
            "commandExecution",
            &"✓".as_bytes()[..1],
        );
        append_legacy_tool_output_delta(&mut states, "two", "commandExecution", &[0xff]);
        append_legacy_tool_output_delta(
            &mut states,
            "one",
            "commandExecution",
            &"✓".as_bytes()[1..],
        );
        assert_eq!(states["one"].streamed_output, "✓");
        assert_eq!(states["two"].streamed_output, "\u{fffd}");
        assert!(states["one"].pending_output_bytes.is_empty());
    }
    #[test]
    fn legacy_prefix_and_modern_mirror_do_not_duplicate_streamed_tool_output() {
        let mut state = BTreeMap::new();
        let first =
            append_legacy_tool_output_delta(&mut state, "call", "commandExecution", b"line 1\n")
                .unwrap();
        assert!(
            append_tool_output_delta(&mut state, "call", "commandExecution", "line 1\n").is_none()
        );
        let second =
            append_legacy_tool_output_delta(&mut state, "call", "commandExecution", b"line 2\n")
                .unwrap();
        let first: Value = serde_json::from_slice(&first.bytes).unwrap();
        let second: Value = serde_json::from_slice(&second.bytes).unwrap();
        assert_eq!(first["output"], "line 1");
        assert_eq!(second["output"], "line 1\nline 2");
    }
    #[test]
    fn modern_only_and_late_legacy_mirror_keep_the_existing_stream() {
        let mut state = BTreeMap::new();
        append_tool_output_delta(&mut state, "call", "commandExecution", "first ").unwrap();
        assert!(
            append_legacy_tool_output_delta(&mut state, "call", "commandExecution", b"first ")
                .is_none()
        );
        let second =
            append_tool_output_delta(&mut state, "call", "commandExecution", "second").unwrap();
        let second: Value = serde_json::from_slice(&second.bytes).unwrap();
        assert_eq!(second["output"], "first second");
    }
}
