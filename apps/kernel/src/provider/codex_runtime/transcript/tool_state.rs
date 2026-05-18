use std::collections::BTreeMap;

use base64::Engine as _;
use serde_json::{json, Value};

use crate::terminal::TerminalOutputKind;

use super::item::is_codex_tool_item;
use super::tool_update::render_codex_tool_transcript_update;
use crate::provider::codex_runtime::CodexOutputChunk;

#[derive(Debug, Clone)]
pub(in crate::provider::codex_runtime) struct CodexToolTranscriptState {
    pub(in crate::provider::codex_runtime) item: Value,
    pub(in crate::provider::codex_runtime) streamed_output: String,
    pub(in crate::provider::codex_runtime) progress_messages: Vec<String>,
    pub(in crate::provider::codex_runtime) last_emitted: Option<String>,
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

pub(in crate::provider::codex_runtime) fn decode_codex_output_delta_chunk(chunk: &str) -> String {
    base64::engine::general_purpose::STANDARD
        .decode(chunk)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|decoded| !decoded.is_empty())
        .unwrap_or_else(|| chunk.to_string())
}

pub(in crate::provider::codex_runtime) fn sync_tool_item(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item: &Value,
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
            progress_messages: Vec::new(),
            last_emitted: None,
        });
    entry.item = item.clone();
    render_tool_chunk_if_changed(&item_id, entry)
}

pub(in crate::provider::codex_runtime) fn append_tool_output_delta(
    tool_items: &mut BTreeMap<String, CodexToolTranscriptState>,
    item_id: &str,
    item_type: &str,
    delta: &str,
) -> Option<CodexOutputChunk> {
    if delta.is_empty() {
        return None;
    }
    let entry = tool_items
        .entry(item_id.to_string())
        .or_insert_with(|| CodexToolTranscriptState {
            item: placeholder_tool_item(item_id, item_type),
            streamed_output: String::new(),
            progress_messages: Vec::new(),
            last_emitted: None,
        });
    entry.streamed_output.push_str(delta);
    render_tool_chunk_if_changed(item_id, entry)
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
            progress_messages: Vec::new(),
            last_emitted: None,
        });
    entry.progress_messages.push(message.trim().to_string());
    render_tool_chunk_if_changed(item_id, entry)
}

fn render_tool_chunk_if_changed(
    item_id: &str,
    state: &mut CodexToolTranscriptState,
) -> Option<CodexOutputChunk> {
    let rendered = render_codex_tool_transcript_update(state)?;
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
