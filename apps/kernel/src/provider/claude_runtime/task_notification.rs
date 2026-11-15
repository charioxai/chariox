//! Completion notifications from Claude's stream and native transcript share the same task IDs.
use serde_json::Value;

#[derive(Clone, Copy)]
pub(crate) struct BackgroundTaskEnd<'a> {
    pub(crate) tool_use_id: Option<&'a str>,
    pub(crate) task_id: Option<&'a str>,
}

fn terminal(status: Option<&str>) -> bool {
    matches!(status, Some("completed" | "failed" | "killed" | "stopped"))
}

pub(crate) fn finished_background_task(value: &Value) -> Option<BackgroundTaskEnd<'_>> {
    let kind = value.get("type")?.as_str()?;
    if kind == "system" && value.get("subtype")?.as_str()? == "task_notification" {
        return terminal(value.get("status").and_then(Value::as_str)).then(|| BackgroundTaskEnd {
            tool_use_id: value.get("tool_use_id").and_then(Value::as_str),
            task_id: value.get("task_id").and_then(Value::as_str),
        });
    }
    let content = match kind {
        "queue-operation" => value.get("content")?,
        "user" => value.get("message")?.get("content")?,
        _ => return None,
    };
    let text = content
        .as_str()
        .or_else(|| {
            let block = content.as_array()?.first()?;
            if block.get("type")?.as_str()? != "text" {
                return None;
            }
            block.get("text")?.as_str()
        })?
        .trim_start();
    // Nested background command notifications carry this provider-written preamble.
    let notification = if text.starts_with("[SYSTEM NOTIFICATION - NOT USER INPUT]") {
        text.split_once("<task-notification>")?.1
    } else {
        text.strip_prefix("<task-notification>")?
    };
    let tag = |name: &str| {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let rest = notification.split_once(open.as_str())?.1;
        Some(rest.split_once(close.as_str())?.0.trim())
    };
    terminal(tag("status")).then(|| BackgroundTaskEnd {
        tool_use_id: tag("tool-use-id"),
        task_id: tag("task-id"),
    })
}
