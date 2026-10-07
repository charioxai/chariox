//! Codex JSON-RPC notification shape parsing.

use serde_json::{json, Value};

use crate::provider::ProviderRunTokenUsage;

use super::JsonRpcMessage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexNotification {
    /// Provider-local correlation, never serialized in the Chariox protocol.
    TurnScoped {
        turn_id: String,
        notification: Box<CodexNotification>,
    },
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
        items: Vec<Value>,
    },
    /// Legacy app-server terminal signal.  It does not always carry the
    /// structured `turn/completed` payload, so the runtime uses it only to
    /// trigger an authoritative `thread/turns/list` backfill.
    TaskComplete {
        turn_id: Option<String>,
    },
    TurnAborted {
        reason: Option<String>,
    },
    Error {
        message: String,
    },
}

pub(super) fn parse_notification(message: JsonRpcMessage) -> Option<CodexNotification> {
    let method = message.method?;
    let params = message.params.unwrap_or(Value::Null);
    let notification = match method.as_str() {
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
            // MP-08 / MP-10 / MP-11: numeric official counters only. Never log
            // an arbitrary provider payload, prompt, or account material.
            if let Some(total) = token_usage.get("total") {
                let counters = [
                    "inputTokens",
                    "cachedInputTokens",
                    "outputTokens",
                    "reasoningOutputTokens",
                    "totalTokens",
                ]
                .into_iter()
                .filter_map(|key| total.get(key).and_then(Value::as_u64).map(|n| (key, n)))
                .collect::<std::collections::BTreeMap<_, _>>();
                crate::logging::debug_with_fields(
                    "daemon.provider.codex",
                    "official cumulative usage counters",
                    serde_json::json!({"counters": counters}),
                );
            }
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
                        accounting: token_usage
                            .get("total")
                            .and_then(crate::usage_accounting::codex_usage),
                        turn_accounting: None,
                        total_tokens,
                        last_tokens,
                        context_tokens,
                        context_window,
                    }
                },
            })
        }
        "turn/started" => Some(CodexNotification::TurnStarted {
            turn_id: codex_turn_id(&params)?,
        }),
        "turn/completed" => parse_turn_completed_notification(&params),
        "codex/event/task_complete" => Some(CodexNotification::TaskComplete {
            turn_id: params
                .get("turnId")
                .or_else(|| params.get("turn_id"))
                .or_else(|| params.get("id"))
                .or_else(|| params.get("msg").and_then(|msg| msg.get("turnId")))
                .or_else(|| params.get("msg").and_then(|msg| msg.get("turn_id")))
                .and_then(Value::as_str)
                .map(str::to_string),
        }),
        "codex/event/turn_aborted" => Some(CodexNotification::TurnAborted {
            reason: params
                .get("msg")
                .and_then(|message| message.get("reason"))
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
                .map(str::to_string),
        }),
        "error" => {
            let error = params.get("error").unwrap_or(&Value::Null);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Codex reported an unknown error");
            // A retried error is progress, not a failure; keep its text as sent.
            let message = if params.get("willRetry").and_then(Value::as_bool) == Some(true) {
                message.to_string()
            } else {
                codex_error_text(error, message)
            };
            Some(CodexNotification::Error { message })
        }
        _ => None,
    }?;
    // Output notifications carry their owning turn separately from the item.
    // Keep that identity across buffering, including after an interrupt ACK.
    let turn_id = params
        .get("turnId")
        .or_else(|| params.get("turn_id"))
        .or_else(|| params.get("msg").and_then(|msg| msg.get("turnId")))
        .or_else(|| params.get("msg").and_then(|msg| msg.get("turn_id")))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    Some(match (&notification, turn_id) {
        (
            CodexNotification::TurnStarted { .. }
            | CodexNotification::TurnCompleted { .. }
            | CodexNotification::TaskComplete { .. }
            | CodexNotification::TokenUsageUpdated { .. },
            _,
        )
        | (_, None) => notification,
        (_, Some(turn_id)) => CodexNotification::TurnScoped {
            turn_id: turn_id.to_string(),
            notification: Box::new(notification),
        },
    })
}

/// Codex's message for a turn error, framed with its structured
/// `codexErrorInfo` (the rollout's `codex_error_info`) when present.
pub(crate) fn codex_error_text(error: &Value, message: &str) -> String {
    match codex_error_info_code(error) {
        Some(code) => crate::provider::provider_coded_failure_text("Codex", &code, message),
        None => message.to_string(),
    }
}

/// The code in snake case: a string variant, or the single key of an object
/// variant such as `{"httpConnectionFailed": {"httpStatusCode": 502}}`.
fn codex_error_info_code(error: &Value) -> Option<String> {
    let info = error
        .get("codexErrorInfo")
        .or_else(|| error.get("codex_error_info"))?;
    let variant = match info {
        Value::String(code) => code.as_str(),
        Value::Object(fields) if fields.len() == 1 => fields.keys().next()?.as_str(),
        _ => return None,
    };
    let mut code = String::with_capacity(variant.len() + 4);
    for ch in variant.chars() {
        if ch.is_ascii_uppercase() {
            if !code.is_empty() {
                code.push('_');
            }
            code.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            code.push(ch);
        } else {
            return None;
        }
    }
    (!code.is_empty()).then_some(code)
}

pub(super) fn rpc_error_message(message: &JsonRpcMessage) -> Option<String> {
    message
        .error
        .as_ref()
        .and_then(|error| error.message.clone())
}

fn optional_codex_turn_id(turn: Option<&Value>) -> Option<String> {
    turn.and_then(|turn| {
        turn.get("id")
            .or_else(|| turn.get("turnId"))
            .or_else(|| turn.get("turn_id"))
    })
    .and_then(Value::as_str)
    .filter(|turn_id| !turn_id.is_empty())
    .map(str::to_string)
}

fn codex_turn_id(params: &Value) -> Option<String> {
    optional_codex_turn_id(params.get("turn")).or_else(|| optional_codex_turn_id(Some(params)))
}

fn parse_turn_completed_notification(params: &Value) -> Option<CodexNotification> {
    // The current app-server schema nests lifecycle fields under `turn`, while
    // older/managed servers emitted the same fields directly in `params`.
    // Accept both shapes so a terminal notification cannot disappear merely
    // because the provider was upgraded independently of the kernel.
    let turn = params.get("turn").unwrap_or(params);
    Some(CodexNotification::TurnCompleted {
        turn_id: codex_turn_id(params)?,
        status: turn
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("completed")
            .to_string(),
        error_message: turn
            .get("error")
            .and_then(|error| {
                let message = error.get("message").and_then(Value::as_str)?;
                Some(codex_error_text(error, message))
            })
            .or_else(|| {
                turn.get("errorMessage")
                    .or_else(|| turn.get("error_message"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }),
        items: turn
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
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
