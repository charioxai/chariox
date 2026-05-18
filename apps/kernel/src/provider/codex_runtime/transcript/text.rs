use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::terminal::TerminalOutputKind;

use super::item::normalize_codex_item_type;
use super::tool_state::normalize_merge_key;
use crate::provider::codex_runtime::CodexOutputChunk;

#[derive(Debug, Clone, Default)]
pub(in crate::provider::codex_runtime) struct CodexTextTranscriptState {
    emitted: String,
}

pub(in crate::provider::codex_runtime) fn append_text_delta(
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

pub(in crate::provider::codex_runtime) fn sync_completed_text_item(
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

pub(in crate::provider::codex_runtime) fn text_from_content_value(
    value: Option<&Value>,
) -> Option<String> {
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
