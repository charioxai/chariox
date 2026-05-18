use serde::Serialize;
use serde_json::{json, Value};

use super::{non_empty, CodexToolTranscriptState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CodexToolTranscriptUpdate {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw: Option<String>,
}

pub(in crate::provider::codex_runtime) fn render_codex_tool_transcript_update(
    state: &CodexToolTranscriptState,
) -> Option<String> {
    let item = &state.item;
    let id = item.get("id").and_then(Value::as_str)?.to_string();
    let item_type = item.get("type").and_then(Value::as_str)?;
    let status = normalize_codex_tool_status(
        item.get("status")
            .and_then(Value::as_str)
            .unwrap_or("updated"),
    );

    let update = match item_type {
        "commandExecution" => CodexToolTranscriptUpdate {
            id,
            tool: Some("bash".to_string()),
            status: Some(status),
            title: None,
            description: item
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(|cwd| format!("cwd {cwd}")),
            text: None,
            input: Some(json!({
                "command": item.get("command").and_then(Value::as_str).unwrap_or_default(),
                "cwd": item.get("cwd").and_then(Value::as_str).unwrap_or_default(),
            })),
            output: prefer_output(
                item.get("aggregatedOutput").and_then(Value::as_str),
                &state.streamed_output,
            ),
            error: command_execution_error(item),
            raw: command_execution_raw(item),
        },
        "fileChange" => CodexToolTranscriptUpdate {
            id,
            tool: Some("apply_patch".to_string()),
            status: Some(status),
            title: item
                .get("changes")
                .and_then(Value::as_array)
                .map(|changes| format!("{} file changes", changes.len()))
                .filter(|title| !title.starts_with('0')),
            description: None,
            text: None,
            input: None,
            output: prefer_output(None, &state.streamed_output),
            error: None,
            raw: item
                .get("changes")
                .filter(|value| !is_empty_json_value(value))
                .map(render_json_value),
        },
        "mcpToolCall" => CodexToolTranscriptUpdate {
            id,
            tool: item
                .get("tool")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string)
                .or_else(|| Some("mcp".to_string())),
            status: Some(status),
            title: item
                .get("server")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string),
            description: None,
            text: (!state.progress_messages.is_empty()).then(|| state.progress_messages.join("\n")),
            input: item
                .get("arguments")
                .filter(|value| !is_empty_json_value(value))
                .cloned(),
            output: item
                .get("result")
                .filter(|value| !is_empty_json_value(value))
                .map(render_json_value),
            error: item
                .get("error")
                .and_then(|value| {
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .or_else(|| value.as_str())
                })
                .and_then(non_empty)
                .map(str::to_string),
            raw: None,
        },
        "dynamicToolCall" => CodexToolTranscriptUpdate {
            id,
            tool: item
                .get("tool")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string)
                .or_else(|| Some("tool".to_string())),
            status: Some(status),
            title: None,
            description: None,
            text: None,
            input: item
                .get("arguments")
                .filter(|value| !is_empty_json_value(value))
                .cloned(),
            output: item
                .get("contentItems")
                .filter(|value| !is_empty_json_value(value))
                .map(render_json_value),
            error: item
                .get("success")
                .and_then(Value::as_bool)
                .filter(|success| !success)
                .map(|_| "Dynamic tool call failed".to_string()),
            raw: None,
        },
        "collabAgentToolCall" => CodexToolTranscriptUpdate {
            id,
            tool: item
                .get("tool")
                .and_then(Value::as_str)
                .and_then(non_empty)
                .map(str::to_string)
                .or_else(|| Some("collab".to_string())),
            status: Some(status),
            title: None,
            description: None,
            text: None,
            input: Some(json!({
                "prompt": item.get("prompt").cloned().unwrap_or(Value::Null),
                "receiverThreadIds": item.get("receiverThreadIds").cloned().unwrap_or(Value::Null),
                "model": item.get("model").cloned().unwrap_or(Value::Null),
                "reasoningEffort": item.get("reasoningEffort").cloned().unwrap_or(Value::Null),
            })),
            output: item
                .get("agentsStates")
                .filter(|value| !is_empty_json_value(value))
                .map(render_json_value),
            error: None,
            raw: None,
        },
        _ => return None,
    };

    serde_json::to_string(&update).ok()
}

fn normalize_codex_tool_status(status: &str) -> String {
    match status {
        "inProgress" => "running".to_string(),
        "completed" => "completed".to_string(),
        "failed" => "error".to_string(),
        "declined" => "declined".to_string(),
        other => non_empty(other).unwrap_or("updated").to_string(),
    }
}

fn command_execution_error(item: &Value) -> Option<String> {
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if status != "failed" && status != "declined" {
        return None;
    }
    item.get("aggregatedOutput")
        .and_then(Value::as_str)
        .and_then(non_empty)
        .map(str::to_string)
        .or_else(|| Some(format!("Command {status}")))
}

fn command_execution_raw(item: &Value) -> Option<String> {
    let exit_code = item.get("exitCode").and_then(Value::as_i64);
    let duration = item.get("durationMs").and_then(Value::as_i64);
    let process_id = item
        .get("processId")
        .and_then(Value::as_str)
        .and_then(non_empty);
    let mut lines = Vec::new();
    if let Some(exit_code) = exit_code {
        lines.push(format!("exit_code: {exit_code}"));
    }
    if let Some(duration) = duration {
        lines.push(format!("duration_ms: {duration}"));
    }
    if let Some(process_id) = process_id {
        lines.push(format!("process_id: {process_id}"));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn prefer_output(primary: Option<&str>, streamed_output: &str) -> Option<String> {
    primary
        .and_then(non_empty)
        .map(str::to_string)
        .or_else(|| non_empty(streamed_output).map(str::to_string))
}

fn render_json_value(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn is_empty_json_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        Value::Object(items) => items.is_empty(),
        Value::String(text) => text.trim().is_empty(),
        _ => false,
    }
}
