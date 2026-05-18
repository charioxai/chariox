use serde_json::Value;

pub(super) fn command_execution_approval_body(params: &Value) -> String {
    let command = params
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("<unknown command>");
    let cwd = params
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut body = format!("Approve command execution?\n\n{command}");
    if let Some(cwd) = cwd {
        body.push_str(&format!("\n\ncwd: {cwd}"));
    }
    if let Some(reason) = reason {
        body.push_str(&format!("\n\nreason: {reason}"));
    }
    body
}

pub(super) fn file_change_approval_body(params: &Value) -> String {
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let changes = params
        .get("changes")
        .map(render_pretty_json)
        .filter(|value| !value.trim().is_empty());
    let mut body = "Approve file changes?".to_string();
    if let Some(reason) = reason {
        body.push_str(&format!("\n\nreason: {reason}"));
    }
    if let Some(changes) = changes {
        body.push_str(&format!("\n\nchanges:\n{changes}"));
    }
    body
}

pub(super) fn exec_command_review_body(params: &Value) -> String {
    let command = params
        .get("command")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "<unknown command>".to_string());
    let cwd = params
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let parsed = params
        .get("parsedCmd")
        .map(render_pretty_json)
        .filter(|value| !value.trim().is_empty());
    let mut body = format!("Approve command execution?\n\n{command}");
    if let Some(cwd) = cwd {
        body.push_str(&format!("\n\ncwd: {cwd}"));
    }
    if let Some(reason) = reason {
        body.push_str(&format!("\n\nreason: {reason}"));
    }
    if let Some(parsed) = parsed {
        body.push_str(&format!("\n\nparsed:\n{parsed}"));
    }
    body
}

pub(super) fn apply_patch_review_body(params: &Value) -> String {
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let grant_root = params
        .get("grantRoot")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let changes = params
        .get("fileChanges")
        .map(render_pretty_json)
        .filter(|value| !value.trim().is_empty());
    let mut body = "Approve file changes?".to_string();
    if let Some(reason) = reason {
        body.push_str(&format!("\n\nreason: {reason}"));
    }
    if let Some(grant_root) = grant_root {
        body.push_str(&format!("\n\ngrant_root: {grant_root}"));
    }
    if let Some(changes) = changes {
        body.push_str(&format!("\n\nchanges:\n{changes}"));
    }
    body
}

fn render_pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}
