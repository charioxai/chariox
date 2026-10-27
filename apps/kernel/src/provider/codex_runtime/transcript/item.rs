use serde_json::Value;

pub(in crate::provider::codex_runtime) fn normalize_codex_item_type(
    raw_type: &str,
) -> Option<&str> {
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

pub(in crate::provider::codex_runtime) fn is_codex_tool_item(item: &Value) -> bool {
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

pub(in crate::provider::codex_runtime) fn codex_item_id(item: &Value) -> Option<&str> {
    item.get("id")
        .or_else(|| item.get("callId"))
        .or_else(|| item.get("call_id"))
        .and_then(Value::as_str)
        .filter(|item_id| !item_id.is_empty())
}

pub(in crate::provider::codex_runtime) fn codex_item_status_is_terminal(item: &Value) -> bool {
    matches!(
        item.get("status").and_then(Value::as_str),
        Some("completed" | "failed" | "canceled" | "cancelled")
    )
}
