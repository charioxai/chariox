use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::terminal::TerminalOutputKind;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct ClaudeTranscriptCursor {
    #[serde(default)]
    files: BTreeMap<String, ClaudeTranscriptFileCursor>,
    #[serde(default)]
    seen_keys: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ClaudeTranscriptFileCursor {
    #[serde(default)]
    line_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ClaudeTranscriptChunk {
    pub(super) kind: TerminalOutputKind,
    pub(super) merge_key_suffix: String,
    pub(super) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct ClaudeTranscriptDrain {
    pub(super) chunks: Vec<ClaudeTranscriptChunk>,
    pub(super) assistant_message_ids: Vec<String>,
    pub(super) session_id: Option<String>,
    pub(super) model: Option<String>,
}

fn claude_transcript_cursor_path(context_file: &str) -> Option<PathBuf> {
    std::path::Path::new(context_file)
        .parent()
        .map(|root| root.join("transcript-cursor.json"))
}

pub(super) fn load_claude_transcript_cursor(context_file: &str) -> ClaudeTranscriptCursor {
    let Some(path) = claude_transcript_cursor_path(context_file) else {
        return ClaudeTranscriptCursor::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub(super) fn save_claude_transcript_cursor(context_file: &str, cursor: &ClaudeTranscriptCursor) {
    let Some(path) = claude_transcript_cursor_path(context_file) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string(cursor) {
        let _ = fs::write(path, raw);
    }
}

pub(super) fn known_claude_transcript_paths(context_file: &str) -> Vec<String> {
    load_claude_transcript_cursor(context_file)
        .files
        .keys()
        .cloned()
        .collect()
}

pub(super) fn drain_claude_transcript_file(
    transcript_path: &str,
    cursor: &mut ClaudeTranscriptCursor,
) -> ClaudeTranscriptDrain {
    let path = Path::new(transcript_path);
    let Ok(raw) = fs::read_to_string(path) else {
        return ClaudeTranscriptDrain::default();
    };
    let lines = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let file_key = transcript_path.to_string();
    let start = cursor
        .files
        .get(&file_key)
        .map(|file| file.line_count)
        .unwrap_or_default()
        .min(lines.len());
    let mut drain = ClaudeTranscriptDrain::default();
    for (index, line) in lines.iter().enumerate().skip(start) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let key = claude_transcript_entry_key(transcript_path, index, &value);
        if !cursor.seen_keys.insert(key) {
            continue;
        }
        if drain.session_id.is_none() {
            drain.session_id = claude_string_field(&value, &["sessionId", "session_id"]);
        }
        if let Some(model) = claude_transcript_model(&value) {
            drain.model = Some(model);
        }
        drain.chunks.extend(claude_transcript_chunks(&value));
        if let Some(message_id) = claude_transcript_assistant_message_id(&value) {
            drain.assistant_message_ids.push(message_id);
        }
    }
    cursor.files.insert(
        file_key,
        ClaudeTranscriptFileCursor {
            line_count: lines.len(),
        },
    );
    drain
}

fn claude_transcript_entry_key(path: &str, index: usize, value: &Value) -> String {
    claude_string_field(value, &["uuid"])
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| claude_string_field(message, &["id"]))
        })
        .map(|id| format!("{path}:{id}"))
        .unwrap_or_else(|| format!("{path}:line:{index}"))
}

fn claude_transcript_chunks(value: &Value) -> Vec<ClaudeTranscriptChunk> {
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return Vec::new();
    };
    match kind {
        "assistant" => claude_assistant_transcript_chunks(value),
        "user" => claude_tool_result_transcript_chunks(value),
        _ => Vec::new(),
    }
}

fn claude_assistant_transcript_chunks(value: &Value) -> Vec<ClaudeTranscriptChunk> {
    let message = value.get("message").unwrap_or(value);
    let message_id =
        claude_transcript_assistant_message_id(value).unwrap_or_else(|| "assistant".to_string());
    let Some(content) = message.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut chunks = Vec::new();
    for (index, block) in content.iter().enumerate() {
        let block_kind = block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match block_kind {
            "text" => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    chunks.push(ClaudeTranscriptChunk {
                        kind: TerminalOutputKind::ProviderOutput,
                        merge_key_suffix: format!("assistant:{message_id}:{index}"),
                        text: text.to_string(),
                    });
                }
            }
            "thinking" => {
                if let Some(text) = block
                    .get("thinking")
                    .or_else(|| block.get("text"))
                    .and_then(Value::as_str)
                {
                    chunks.push(ClaudeTranscriptChunk {
                        kind: TerminalOutputKind::ProviderReasoning,
                        merge_key_suffix: format!("thinking:{message_id}:{index}"),
                        text: text.to_string(),
                    });
                }
            }
            "tool_use" => {
                let payload = serde_json::json!({
                    "id": block.get("id").cloned().unwrap_or(Value::Null),
                    "tool": block.get("name").and_then(Value::as_str).unwrap_or("tool"),
                    "status": "started",
                    "input": block.get("input").cloned().unwrap_or(Value::Null),
                });
                chunks.push(ClaudeTranscriptChunk {
                    kind: TerminalOutputKind::ProviderTool,
                    merge_key_suffix: format!(
                        "tool:{}",
                        block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or(&message_id)
                    ),
                    text: payload.to_string(),
                });
            }
            _ => {}
        }
    }
    chunks
}

fn claude_tool_result_transcript_chunks(value: &Value) -> Vec<ClaudeTranscriptChunk> {
    let message = value.get("message").unwrap_or(value);
    let Some(content) = message.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        .filter_map(|block| {
            let tool_use_id = block.get("tool_use_id").and_then(Value::as_str)?;
            let content = match block.get("content") {
                Some(Value::String(value)) => value.to_string(),
                Some(value) => serde_json::to_string(value).unwrap_or_else(|_| value.to_string()),
                None => String::new(),
            };
            let is_error = block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let payload = serde_json::json!({
                "id": tool_use_id,
                "tool": "tool_result",
                "status": if is_error { "failed" } else { "completed" },
                "output": content,
            });
            Some(ClaudeTranscriptChunk {
                kind: TerminalOutputKind::ProviderTool,
                merge_key_suffix: format!("tool:{tool_use_id}"),
                text: payload.to_string(),
            })
        })
        .collect()
}

fn claude_transcript_assistant_message_id(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    value
        .get("message")
        .and_then(|message| claude_string_field(message, &["id"]))
        .or_else(|| claude_string_field(value, &["uuid"]))
}

fn claude_transcript_model(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    value
        .get("message")
        .and_then(|message| claude_string_field(message, &["model"]))
        .map(|model| {
            if model.starts_with("claude/") {
                model
            } else {
                format!("claude/{model}")
            }
        })
}

fn claude_string_field(value: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| {
        value
            .get(*field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}
