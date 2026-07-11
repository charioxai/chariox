use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use crate::session::unix_epoch_ms;

const CLAUDE_HOOK_PERMISSION_TOMBSTONE_TTL_MS: u64 = 30_000;

pub(super) fn claude_native_marker(context_file: &str) -> Option<String> {
    let marker = std::path::Path::new(context_file).with_file_name("active-prompt-id");
    fs::read_to_string(marker)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn write_claude_native_marker(context_file: &str, value: &str) {
    let marker = std::path::Path::new(context_file).with_file_name("active-prompt-id");
    let _ = fs::write(marker, value);
}

pub(super) fn claude_headless_submit_retry_path(context_file: &str) -> Option<PathBuf> {
    std::path::Path::new(context_file)
        .parent()
        .map(|root| root.join("headless-submit-retry.json"))
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct ClaudeHeadlessSubmitRetry {
    pub(super) prompt_id: String,
    pub(super) count: u8,
    pub(super) last_attempt_ms: u64,
}

pub(super) fn read_claude_headless_submit_retry(context_file: &str) -> ClaudeHeadlessSubmitRetry {
    let Some(path) = claude_headless_submit_retry_path(context_file) else {
        return ClaudeHeadlessSubmitRetry::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub(super) fn write_claude_headless_submit_retry(
    context_file: &str,
    prompt_id: &str,
    count: u8,
    last_attempt_ms: u64,
) {
    let Some(path) = claude_headless_submit_retry_path(context_file) else {
        return;
    };
    let payload = ClaudeHeadlessSubmitRetry {
        prompt_id: prompt_id.to_string(),
        count,
        last_attempt_ms,
    };
    if let Ok(raw) = serde_json::to_string(&payload) {
        let _ = fs::write(path, raw);
    }
}

pub(super) fn write_claude_headless_startup_wait_marker(context_file: &str) {
    write_claude_native_marker(context_file, &format!("startup-wait:{}", unix_epoch_ms()));
}

pub(super) fn append_claude_headless_debug(context_file: &str, label: &str, value: &str) {
    if std::env::var_os("ARROBA_CLAUDE_HEADLESS_DEBUG").is_none() {
        return;
    }
    let Some(root) = std::path::Path::new(context_file).parent() else {
        return;
    };
    let path = root.join("headless-debug.log");
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| {
            use std::io::Write;
            writeln!(file, "[{}] {label}: {value}", unix_epoch_ms())
        });
}

fn claude_permission_input_dir(context_file: &str) -> Option<PathBuf> {
    std::path::Path::new(context_file)
        .parent()
        .map(|root| root.join("permission-inputs"))
}

pub(super) fn claude_permission_recent_file(context_file: &str) -> Option<PathBuf> {
    std::path::Path::new(context_file)
        .parent()
        .map(|root| root.join("permission-recent.txt"))
}

fn claude_hook_permission_tombstone_file(context_file: &str) -> Option<PathBuf> {
    std::path::Path::new(context_file)
        .parent()
        .map(|root| root.join("hook-permission-tombstone.json"))
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct ClaudeHookPermissionTombstone {
    recorded_at_ms: u64,
    tool_name: String,
    detail: String,
}

pub(super) fn write_claude_hook_permission_tombstone(context_file: &str, event: &Value) {
    let Some(path) = claude_hook_permission_tombstone_file(context_file) else {
        return;
    };
    let tool_name = event
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("tool")
        .to_string();
    let detail = event
        .get("tool_input")
        .and_then(Value::as_object)
        .and_then(|input| {
            input
                .get("command")
                .or_else(|| input.get("file_path"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string();
    let tombstone = ClaudeHookPermissionTombstone {
        recorded_at_ms: unix_epoch_ms(),
        tool_name,
        detail,
    };
    if let Ok(raw) = serde_json::to_string(&tombstone) {
        let _ = fs::write(path, raw);
    }
}

pub(super) fn clear_claude_hook_permission_tombstone(context_file: &str) {
    if let Some(path) = claude_hook_permission_tombstone_file(context_file) {
        let _ = fs::remove_file(path);
    }
}

pub(super) fn take_matching_claude_hook_permission_tombstone(
    context_file: &str,
    rendered: &str,
) -> bool {
    let Some(path) = claude_hook_permission_tombstone_file(context_file) else {
        return false;
    };
    let Some(tombstone) = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<ClaudeHookPermissionTombstone>(&raw).ok())
    else {
        let _ = fs::remove_file(path);
        return false;
    };
    if unix_epoch_ms().saturating_sub(tombstone.recorded_at_ms)
        > CLAUDE_HOOK_PERMISSION_TOMBSTONE_TTL_MS
    {
        let _ = fs::remove_file(path);
        return false;
    }
    let rendered = compact_claude_permission_text(rendered);
    let tool_name = compact_claude_permission_text(&tombstone.tool_name);
    let detail = compact_claude_permission_text(&tombstone.detail);
    if !rendered.contains(&tool_name) || (!detail.is_empty() && !rendered.contains(&detail)) {
        return false;
    }
    let _ = fs::remove_file(path);
    true
}

fn compact_claude_permission_text(value: &str) -> String {
    normalize_claude_rendered_permission_text(value)
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

pub(super) fn write_claude_hook_context_response(
    context_file: &str,
    request_id: &str,
    context: &str,
) {
    if request_id.trim().is_empty() {
        return;
    }
    let Some(root) = std::path::Path::new(context_file).parent() else {
        return;
    };
    let dir = root.join("hook-context-responses");
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join(format!("{request_id}.txt")), context);
}

pub(super) fn write_claude_permission_response(
    context_file: &str,
    request_id: &str,
    allowed: bool,
    reason: &str,
) {
    if request_id.trim().is_empty() {
        return;
    }
    let Some(root) = std::path::Path::new(context_file).parent() else {
        return;
    };
    let dir = root.join("permission-responses");
    let _ = fs::create_dir_all(&dir);
    let payload = serde_json::json!({
        "permissionDecision": if allowed { "allow" } else { "deny" },
        "permissionDecisionReason": reason,
    });
    let _ = fs::write(dir.join(format!("{request_id}.json")), payload.to_string());
}

pub(super) fn write_claude_permission_input(
    context_file: &str,
    interaction_id: &str,
    input: &[u8],
) {
    let Some(dir) = claude_permission_input_dir(context_file) else {
        return;
    };
    let _ = fs::create_dir_all(&dir);
    let filename = format!("{}.input", safe_claude_permission_filename(interaction_id));
    let _ = fs::write(dir.join(filename), input);
}

pub(super) fn take_claude_permission_inputs(context_file: &str) -> Vec<Vec<u8>> {
    let Some(dir) = claude_permission_input_dir(context_file) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("input"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let bytes = fs::read(&path).ok();
            let _ = fs::remove_file(path);
            bytes
        })
        .collect()
}

fn safe_claude_permission_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn timestamp_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

pub(super) fn should_bridge_claude_permission(event: &Value) -> bool {
    let Some(tool_name) = event
        .get("tool_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if tool_name.starts_with("mcp__arroba__") || tool_name.starts_with("arroba.") {
        return false;
    }
    !(event.get("hook_event_name").and_then(Value::as_str) == Some("PreToolUse")
        && event.get("permission_mode").and_then(Value::as_str) == Some("bypassPermissions"))
}

pub(super) fn format_claude_permission_message(event: &Value) -> String {
    let tool_name = event
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("tool");
    let permission_mode = event.get("permission_mode").and_then(Value::as_str);
    let details = event
        .get("tool_input")
        .map(format_claude_tool_input)
        .filter(|value| !value.trim().is_empty());
    let mut pieces = vec![format!("Claude Code wants to run {tool_name}.")];
    if let Some(permission_mode) = permission_mode {
        pieces.push(format!("Permission mode: {permission_mode}."));
    }
    if let Some(details) = details {
        pieces.push(String::new());
        pieces.push(details);
    }
    pieces.join("\n")
}

fn format_claude_tool_input(input: &Value) -> String {
    let Some(record) = input.as_object() else {
        return String::new();
    };
    if let Some(command) = record.get("command").and_then(Value::as_str) {
        return ["Command:", "", command].join("\n");
    }
    if let Some(file_path) = record.get("file_path").and_then(Value::as_str) {
        let mut pieces = vec![format!("File: {file_path}")];
        if let Some(old_string) = record.get("old_string").and_then(Value::as_str) {
            pieces.extend([String::new(), "Old:".to_string(), old_string.to_string()]);
        }
        if let Some(new_string) = record.get("new_string").and_then(Value::as_str) {
            pieces.extend([String::new(), "New:".to_string(), new_string.to_string()]);
        }
        if let Some(content) = record.get("content").and_then(Value::as_str) {
            pieces.extend([String::new(), "Content:".to_string(), content.to_string()]);
        }
        return pieces.join("\n");
    }
    serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
}

pub(super) fn claude_rendered_permission_visible(text: &str) -> bool {
    let normalized = normalize_claude_rendered_permission_text(text);
    let compact = normalized.replace(' ', "");
    (normalized.contains("Do you want to proceed?") || compact.contains("Doyouwanttoproceed?"))
        && (normalized.contains("1. Yes") || compact.contains("1.Yes"))
        && (normalized.contains("3. No") || compact.contains("3.No"))
}

pub(super) fn claude_headless_workspace_trust_visible(text: &str) -> bool {
    let normalized = normalize_claude_rendered_permission_text(text);
    let normalized_lower = normalized.to_ascii_lowercase();
    let compact = normalized_lower.replace(' ', "");
    (normalized_lower.contains("quick safety check") || compact.contains("quicksafetycheck"))
        && (normalized_lower.contains("trust this folder") || compact.contains("trustthisfolder"))
}

pub(super) fn claude_headless_bypass_confirmation_visible(text: &str) -> bool {
    let normalized = normalize_claude_rendered_permission_text(text);
    let normalized_lower = normalized.to_ascii_lowercase();
    let compact = normalized_lower.replace(' ', "");
    (normalized_lower.contains("bypass permissions mode")
        || compact.contains("bypasspermissionsmode"))
        && (normalized_lower.contains("yes, i accept") || compact.contains("yes,iaccept"))
}

pub(super) fn update_claude_permission_recent(context_file: &str, rendered: &str) -> String {
    let normalized = normalize_claude_rendered_permission_text(rendered);
    if normalized.trim().is_empty() {
        return claude_permission_recent_file(context_file)
            .and_then(|path| fs::read_to_string(path).ok())
            .unwrap_or_default();
    }
    let Some(path) = claude_permission_recent_file(context_file) else {
        return normalized;
    };
    let mut recent = fs::read_to_string(&path).unwrap_or_default();
    recent.push(' ');
    recent.push_str(&normalized);
    if recent.chars().count() > 4000 {
        recent = recent
            .chars()
            .rev()
            .take(4000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
    }
    let _ = fs::write(path, &recent);
    recent
}

pub(super) fn clear_claude_permission_recent(context_file: &str) {
    if let Some(path) = claude_permission_recent_file(context_file) {
        let _ = fs::write(path, "");
    }
}

fn normalize_claude_rendered_permission_text(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    while let Some(next) = chars.next() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            let _ = chars.next();
                            break;
                        }
                    }
                }
                Some(_) | None => {}
            }
            continue;
        }
        if ch.is_control() {
            if !output.ends_with(' ') {
                output.push(' ');
            }
            continue;
        }
        if ch.is_whitespace() {
            if !output.ends_with(' ') {
                output.push(' ');
            }
        } else {
            output.push(ch);
        }
    }
    output
}

pub(super) fn claude_headless_composer_visible(text: &str) -> bool {
    let normalized = normalize_claude_rendered_permission_text(text);
    let normalized_lower = normalized.to_ascii_lowercase();
    let compact = normalized_lower.replace(' ', "");
    (normalized_lower.contains("try \"write a test for")
        || compact.contains("try\"writeatestfor")
        || normalized_lower.contains("bypass permissions on")
        || compact.contains("bypasspermissionson")
        || normalized_lower.contains("for shortcuts")
        || compact.contains("forshortcuts"))
        && !(claude_headless_workspace_trust_visible(&normalized)
            || claude_headless_bypass_confirmation_visible(&normalized))
}

pub(super) fn claude_headless_prompt_waiting_in_composer(
    text: &str,
    expected_prompt: &str,
) -> bool {
    let normalized = normalize_claude_rendered_permission_text(text);
    let normalized_lower = normalized.to_ascii_lowercase();
    let compact = normalized_lower.replace(' ', "");
    let expected = normalize_claude_rendered_permission_text(expected_prompt)
        .trim()
        .to_ascii_lowercase();
    let expected_compact = expected.replace(' ', "");
    let direct_prompt_waiting = !expected.is_empty()
        && (normalized_lower.trim_end().ends_with(&expected)
            || compact.trim_end().ends_with(&expected_compact));
    direct_prompt_waiting
        || normalized_lower.contains("[pasted text")
        || compact.contains("[pastedtext")
        || normalized_lower.contains("paste again to expand")
        || compact.contains("pasteagaintoexpand")
}

pub(super) fn normalize_claude_visible_prompt_for_headless(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn extract_native_hidden_instructions(prompt: &str) -> String {
    let start = crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_START;
    let end = crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_END;
    let Some(start_index) = prompt.find(start) else {
        return String::new();
    };
    let after_start = start_index + start.len();
    let Some(end_index) = prompt[after_start..]
        .find(end)
        .map(|index| after_start + index)
    else {
        return prompt[after_start..].trim().to_string();
    };
    prompt[after_start..end_index].trim().to_string()
}

pub(super) fn redact_native_hidden_instructions(prompt: &str) -> String {
    let start = crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_START;
    let end = crate::provider::NATIVE_TUI_HIDDEN_INSTRUCTIONS_END;
    let Some(start_index) = prompt.find(start) else {
        return prompt.to_string();
    };
    let after_start = start_index + start.len();
    let Some(end_index) = prompt[after_start..]
        .find(end)
        .map(|index| after_start + index + end.len())
    else {
        return prompt[..start_index].to_string();
    };
    let mut redacted = String::new();
    redacted.push_str(&prompt[..start_index]);
    redacted.push_str(&prompt[end_index..]);
    redacted.replace("\n\n\n", "\n\n")
}
