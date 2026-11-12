use crate::provider_output_policy::output_bounds::shrink_strings_to_fit;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use serde_json::Value;

use crate::terminal::TerminalOutputKind;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct ClaudeTranscriptCursor {
    #[serde(default)]
    files: BTreeMap<String, ClaudeTranscriptFileCursor>,
    #[serde(default)]
    seen_keys: BTreeSet<String>,
    #[serde(default)]
    seen_assistant_message_ids: BTreeSet<String>,
    /// Calls whose result has not arrived yet: the result line names only the call id.
    #[serde(default)]
    pending_tool_calls: BTreeMap<String, ClaudeToolCall>,
    /// Launched background commands still running, by tool call id. They
    /// outlive the turn that started them.
    #[serde(default)]
    background_calls: BTreeMap<String, ClaudeBackgroundCall>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ClaudeToolCall {
    tool: String,
    input: Value,
    /// When the kernel first saw the call.
    #[serde(default)]
    started_at_ms: Option<u64>,
    #[serde(default)]
    background: bool,
}

/// A background command's launch snapshot and the task id Claude names it by.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ClaudeBackgroundCall {
    task_id: Option<String>,
    snapshot: Value,
}

/// A larger input stays in the call's own history entry; the result keeps only the tool's name.
const PENDING_TOOL_INPUT_BYTES: usize = 8 * 1024;
const PENDING_TOOL_CALLS: usize = 64;

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
    pub(super) terminal_assistant_message_ids: Vec<String>,
    pub(super) terminal_failure: Option<String>,
    pub(super) enqueued_prompts: Vec<String>,
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

#[cfg(test)]
pub(super) fn drain_claude_transcript_file(
    transcript_path: &str,
    cursor: &mut ClaudeTranscriptCursor,
) -> ClaudeTranscriptDrain {
    drain_claude_transcript_file_since(transcript_path, cursor, None)
}

#[cfg(test)]
pub(super) fn drain_claude_transcript_file_since(
    transcript_path: &str,
    cursor: &mut ClaudeTranscriptCursor,
    minimum_timestamp_ms: Option<u64>,
) -> ClaudeTranscriptDrain {
    drain_claude_transcript_file_at(
        transcript_path,
        cursor,
        minimum_timestamp_ms,
        crate::session::unix_epoch_ms(),
    )
}

#[cfg(test)]
pub(super) fn drain_claude_transcript_file_at(
    transcript_path: &str,
    cursor: &mut ClaudeTranscriptCursor,
    minimum_timestamp_ms: Option<u64>,
    now_ms: u64,
) -> ClaudeTranscriptDrain {
    let path = Path::new(transcript_path);
    let Ok(raw) = fs::read_to_string(path) else {
        return ClaudeTranscriptDrain::default();
    };
    drain_claude_transcript_raw_since(transcript_path, &raw, cursor, minimum_timestamp_ms, now_ms)
}

/// Drains the transcript lines not seen yet; `now_ms` is when the kernel
/// reads them, the start of a call and the end of a background command.
pub(super) fn drain_claude_transcript_raw_since(
    transcript_path: &str,
    raw: &str,
    cursor: &mut ClaudeTranscriptCursor,
    minimum_timestamp_ms: Option<u64>,
    now_ms: u64,
) -> ClaudeTranscriptDrain {
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
    let mut consumed_line_count = start;
    for (index, line) in lines.iter().enumerate().skip(start) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            if index + 1 == lines.len() && !raw.ends_with('\n') {
                break;
            }
            consumed_line_count = index + 1;
            continue;
        };
        consumed_line_count = index + 1;
        let key = claude_transcript_entry_key(transcript_path, index, &value);
        if !cursor.seen_keys.insert(key) {
            continue;
        }
        if minimum_timestamp_ms.is_some_and(|minimum_timestamp_ms| {
            claude_transcript_timestamp_ms(&value)
                .is_some_and(|timestamp_ms| timestamp_ms < minimum_timestamp_ms)
        }) {
            // The cutoff drops the old turns, not the end of a command the kernel
            // already tracks: its notice or stop can be written before a newer
            // prompt starts and still be unread when that prompt's cutoff is set.
            drain
                .chunks
                .extend(claude_background_end_chunks(&value, cursor, now_ms));
            continue;
        }
        if drain.session_id.is_none() {
            drain.session_id = claude_string_field(&value, &["sessionId", "session_id"]);
        }
        if claude_transcript_is_internal_resume_entry(&value) {
            continue;
        }
        if let Some(model) = claude_transcript_model(&value) {
            drain.model = Some(model);
        }
        if let Some(prompt) = claude_transcript_enqueued_prompt(&value) {
            drain.enqueued_prompts.push(prompt);
        }
        let chunks = claude_transcript_chunks(&value, cursor, now_ms);
        if drain.terminal_failure.is_none() {
            drain.terminal_failure = chunks.iter().find_map(|chunk| {
                crate::provider::classify_provider_terminal_failure_output_text(
                    "claude",
                    &chunk.text,
                )
            });
        }
        drain.chunks.extend(chunks);
        if let Some(message_id) = claude_transcript_assistant_message_id(&value) {
            if cursor.seen_assistant_message_ids.insert(message_id.clone()) {
                drain.assistant_message_ids.push(message_id.clone());
                if claude_transcript_assistant_turn_is_terminal(&value) {
                    drain.terminal_assistant_message_ids.push(message_id);
                }
            }
        }
    }
    cursor.files.insert(
        file_key,
        ClaudeTranscriptFileCursor {
            line_count: consumed_line_count,
        },
    );
    drain
}

fn claude_transcript_assistant_turn_is_terminal(value: &Value) -> bool {
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return false;
    }
    matches!(
        value
            .get("message")
            .unwrap_or(value)
            .get("stop_reason")
            .and_then(Value::as_str),
        Some("end_turn" | "stop_sequence" | "max_tokens" | "refusal")
    )
}

fn claude_transcript_is_internal_resume_entry(value: &Value) -> bool {
    match value.get("type").and_then(Value::as_str) {
        Some("user") => value.get("isMeta").and_then(Value::as_bool) == Some(true),
        Some("assistant") => {
            value
                .get("message")
                .and_then(|message| message.get("model"))
                .and_then(Value::as_str)
                == Some("<synthetic>")
                && value.get("isApiErrorMessage").and_then(Value::as_bool) != Some(true)
        }
        _ => false,
    }
}

fn claude_transcript_enqueued_prompt(value: &Value) -> Option<String> {
    (value.get("type").and_then(Value::as_str) == Some("queue-operation")
        && value.get("operation").and_then(Value::as_str) == Some("enqueue"))
    .then(|| claude_string_field(value, &["content"]))
    .flatten()
}

fn claude_transcript_timestamp_ms(value: &Value) -> Option<u64> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.timestamp_millis().max(0) as u64)
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

fn claude_transcript_chunks(
    value: &Value,
    cursor: &mut ClaudeTranscriptCursor,
    now_ms: u64,
) -> Vec<ClaudeTranscriptChunk> {
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return Vec::new();
    };
    if let Some(notification) = claude_task_notification(value) {
        return finish_background_call(cursor, notification, now_ms)
            .into_iter()
            .collect();
    }
    match kind {
        "assistant" => {
            claude_assistant_transcript_chunks(value, &mut cursor.pending_tool_calls, now_ms)
        }
        "user" => claude_tool_result_transcript_chunks(value, cursor, now_ms),
        _ => Vec::new(),
    }
}

/// Claude tells the model a background task ended with a
/// `<task-notification>`: first queued (`queue-operation` enqueue), then as
/// the user turn it starts. A command the session stopped on exit is only queued.
fn claude_task_notification(value: &Value) -> Option<ClaudeTaskEnd<'_>> {
    let text = match value.get("type").and_then(Value::as_str)? {
        "queue-operation" => value.get("content")?.as_str()?,
        "user" => value.get("message")?.get("content")?.as_str()?,
        _ => return None,
    };
    let notification = text.trim_start().strip_prefix("<task-notification>")?;
    let tag = |name: &str| {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let rest = notification.split_once(open.as_str())?.1;
        Some(rest.split_once(close.as_str())?.0.trim())
    };
    if !matches!(
        tag("status"),
        Some("completed" | "failed" | "killed" | "stopped")
    ) {
        return None;
    }
    Some(ClaudeTaskEnd {
        tool_use_id: tag("tool-use-id"),
        task_id: tag("task-id"),
    })
}

/// The background command a task notification or a stop result is about.
#[derive(Clone, Copy)]
struct ClaudeTaskEnd<'a> {
    tool_use_id: Option<&'a str>,
    task_id: Option<&'a str>,
}

/// The ended background call's launch snapshot, now with its end time.
fn finish_background_call(
    cursor: &mut ClaudeTranscriptCursor,
    end: ClaudeTaskEnd<'_>,
    now_ms: u64,
) -> Option<ClaudeTranscriptChunk> {
    let id = end
        .tool_use_id
        .filter(|id| cursor.background_calls.contains_key(*id))
        .map(str::to_string)
        .or_else(|| {
            let task_id = end.task_id?;
            cursor
                .background_calls
                .iter()
                .find(|(_, call)| call.task_id.as_deref() == Some(task_id))
                .map(|(id, _)| id.clone())
        })?;
    let mut snapshot = cursor.background_calls.remove(&id)?.snapshot;
    snapshot["finished_at_ms"] = serde_json::json!(now_ms);
    Some(ClaudeTranscriptChunk {
        kind: TerminalOutputKind::ProviderTool,
        merge_key_suffix: format!("tool:{id}"),
        text: snapshot.to_string(),
    })
}

fn track_pending_call(
    pending_tool_calls: &mut BTreeMap<String, ClaudeToolCall>,
    id: &str,
    name: &str,
    input: &Value,
    background: bool,
    now_ms: u64,
) {
    if pending_tool_calls.len() >= PENDING_TOOL_CALLS {
        return;
    }
    // A file write carries its whole content: keep its path and the start of the content, which the row shows.
    let kept = if input.to_string().len() <= PENDING_TOOL_INPUT_BYTES {
        input.clone()
    } else {
        let mut shrunk = input.clone();
        if shrink_strings_to_fit(
            &mut shrunk,
            PENDING_TOOL_INPUT_BYTES,
            "\n[chariox: tool transcript truncated]",
        ) {
            shrunk
        } else {
            Value::Null
        }
    };
    pending_tool_calls.insert(
        id.to_string(),
        ClaudeToolCall {
            tool: name.to_string(),
            input: kept,
            started_at_ms: Some(now_ms),
            background,
        },
    );
}

/// What an entry older than the active prompt still does: ends the tracked
/// background commands it names, and nothing else reaches the prompt. A stop
/// or poll call is remembered silently so its older result can end its task.
fn claude_background_end_chunks(
    value: &Value,
    cursor: &mut ClaudeTranscriptCursor,
    now_ms: u64,
) -> Vec<ClaudeTranscriptChunk> {
    if let Some(notification) = claude_task_notification(value) {
        return finish_background_call(cursor, notification, now_ms)
            .into_iter()
            .collect();
    }
    let kind = value.get("type").and_then(Value::as_str);
    let message = value.get("message").unwrap_or(value);
    let Some(content) = message.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    match kind {
        Some("assistant") => {
            for block in content {
                let (Some("tool_use"), Some(id), Some(name)) = (
                    block.get("type").and_then(Value::as_str),
                    block.get("id").and_then(Value::as_str),
                    block.get("name").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if crate::provider::ends_background_commands(name) {
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    track_pending_call(
                        &mut cursor.pending_tool_calls,
                        id,
                        name,
                        &input,
                        false,
                        now_ms,
                    );
                }
            }
            Vec::new()
        }
        Some("user") => {
            let mut ended_tasks = Vec::new();
            for block in content {
                let Some(id) = block
                    .get("type")
                    .and_then(Value::as_str)
                    .filter(|kind| *kind == "tool_result")
                    .and_then(|_| block.get("tool_use_id").and_then(Value::as_str))
                else {
                    continue;
                };
                if !cursor
                    .pending_tool_calls
                    .get(id)
                    .is_some_and(|call| crate::provider::ends_background_commands(&call.tool))
                {
                    continue;
                }
                let call = cursor.pending_tool_calls.remove(id);
                let failed = block.get("is_error").and_then(Value::as_bool) == Some(true);
                if let (Some(call), false) = (call, failed) {
                    let output = claude_tool_result_text(block.get("content"));
                    ended_tasks.extend(crate::provider::ended_task_of_result(
                        &call.tool,
                        &call.input,
                        &output,
                    ));
                }
            }
            finish_ended_tasks(cursor, ended_tasks, now_ms)
        }
        _ => Vec::new(),
    }
}

fn finish_ended_tasks(
    cursor: &mut ClaudeTranscriptCursor,
    ended_tasks: Vec<String>,
    now_ms: u64,
) -> Vec<ClaudeTranscriptChunk> {
    ended_tasks
        .iter()
        .filter_map(|task_id| {
            let end = ClaudeTaskEnd {
                tool_use_id: None,
                task_id: Some(task_id),
            };
            finish_background_call(cursor, end, now_ms)
        })
        .collect()
}

fn claude_tool_result_text(result: Option<&Value>) -> String {
    match result {
        Some(Value::String(value)) => value.to_string(),
        Some(Value::Array(_)) => crate::provider::bounded_result(result),
        Some(value) => serde_json::to_string(value).unwrap_or_else(|_| value.to_string()),
        None => String::new(),
    }
}

fn claude_assistant_transcript_chunks(
    value: &Value,
    pending_tool_calls: &mut BTreeMap<String, ClaudeToolCall>,
    now_ms: u64,
) -> Vec<ClaudeTranscriptChunk> {
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
                let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                let background = crate::provider::launches_background_task(name, Some(&input));
                if let Some(id) = block.get("id").and_then(Value::as_str) {
                    track_pending_call(pending_tool_calls, id, name, &input, background, now_ms);
                }
                let mut payload = serde_json::json!({
                    "id": block.get("id").cloned().unwrap_or(Value::Null),
                    "tool": name,
                    "status": "started",
                    "input": input,
                    "started_at_ms": now_ms,
                });
                if background {
                    payload["background"] = serde_json::json!(true);
                    payload["finished_at_ms"] = Value::Null;
                }
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

/// Claude closes a call that a stopped session left open with a result of its own on resume.
const CLAUDE_RESUME_REPAIR_RESULT_PREFIX: &str = "[Tool call interrupted:";
/// Claude's result for a call the person stopped (or declined): the call was cancelled, not failed.
const CLAUDE_STOPPED_CALL_RESULT_PREFIX: &str =
    "The user doesn't want to proceed with this tool use.";

/// At most this many background commands are tracked at once; an untracked
/// one ends with its provider process.
const BACKGROUND_CALLS: usize = 64;

fn claude_tool_result_transcript_chunks(
    value: &Value,
    cursor: &mut ClaudeTranscriptCursor,
    now_ms: u64,
) -> Vec<ClaudeTranscriptChunk> {
    let message = value.get("message").unwrap_or(value);
    let Some(content) = message.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut ended_tasks = Vec::new();
    let mut chunks: Vec<ClaudeTranscriptChunk> = content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        .filter_map(|block| {
            let tool_use_id = block.get("tool_use_id").and_then(Value::as_str)?;
            let result = block.get("content");
            if result
                .and_then(Value::as_str)
                .is_some_and(|text| text.starts_with(CLAUDE_RESUME_REPAIR_RESULT_PREFIX))
            {
                cursor.pending_tool_calls.remove(tool_use_id);
                return None;
            }
            let content = claude_tool_result_text(result);
            let (images, images_omitted) = crate::provider::bounded_result_images(result);
            let is_error = block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let status = if !is_error {
                "completed"
            } else if content.starts_with(CLAUDE_STOPPED_CALL_RESULT_PREFIX) {
                "cancelled"
            } else {
                "failed"
            };
            // The result replaces the call's history entry, so it carries the call's name and input.
            let call = cursor.pending_tool_calls.remove(tool_use_id);
            let mut payload = serde_json::json!({
                "id": tool_use_id,
                "tool": call.as_ref().map_or("tool_result", |call| call.tool.as_str()),
                "status": status,
                "output": content,
            });
            if !images.is_empty() {
                payload["images"] = serde_json::json!(images);
            }
            if images_omitted > 0 {
                payload["images_omitted"] = serde_json::json!(images_omitted);
            }
            if let Some(call) = call {
                if let Some(started_at_ms) = call.started_at_ms {
                    payload["started_at_ms"] = serde_json::json!(started_at_ms);
                }
                if status == "completed" {
                    if let Some(task_id) =
                        crate::provider::ended_task_of_result(&call.tool, &call.input, &content)
                    {
                        ended_tasks.push(task_id);
                    }
                }
                payload["input"] = call.input;
                if call.background {
                    payload["background"] = serde_json::json!(true);
                    // A launch that failed or was stopped never left a command running.
                    payload["finished_at_ms"] = if status == "completed" {
                        Value::Null
                    } else {
                        serde_json::json!(now_ms)
                    };
                    if status == "completed" && cursor.background_calls.len() < BACKGROUND_CALLS {
                        cursor.background_calls.insert(
                            tool_use_id.to_string(),
                            ClaudeBackgroundCall {
                                task_id: crate::provider::launched_task_id(
                                    value.get("toolUseResult"),
                                    &content,
                                ),
                                snapshot: payload.clone(),
                            },
                        );
                    }
                }
            }
            Some(ClaudeTranscriptChunk {
                kind: TerminalOutputKind::ProviderTool,
                merge_key_suffix: format!("tool:{tool_use_id}"),
                text: payload.to_string(),
            })
        })
        .collect();
    chunks.extend(finish_ended_tasks(cursor, ended_tasks, now_ms));
    chunks
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
        .filter(|model| model != "<synthetic>")
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
