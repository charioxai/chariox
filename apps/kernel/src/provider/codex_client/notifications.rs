//! Codex JSON-RPC notification shape parsing.

use serde_json::{json, Value};

use crate::provider::ProviderRunTokenUsage;

use super::JsonRpcMessage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexNotification {
    AgentMessageDelta {
        item_id: String,
        delta: String,
    },
    ReasoningTextDelta {
        item_id: String,
        delta: String,
    },
    ReasoningSummaryTextDelta {
        item_id: String,
        delta: String,
    },
    ReasoningSummaryPartAdded {
        item_id: String,
        summary_index: usize,
    },
    ItemStarted {
        item: Value,
    },
    ItemCompleted {
        item: Value,
    },
    ExecCommandStarted {
        call_id: String,
        command: Value,
        cwd: Option<String>,
    },
    ExecCommandCompleted {
        call_id: String,
        command: Value,
        cwd: Option<String>,
        output: Option<String>,
        exit_code: Option<i64>,
        success: Option<bool>,
        stderr: Option<String>,
    },
    ExecCommandOutputDelta {
        call_id: String,
        chunk: String,
    },
    CommandExecutionOutputDelta {
        item_id: String,
        delta: String,
    },
    FileChangeOutputDelta {
        item_id: String,
        delta: String,
    },
    McpToolCallProgress {
        item_id: String,
        message: String,
    },
    TokenUsageUpdated {
        thread_id: String,
        turn_id: String,
        usage: ProviderRunTokenUsage,
    },
    TurnStarted {
        turn_id: String,
    },
    TurnCompleted {
        turn_id: String,
        status: String,
        error_message: Option<String>,
    },
    Error {
        message: String,
    },
}

pub(super) fn parse_notification(message: JsonRpcMessage) -> Option<CodexNotification> {
    let method = message.method?;
    let params = message.params.unwrap_or(Value::Null);
    match method.as_str() {
        "item/agentMessage/delta" => Some(CodexNotification::AgentMessageDelta {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            delta: params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "item/reasoning/textDelta" => Some(CodexNotification::ReasoningTextDelta {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            delta: params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "item/reasoning/summaryTextDelta" => Some(CodexNotification::ReasoningSummaryTextDelta {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            delta: params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "item/reasoning/summaryPartAdded" => Some(CodexNotification::ReasoningSummaryPartAdded {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            summary_index: params
                .get("summaryIndex")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
        }),
        "item/started" => Some(CodexNotification::ItemStarted {
            item: params.get("item").cloned().unwrap_or(Value::Null),
        }),
        "item/completed" => Some(CodexNotification::ItemCompleted {
            item: params.get("item").cloned().unwrap_or(Value::Null),
        }),
        "codex/event/exec_command_begin" => {
            let msg = params.get("msg").unwrap_or(&Value::Null);
            Some(CodexNotification::ExecCommandStarted {
                call_id: msg
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                command: msg.get("command").cloned().unwrap_or(Value::Null),
                cwd: msg.get("cwd").and_then(Value::as_str).map(str::to_string),
            })
        }
        "codex/event/exec_command_end" => {
            let msg = params.get("msg").unwrap_or(&Value::Null);
            Some(CodexNotification::ExecCommandCompleted {
                call_id: msg
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                command: msg.get("command").cloned().unwrap_or(Value::Null),
                cwd: msg.get("cwd").and_then(Value::as_str).map(str::to_string),
                output: msg
                    .get("aggregated_output")
                    .or_else(|| msg.get("aggregatedOutput"))
                    .or_else(|| msg.get("formatted_output"))
                    .or_else(|| msg.get("stdout"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                exit_code: msg
                    .get("exit_code")
                    .or_else(|| msg.get("exitCode"))
                    .and_then(Value::as_i64),
                success: msg.get("success").and_then(Value::as_bool),
                stderr: msg
                    .get("stderr")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        }
        "codex/event/exec_command_output_delta" => {
            let msg = params.get("msg").unwrap_or(&Value::Null);
            Some(CodexNotification::ExecCommandOutputDelta {
                call_id: msg
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                chunk: msg
                    .get("chunk")
                    .or_else(|| msg.get("delta"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        }
        "codex/event/patch_apply_begin" => {
            let msg = params.get("msg").unwrap_or(&Value::Null);
            Some(CodexNotification::ItemStarted {
                item: legacy_codex_file_change_item(&params, msg, "inProgress"),
            })
        }
        "codex/event/patch_apply_end" => {
            let msg = params.get("msg").unwrap_or(&Value::Null);
            Some(CodexNotification::ItemCompleted {
                item: legacy_codex_file_change_item(
                    &params,
                    msg,
                    if msg.get("success").and_then(Value::as_bool) == Some(false) {
                        "failed"
                    } else {
                        "completed"
                    },
                ),
            })
        }
        "item/commandExecution/outputDelta" => {
            Some(CodexNotification::CommandExecutionOutputDelta {
                item_id: params
                    .get("itemId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                delta: params
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        }
        "item/fileChange/outputDelta" => Some(CodexNotification::FileChangeOutputDelta {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            delta: params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "item/mcpToolCall/progress" => Some(CodexNotification::McpToolCallProgress {
            item_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message: params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "thread/tokenUsage/updated" => {
            let token_usage = params.get("tokenUsage")?;
            Some(CodexNotification::TokenUsageUpdated {
                thread_id: params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                turn_id: params
                    .get("turnId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                usage: {
                    let total_tokens = token_usage
                        .get("total")
                        .and_then(|total| total.get("totalTokens"))
                        .and_then(Value::as_i64)
                        .and_then(|value| u64::try_from(value).ok());
                    let last_tokens = token_usage
                        .get("last")
                        .and_then(|last| last.get("totalTokens"))
                        .and_then(Value::as_i64)
                        .and_then(|value| u64::try_from(value).ok());
                    let context_window = token_usage
                        .get("modelContextWindow")
                        .and_then(Value::as_i64)
                        .and_then(|value| u64::try_from(value).ok());
                    let context_tokens = match (last_tokens, context_window) {
                        (Some(tokens), Some(window)) if tokens <= window => Some(tokens),
                        _ => None,
                    };

                    ProviderRunTokenUsage {
                        total_tokens,
                        last_tokens,
                        context_tokens,
                        context_window,
                    }
                },
            })
        }
        "turn/started" => Some(CodexNotification::TurnStarted {
            turn_id: params
                .get("turn")
                .and_then(|turn| turn.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "turn/completed" => parse_turn_completed_notification(&params),
        "error" => Some(CodexNotification::Error {
            message: params
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Codex reported an unknown error")
                .to_string(),
        }),
        _ => None,
    }
}

pub(super) fn rpc_error_message(message: &JsonRpcMessage) -> Option<String> {
    message
        .error
        .as_ref()
        .and_then(|error| error.message.clone())
}

fn optional_codex_turn_id(turn: Option<&Value>) -> Option<String> {
    turn.and_then(|turn| turn.get("id"))
        .and_then(Value::as_str)
        .filter(|turn_id| !turn_id.is_empty())
        .map(str::to_string)
}

fn parse_turn_completed_notification(params: &Value) -> Option<CodexNotification> {
    let turn = params.get("turn")?;
    Some(CodexNotification::TurnCompleted {
        turn_id: optional_codex_turn_id(Some(turn))?,
        status: turn
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("completed")
            .to_string(),
        error_message: turn
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn legacy_codex_file_change_item(params: &Value, msg: &Value, status: &str) -> Value {
    let id = msg
        .get("call_id")
        .or_else(|| msg.get("callId"))
        .or_else(|| msg.get("id"))
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("patch");
    json!({
        "type": "fileChange",
        "id": id,
        "status": status,
        "changes": msg.get("changes").cloned().unwrap_or_else(|| json!([])),
    })
}
