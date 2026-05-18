//! Codex transcript projection for text, tool items, and provider tool chunks.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde_json::{json, Value};

use crate::terminal::TerminalOutputKind;

use super::CodexOutputChunk;

mod tool_update;

pub(super) use tool_update::render_codex_tool_transcript_update;

#[derive(Debug, Clone)]
pub(super) struct CodexToolTranscriptState {
    pub(super) item: Value,
    pub(super) streamed_output: String,
    pub(super) progress_messages: Vec<String>,
    pub(super) last_emitted: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct CodexTextTranscriptState {
    emitted: String,
}

pub(super) fn append_text_delta(
    text_items: &mut BTreeMap<String, CodexTextTranscriptState>,
    item_id: &str,
    fallback: &str,
    kind: TerminalOutputKind,
    delta: &str,
    chunks: &mut Vec<CodexOutputChunk>,
) {
    if delta.is_empty() {
        return;
    }
    let merge_key = normalize_merge_key(item_id, fallback);
    text_items
        .entry(merge_key.clone())
        .or_default()
        .emitted
        .push_str(delta);
    chunks.push(CodexOutputChunk {
        kind,
        merge_key: Some(merge_key),
        bytes: delta.as_bytes().to_vec(),
    });
}

pub(super) fn sync_completed_text_item(
    text_items: &mut BTreeMap<String, CodexTextTranscriptState>,
    item: &Value,
) -> Option<CodexOutputChunk> {
    let item_type = normalize_codex_item_type(item.get("type").and_then(Value::as_str)?)?;
    let (kind, fallback) = match item_type {
        "agentMessage" => (TerminalOutputKind::ProviderOutput, "codex-agent-message"),
        "reasoning" => (TerminalOutputKind::ProviderReasoning, "codex-reasoning"),
        _ => return None,
    };
    let text = completed_text_item_text(item_type, item)?;
    if text.is_empty() {
        return None;
    }
    let item_id = item.get("id").and_then(Value::as_str).unwrap_or_default();
    let merge_key = normalize_merge_key(item_id, fallback);
    let entry = text_items.entry(merge_key.clone()).or_default();
    let delta = if entry.emitted.is_empty() {
        text.as_str()
    } else if let Some(suffix) = text.strip_prefix(&entry.emitted) {
        suffix
    } else {
        crate::logging::debug_with_fields(
            "daemon.provider.codex",
            "codex completed text item did not match streamed prefix",
            json!({
                "id": item_id,
                "type": item_type,
                "streamed_len": entry.emitted.len(),
                "completed_len": text.len(),
            }),
        );
        return None;
    };
    if delta.is_empty() {
        return None;
    }
    entry.emitted.push_str(delta);
    Some(CodexOutputChunk {
        kind,
        merge_key: Some(merge_key),
        bytes: delta.as_bytes().to_vec(),
    })
}

fn completed_text_item_text(item_type: &str, item: &Value) -> Option<String> {
    match item_type {
        "agentMessage" => item
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| text_from_content_value(item.get("content"))),
        "reasoning" => text_from_string_array(item.get("summary"))
            .or_else(|| text_from_string_array(item.get("content")))
            .or_else(|| item.get("text").and_then(Value::as_str).map(str::to_string)),
        _ => None,
    }
}

pub(super) fn normalize_codex_item_type(raw_type: &str) -> Option<&str> {
    match raw_type {
        "" => None,
        "UserMessage" => Some("userMessage"),
        "AgentMessage" => Some("agentMessage"),
        "Reasoning" => Some("reasoning"),
        "Plan" => Some("plan"),
        "CommandExecution" => Some("commandExecution"),
        "FileChange" => Some("fileChange"),
        "McpToolCall" => Some("mcpToolCall"),
        "WebSearch" => Some("webSearch"),
        other => Some(other),
    }
}

fn text_from_string_array(value: Option<&Value>) -> Option<String> {
    let items = value?.as_array()?;
    let text = items
        .iter()
        .filter_map(Value::as_str)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

pub(super) fn text_from_content_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => (!text.is_empty()).then(|| text.clone()),
        Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .or_else(|| item.get("text").and_then(Value::as_str).map(str::to_string))
                })
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

pub(super) fn codex_exec_command_item(
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

pub(super) fn decode_codex_output_delta_chunk(chunk: &str) -> String {
    base64::engine::general_purpose::STANDARD
        .decode(chunk)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|decoded| !decoded.is_empty())
        .unwrap_or_else(|| chunk.to_string())
}

pub(super) fn sync_tool_item(
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

pub(super) fn append_tool_output_delta(
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

pub(super) fn append_tool_progress(
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

pub(super) fn is_codex_tool_item(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(Value::as_str),
        Some(
            "commandExecution"
                | "fileChange"
                | "mcpToolCall"
                | "dynamicToolCall"
                | "collabAgentToolCall"
        )
    )
}

pub(super) fn codex_item_id(item: &Value) -> Option<&str> {
    item.get("id")
        .or_else(|| item.get("callId"))
        .or_else(|| item.get("call_id"))
        .and_then(Value::as_str)
        .filter(|item_id| !item_id.is_empty())
}

pub(super) fn codex_item_status_is_terminal(item: &Value) -> bool {
    matches!(
        item.get("status").and_then(Value::as_str),
        Some("completed" | "failed" | "canceled" | "cancelled")
    )
}

fn normalize_merge_key(item_id: &str, fallback: &str) -> String {
    non_empty(item_id).unwrap_or(fallback).to_string()
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}
