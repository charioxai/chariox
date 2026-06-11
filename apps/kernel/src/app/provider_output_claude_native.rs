use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::Value;

use super::provider_output_fanout::ProviderOutputFanout;
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{
    ProviderNativeInteractionBridge, ProviderPromptSignalBatch, ProviderResumeState,
    RuntimeProviderRun,
};
use crate::session::{
    unix_epoch_ms, PromptAttachment, RuntimeInteraction, RuntimeInteractionChoice,
    RuntimeInteractionChoiceStyle, RuntimeInteractionKind, RuntimeInteractionLevel,
};
use crate::terminal::TerminalOutputKind;

const CLAUDE_ATTACHMENT_CONTEXT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ClaudeTranscriptCursor {
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
struct ClaudeTranscriptChunk {
    kind: TerminalOutputKind,
    merge_key_suffix: String,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ClaudeTranscriptDrain {
    chunks: Vec<ClaudeTranscriptChunk>,
    assistant_message_ids: Vec<String>,
    session_id: Option<String>,
    model: Option<String>,
}

fn claude_transcript_cursor_path(context_file: &str) -> Option<PathBuf> {
    std::path::Path::new(context_file)
        .parent()
        .map(|root| root.join("transcript-cursor.json"))
}

fn load_claude_transcript_cursor(context_file: &str) -> ClaudeTranscriptCursor {
    let Some(path) = claude_transcript_cursor_path(context_file) else {
        return ClaudeTranscriptCursor::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_claude_transcript_cursor(context_file: &str, cursor: &ClaudeTranscriptCursor) {
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

fn drain_claude_transcript_file(
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

fn claude_native_marker(context_file: &str) -> Option<String> {
    let marker = std::path::Path::new(context_file).with_file_name("active-prompt-id");
    fs::read_to_string(marker)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn write_claude_native_marker(context_file: &str, value: &str) {
    let marker = std::path::Path::new(context_file).with_file_name("active-prompt-id");
    let _ = fs::write(marker, value);
}

fn write_claude_headless_startup_wait_marker(context_file: &str) {
    write_claude_native_marker(context_file, &format!("startup-wait:{}", unix_epoch_ms()));
}

fn append_claude_headless_debug(context_file: &str, label: &str, value: &str) {
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

fn claude_permission_recent_file(context_file: &str) -> Option<PathBuf> {
    std::path::Path::new(context_file)
        .parent()
        .map(|root| root.join("permission-recent.txt"))
}

fn write_claude_hook_context_response(context_file: &str, request_id: &str, context: &str) {
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

fn write_claude_permission_response(
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

fn write_claude_permission_input(context_file: &str, interaction_id: &str, input: &[u8]) {
    let Some(dir) = claude_permission_input_dir(context_file) else {
        return;
    };
    let _ = fs::create_dir_all(&dir);
    let filename = format!("{}.input", safe_claude_permission_filename(interaction_id));
    let _ = fs::write(dir.join(filename), input);
}

fn take_claude_permission_inputs(context_file: &str) -> Vec<Vec<u8>> {
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

fn timestamp_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn should_bridge_claude_permission(event: &Value) -> bool {
    let Some(tool_name) = event.get("tool_name").and_then(Value::as_str) else {
        return false;
    };
    matches!(
        tool_name,
        "Bash" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit"
    )
}

fn format_claude_permission_message(event: &Value) -> String {
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

fn claude_rendered_permission_visible(text: &str) -> bool {
    let normalized = normalize_claude_rendered_permission_text(text);
    let compact = normalized.replace(' ', "");
    (normalized.contains("Bash command") || compact.contains("Bashcommand"))
        && (normalized.contains("Do you want to proceed?")
            || compact.contains("Doyouwanttoproceed?"))
        && (normalized.contains("1. Yes") || compact.contains("1.Yes"))
        && (normalized.contains("3. No") || compact.contains("3.No"))
}

fn claude_headless_workspace_trust_visible(text: &str) -> bool {
    let normalized = normalize_claude_rendered_permission_text(text);
    let normalized_lower = normalized.to_ascii_lowercase();
    let compact = normalized_lower.replace(' ', "");
    (normalized_lower.contains("quick safety check") || compact.contains("quicksafetycheck"))
        && (normalized_lower.contains("trust this folder") || compact.contains("trustthisfolder"))
}

fn claude_headless_bypass_confirmation_visible(text: &str) -> bool {
    let normalized = normalize_claude_rendered_permission_text(text);
    let normalized_lower = normalized.to_ascii_lowercase();
    let compact = normalized_lower.replace(' ', "");
    (normalized_lower.contains("bypass permissions mode")
        || compact.contains("bypasspermissionsmode"))
        && (normalized_lower.contains("yes, i accept") || compact.contains("yes,iaccept"))
}

fn update_claude_permission_recent(context_file: &str, rendered: &str) -> String {
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

fn clear_claude_permission_recent(context_file: &str) {
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

fn claude_headless_composer_visible(text: &str) -> bool {
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

fn normalize_claude_visible_prompt_for_headless(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_native_hidden_instructions(prompt: &str) -> String {
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

fn redact_native_hidden_instructions(prompt: &str) -> String {
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

pub(crate) struct ProviderOutputClaudeNativeBridge<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderOutputClaudeNativeBridge<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn process(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
    ) -> Result<(), DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(());
        };
        let Some(events_file) = provider_run.pty_env().get("ARROBA_CLAUDE_NATIVE_EVENTS") else {
            return Ok(());
        };
        let Some(context_file) = provider_run.pty_env().get("ARROBA_CLAUDE_NATIVE_CONTEXT") else {
            return Ok(());
        };

        for input in take_claude_permission_inputs(context_file) {
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, &input)?;
            write_claude_native_marker(context_file, "");
        }
        self.inject_pending_prompt(
            session_id,
            provider_run_id,
            &agent_id,
            context_file,
            provider_run,
        )?;
        if provider_run.provider() == "claude-headless" {
            self.drain_known_headless_transcripts(session_id, provider_run_id, context_file)?;
        }

        let events_path = std::path::Path::new(events_file);
        let raw = fs::read_to_string(events_path).unwrap_or_default();
        if raw.trim().is_empty() {
            return Ok(());
        }
        let _ = fs::write(events_path, "");
        let attachment_id = self
            .app
            .attachments
            .list_session_attachment_ids(session_id)
            .into_iter()
            .next();

        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if provider_run.provider() == "claude-headless" {
                if let Some(transcript_path) = event
                    .get("transcript_path")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    self.drain_headless_transcript(
                        session_id,
                        provider_run_id,
                        context_file,
                        transcript_path,
                    )?;
                }
            }
            let event_name = event
                .get("hook_event_name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if event_name == "UserPromptSubmit" {
                let Some(prompt) = event
                    .get("prompt")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|prompt| !prompt.is_empty())
                else {
                    continue;
                };
                let active_prompt_id = self
                    .app
                    .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?
                    .map(|prompt| prompt.id().to_string());
                let marker = claude_native_marker(context_file);
                if active_prompt_id
                    .as_deref()
                    .is_some_and(|id| marker.as_deref() == Some(&format!("injected:{id}")))
                {
                    continue;
                }
                if let Some(request_id) =
                    event.get("hook_context_request_id").and_then(Value::as_str)
                {
                    let context =
                        self.claude_native_prompt_context(session_id, &agent_id, prompt)?;
                    write_claude_hook_context_response(context_file, request_id, &context);
                }
                let Some(attachment_id) = attachment_id.as_deref() else {
                    continue;
                };
                let attachments = extract_claude_native_prompt_attachments(
                    prompt,
                    provider_run.working_directory().map(PathBuf::as_path),
                );
                let outcome = self.app.record_native_prompt_started_with_attachments(
                    session_id,
                    attachment_id,
                    &agent_id,
                    prompt,
                    attachments,
                )?;
                if let crate::session::PromptSubmissionOutcome::Started { prompt } = outcome {
                    write_claude_native_marker(context_file, &format!("native:{}", prompt.id()));
                }
            } else if matches!(event_name, "Stop" | "StopFailure" | "SessionEnd") {
                if provider_run.provider() == "claude-headless" {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    self.drain_known_headless_transcripts(
                        session_id,
                        provider_run_id,
                        context_file,
                    )?;
                }
                let _ = fs::write(context_file, "");
                write_claude_native_marker(context_file, "");
                let _ =
                    self.app
                        .complete_active_prompt(session_id, &agent_id, Some(provider_run_id));
                if provider_run.provider() == "claude-headless" {
                    if let Some(next_prompt) = self
                        .app
                        .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?
                    {
                        crate::logging::debug_with_fields(
                            "daemon.claude_headless",
                            "marked post-stop queued prompt ready",
                            serde_json::json!({
                                "session_id": session_id,
                                "provider_run_id": provider_run_id,
                                "agent_id": agent_id,
                                "prompt_id": next_prompt.id(),
                            }),
                        );
                        write_claude_native_marker(
                            context_file,
                            &format!("post-stop-ready:{}", next_prompt.id()),
                        );
                    }
                }
            } else if matches!(event_name, "PreToolUse" | "PermissionRequest") {
                self.resolve_permission_event(
                    session_id,
                    provider_run_id,
                    &agent_id,
                    context_file,
                    native_interaction_bridge.clone(),
                    &event,
                )?;
            }
        }
        if provider_run.provider() == "claude-headless" {
            self.drain_known_headless_transcripts(session_id, provider_run_id, context_file)?;
        }
        Ok(())
    }

    fn drain_known_headless_transcripts(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        context_file: &str,
    ) -> Result<(), DaemonError> {
        let paths = load_claude_transcript_cursor(context_file)
            .files
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for path in paths {
            self.drain_headless_transcript(session_id, provider_run_id, context_file, &path)?;
        }
        Ok(())
    }

    fn drain_headless_transcript(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        context_file: &str,
        transcript_path: &str,
    ) -> Result<(), DaemonError> {
        let mut cursor = load_claude_transcript_cursor(context_file);
        let drain = drain_claude_transcript_file(transcript_path, &mut cursor);
        save_claude_transcript_cursor(context_file, &cursor);
        if drain.chunks.is_empty()
            && drain.assistant_message_ids.is_empty()
            && drain.session_id.is_none()
            && drain.model.is_none()
        {
            return Ok(());
        }

        let mut metadata = ProviderPromptSignalBatch::default();
        if let Some(session_id) = drain.session_id {
            metadata.resolved_resume_state =
                Some(ProviderResumeState::from_claude_session_id(session_id));
        }
        if let Some(model) = drain.model {
            metadata.resolved_model = Some(model);
            metadata.resolved_model_source = Some("claude.headless.transcript");
        }
        if metadata.resolved_resume_state.is_some() || metadata.resolved_model.is_some() {
            self.app
                .providers
                .apply_structured_output_metadata(provider_run_id, &metadata)?;
            if let Ok(run) = self.app.providers.get_run(provider_run_id) {
                self.app.update_provider_run_projection(run);
            }
        }

        let recipient_attachment_ids = self.app.attachments.list_session_attachment_ids(session_id);
        let fanout = ProviderOutputFanout::new(self.app);
        let mut saw_response_content = false;
        let mut saw_runtime_activity = false;
        for chunk in drain.chunks {
            if chunk.text.is_empty() {
                continue;
            }
            if matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput | TerminalOutputKind::ProviderReasoning
            ) {
                saw_response_content = true;
            }
            if matches!(
                chunk.kind,
                TerminalOutputKind::ProviderOutput
                    | TerminalOutputKind::ProviderReasoning
                    | TerminalOutputKind::ProviderTool
                    | TerminalOutputKind::ProviderStatus
            ) {
                saw_runtime_activity = true;
            }
            fanout.fan_out(
                session_id,
                provider_run_id,
                chunk.kind,
                Some(format!(
                    "claude-headless:{provider_run_id}:{}",
                    chunk.merge_key_suffix
                )),
                recipient_attachment_ids.clone(),
                chunk.text.as_bytes(),
            );
        }
        if saw_response_content {
            crate::transport::flow_control::note_prompt_response_content(self.app, provider_run_id);
        } else if saw_runtime_activity {
            crate::transport::flow_control::note_prompt_output(self.app, provider_run_id);
        }
        for message_id in drain.assistant_message_ids {
            ProviderOutputFanout::new(self.app).record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids.clone(),
                &message_id,
                unix_epoch_ms(),
            );
            crate::transport::flow_control::mark_prompt_completion_recorded(
                self.app,
                provider_run_id,
            );
        }
        Ok(())
    }

    pub(crate) fn process_terminal_output(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        provider_run: &RuntimeProviderRun,
        native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
        rendered: &str,
    ) -> Result<(), DaemonError> {
        let Some(bridge) = native_interaction_bridge else {
            return Ok(());
        };
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(());
        };
        let Some(context_file) = provider_run.pty_env().get("ARROBA_CLAUDE_NATIVE_CONTEXT") else {
            return Ok(());
        };
        let visible = claude_rendered_permission_visible(rendered);
        if provider_run.provider() == "claude-headless" && !rendered.is_empty() {
            append_claude_headless_debug(context_file, "pty", rendered);
            self.drain_known_headless_transcripts(session_id, provider_run_id, context_file)?;
        }
        if provider_run.provider() == "claude-headless" {
            let recent = update_claude_permission_recent(context_file, rendered);
            if claude_headless_workspace_trust_visible(&recent) {
                append_claude_headless_debug(context_file, "auto_confirm", "workspace_trust");
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
                write_claude_headless_startup_wait_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
            if claude_headless_bypass_confirmation_visible(&recent) {
                append_claude_headless_debug(context_file, "auto_confirm", "bypass_permissions");
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\x1b[B\r")?;
                write_claude_headless_startup_wait_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
        }
        let recent = if visible {
            rendered.to_string()
        } else {
            update_claude_permission_recent(context_file, rendered)
        };
        if !visible && !claude_rendered_permission_visible(&recent) {
            return Ok(());
        }
        if claude_native_marker(context_file)
            .as_deref()
            .is_some_and(|value| value.starts_with("permission:"))
        {
            return Ok(());
        }
        let interaction_id = format!(
            "claude-rendered-permission-{provider_run_id}-{}",
            timestamp_millis()
        );
        write_claude_native_marker(context_file, &format!("permission:{interaction_id}"));
        clear_claude_permission_recent(context_file);
        let interaction = RuntimeInteraction::new(
            interaction_id.clone(),
            agent_id,
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some("Approve Claude Code Bash?".to_string()),
            "Claude Code is showing a native Bash permission prompt.",
            vec![
                RuntimeInteractionChoice::new(
                    "allow_once",
                    "Allow once",
                    "allow",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            Some(300),
            Some("deny".to_string()),
        );
        let session_id = session_id.to_string();
        let context_file = context_file.to_string();
        std::thread::spawn(move || {
            let input = match bridge.request_blocking(&session_id, interaction) {
                Ok(resolution)
                    if resolution.reply.as_deref() == Some("allow")
                        || resolution.choice_id.as_deref() == Some("allow_once") =>
                {
                    b"\r".to_vec()
                }
                Ok(_) => vec![0x03],
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.provider_output",
                        "Claude rendered permission bridge failed",
                        serde_json::json!({
                            "session_id": session_id,
                            "interaction_id": interaction_id,
                            "error": error.to_string(),
                        }),
                    );
                    vec![0x03]
                }
            };
            write_claude_permission_input(&context_file, &interaction_id, &input);
        });
        Ok(())
    }

    fn resolve_permission_event(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        native_interaction_bridge: Option<std::sync::Arc<dyn ProviderNativeInteractionBridge>>,
        event: &Value,
    ) -> Result<(), DaemonError> {
        let Some(bridge) = native_interaction_bridge else {
            return Ok(());
        };
        if !should_bridge_claude_permission(event) {
            return Ok(());
        }
        let Some(request_id) = event
            .get("hook_context_request_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(());
        };
        let tool_name = event
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("tool");
        let interaction = RuntimeInteraction::new(
            format!("claude-native-permission-{provider_run_id}-{request_id}"),
            agent_id.to_string(),
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            Some(format!("Approve Claude Code {tool_name}?")),
            format_claude_permission_message(event),
            vec![
                RuntimeInteractionChoice::new(
                    "allow_once",
                    "Allow once",
                    "allow",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            Some(300),
            Some("deny".to_string()),
        );
        let session_id = session_id.to_string();
        let context_file = context_file.to_string();
        let request_id = request_id.to_string();
        std::thread::spawn(
            move || match bridge.request_blocking(&session_id, interaction) {
                Ok(resolution) => {
                    let allowed = resolution.reply.as_deref() == Some("allow")
                        || resolution.choice_id.as_deref() == Some("allow_once");
                    write_claude_permission_response(
                        &context_file,
                        &request_id,
                        allowed,
                        if allowed {
                            "Approved through Arroba."
                        } else if resolution.status == "timed_out" {
                            "Timed out waiting for Arroba approval."
                        } else {
                            "Denied through Arroba."
                        },
                    );
                }
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.provider_output",
                        "Claude native permission bridge failed",
                        serde_json::json!({
                            "session_id": session_id,
                            "request_id": request_id,
                            "error": error.to_string(),
                        }),
                    );
                    write_claude_permission_response(
                        &context_file,
                        &request_id,
                        false,
                        "Arroba permission bridge failed.",
                    );
                }
            },
        );
        Ok(())
    }

    fn claude_native_prompt_context(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt: &str,
    ) -> Result<String, DaemonError> {
        let agent = self.app.agents.get_agent(agent_id)?;
        let session = self.app.sessions.get_session(session_id)?;
        let skill_grants = agent.skill_grants();
        crate::skill::format_granted_skill_prompt_context(
            agent.agent_ref(),
            &skill_grants,
            session.workspace_id(),
            prompt,
        )
    }

    fn inject_pending_prompt(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
        provider_run: &RuntimeProviderRun,
    ) -> Result<(), DaemonError> {
        let Some(prompt) = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
        else {
            return Ok(());
        };
        let mut marker = claude_native_marker(context_file);
        let force_post_stop_ready = provider_run.provider() == "claude-headless"
            && marker
                .as_deref()
                .is_some_and(|value| value == format!("post-stop-ready:{}", prompt.id()));
        if force_post_stop_ready {
            crate::logging::debug_with_fields(
                "daemon.claude_headless",
                "forcing post-stop queued prompt injection",
                serde_json::json!({
                    "session_id": session_id,
                    "provider_run_id": provider_run_id,
                    "agent_id": agent_id,
                    "prompt_id": prompt.id(),
                }),
            );
            write_claude_native_marker(context_file, "");
            marker = None;
        }
        let prompt_typed_for_headless = provider_run.provider() == "claude-headless"
            && marker.as_deref() == Some(&format!("typed:{}", prompt.id()));
        if let Some(started_at_ms) = marker
            .as_deref()
            .and_then(|value| value.strip_prefix("startup-wait:"))
            .and_then(|value| value.parse::<u64>().ok())
        {
            if unix_epoch_ms().saturating_sub(started_at_ms) < 2_500 {
                append_claude_headless_debug(context_file, "startup_wait", prompt.id());
                return Ok(());
            }
            write_claude_native_marker(context_file, "");
            marker = None;
        }
        if provider_run.provider() == "claude-headless"
            && marker.is_none()
            && unix_epoch_ms().saturating_sub(provider_run.started_at_ms()) < 4_000
        {
            append_claude_headless_debug(context_file, "inject_wait", prompt.id());
            return Ok(());
        }
        if provider_run.provider() == "claude-headless" {
            let recent = claude_permission_recent_file(context_file)
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default();
            if claude_headless_workspace_trust_visible(&recent) {
                append_claude_headless_debug(
                    context_file,
                    "inject_auto_confirm",
                    "workspace_trust",
                );
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
                write_claude_headless_startup_wait_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
            if claude_headless_bypass_confirmation_visible(&recent) {
                append_claude_headless_debug(
                    context_file,
                    "inject_auto_confirm",
                    "bypass_permissions",
                );
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\x1b[B\r")?;
                write_claude_headless_startup_wait_marker(context_file);
                clear_claude_permission_recent(context_file);
                return Ok(());
            }
            if !force_post_stop_ready
                && !prompt_typed_for_headless
                && !claude_headless_composer_visible(&recent)
            {
                append_claude_headless_debug(context_file, "inject_wait_composer", prompt.id());
                return Ok(());
            }
        }
        if marker.as_deref() == Some(&format!("typed:{}", prompt.id())) {
            append_claude_headless_debug(context_file, "inject_enter", prompt.id());
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
            write_claude_native_marker(context_file, &format!("injected:{}", prompt.id()));
            return Ok(());
        }
        if marker
            .as_deref()
            .is_some_and(|value| value.ends_with(prompt.id()))
        {
            return Ok(());
        }
        let native_attachment_suffix =
            format_claude_native_attachment_prompt_suffix(prompt.attachments(), context_file);
        let visible = redact_native_hidden_instructions(prompt.prompt())
            .trim()
            .to_string();
        let native_hidden = extract_native_hidden_instructions(prompt.prompt());
        let attachment_context =
            format_claude_attachment_context(prompt.attachments(), context_file);
        let hidden_context = if provider_run.provider() == "claude-headless" {
            let envelope = crate::prompt_assembly::PromptAssemblyService::from_env()?
                .assemble_provider_turn(
                    provider_run,
                    &visible,
                    Some(prompt.hidden_system_context()),
                    prompt.attachments().to_vec(),
                    crate::prompt_assembly::PromptAssemblyMode::NormalProviderTurn,
                )?;
            let skill_context =
                self.claude_native_prompt_context(session_id, agent_id, &visible)?;
            join_claude_context([
                envelope.hidden_system_context,
                skill_context,
                native_hidden,
                attachment_context,
            ])
        } else {
            join_claude_context([native_hidden, attachment_context])
        };
        let _ = fs::write(context_file, hidden_context);
        let visible = join_claude_context([native_attachment_suffix, visible]);
        if !visible.is_empty() {
            let input = if provider_run.provider() == "claude-headless" {
                normalize_claude_visible_prompt_for_headless(&visible)
            } else {
                visible.clone()
            };
            append_claude_headless_debug(context_file, "inject_prompt", &input);
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, input.as_bytes())?;
            if provider_run.provider() == "claude-headless" {
                write_claude_native_marker(context_file, &format!("typed:{}", prompt.id()));
            } else {
                write_claude_native_marker(context_file, &format!("injected:{}", prompt.id()));
                std::thread::sleep(std::time::Duration::from_millis(250));
                self.app
                    .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
            }
        } else {
            append_claude_headless_debug(context_file, "inject_empty", prompt.id());
            write_claude_native_marker(context_file, &format!("injected:{}", prompt.id()));
        }
        Ok(())
    }
}

fn join_claude_context(parts: impl IntoIterator<Item = String>) -> String {
    parts
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_claude_attachment_context(
    attachments: &[PromptAttachment],
    context_file: &str,
) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let blocks = attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| format_claude_attachment_block(attachment, index, context_file))
        .collect::<Vec<_>>();
    join_claude_context(
        std::iter::once(
            "The user included prompt attachments. Treat them as part of the current user request."
                .to_string(),
        )
        .chain(blocks),
    )
}

fn format_claude_attachment_block(
    attachment: &PromptAttachment,
    index: usize,
    context_file: &str,
) -> String {
    let display_name = attachment
        .filename()
        .map(str::to_string)
        .unwrap_or_else(|| format!("attachment-{}", index + 1));
    let attachment_path = materialize_claude_attachment_path(attachment, index, context_file);
    let mut pieces = vec![
        format!("Attachment {}: {display_name}", index + 1),
        format!("MIME: {}", attachment.mime()),
    ];
    if let Some(path) = attachment_path.as_ref() {
        pieces.push(format!("Path: {}", path.display()));
    }
    if let Some(text) = read_claude_text_attachment(attachment, attachment_path.as_deref()) {
        pieces.extend(["".to_string(), "Content:".to_string(), "```".to_string()]);
        pieces.push(text);
        pieces.push("```".to_string());
    } else if attachment_path.is_some() {
        pieces.extend([
            "".to_string(),
            "The attachment is available on disk at the path above.".to_string(),
        ]);
    } else {
        pieces.extend([
            "".to_string(),
            "The attachment content is not available to the Claude native bridge.".to_string(),
        ]);
    }
    pieces.join("\n")
}

fn format_claude_native_attachment_prompt_suffix(
    attachments: &[PromptAttachment],
    context_file: &str,
) -> String {
    attachments
        .iter()
        .enumerate()
        .filter(|(_, attachment)| !attachment_is_textual(attachment.mime()))
        .filter_map(|(index, attachment)| {
            materialize_claude_attachment_path(attachment, index, context_file)
        })
        .map(|path| claude_attachment_mention(&path))
        .collect::<Vec<_>>()
        .join(" ")
}

fn materialize_claude_attachment_path(
    attachment: &PromptAttachment,
    index: usize,
    context_file: &str,
) -> Option<PathBuf> {
    if let Some(path) = local_attachment_path(attachment.url()) {
        return Some(path);
    }
    let contents_base64 = attachment.contents_base64()?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(contents_base64)
        .ok()?;
    let root = Path::new(context_file).with_file_name("attachments");
    fs::create_dir_all(&root).ok()?;
    let filename = safe_attachment_filename(attachment, index);
    let path = root.join(filename);
    fs::write(&path, bytes).ok()?;
    Some(path)
}

fn local_attachment_path(url: &str) -> Option<PathBuf> {
    let path = url
        .strip_prefix("file://localhost")
        .or_else(|| url.strip_prefix("file://"))?;
    if path.starts_with('/') {
        Some(PathBuf::from(path))
    } else {
        None
    }
}

fn read_claude_text_attachment(
    attachment: &PromptAttachment,
    attachment_path: Option<&Path>,
) -> Option<String> {
    if !attachment_is_textual(attachment.mime()) {
        return None;
    }
    let bytes = if let Some(contents_base64) = attachment.contents_base64() {
        base64::engine::general_purpose::STANDARD
            .decode(contents_base64)
            .ok()?
    } else {
        fs::read(attachment_path?).ok()?
    };
    if bytes.len() > CLAUDE_ATTACHMENT_CONTEXT_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn attachment_is_textual(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/javascript"
                | "application/typescript"
                | "application/xml"
                | "application/yaml"
                | "application/x-yaml"
        )
        || mime.ends_with("+json")
        || mime.ends_with("+xml")
}

fn claude_attachment_mention(path: &Path) -> String {
    let value = path.display().to_string();
    if value
        .chars()
        .all(|ch| !ch.is_whitespace() && !matches!(ch, '"' | '\'' | '\\'))
    {
        format!("@{value}")
    } else {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("@\"{escaped}\"")
    }
}

fn safe_attachment_filename(attachment: &PromptAttachment, index: usize) -> String {
    let fallback = format!(
        "attachment-{}{}",
        index + 1,
        extension_for_mime(attachment.mime())
    );
    let raw = attachment.filename().unwrap_or(&fallback);
    let sanitized = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim_matches('_');
    format!(
        "{}-{}",
        index + 1,
        if sanitized.is_empty() {
            fallback.as_str()
        } else {
            sanitized
        }
    )
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "application/pdf" => ".pdf",
        "application/json" => ".json",
        _ if mime.starts_with("text/") => ".txt",
        _ => ".bin",
    }
}

fn extract_claude_native_prompt_attachments(
    prompt: &str,
    working_directory: Option<&Path>,
) -> Vec<PromptAttachment> {
    let mut attachments = Vec::new();
    for token in prompt
        .split_whitespace()
        .filter_map(|part| part.strip_prefix('@'))
    {
        let token = token
            .trim_matches('"')
            .trim_matches('\'')
            .trim_end_matches([',', '.', ';', ':', '!', '?', ')']);
        if token.is_empty() {
            continue;
        }
        let path = resolve_claude_attachment_path(token, working_directory);
        if !path.is_file() {
            continue;
        }
        let mime = mime_for_path(&path);
        attachments.push(PromptAttachment::new(
            format!("file://{}", path.display()),
            mime,
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned()),
        ));
    }
    attachments
}

fn resolve_claude_attachment_path(value: &str, working_directory: Option<&Path>) -> PathBuf {
    if let Some(path) = local_attachment_path(value) {
        return path;
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        working_directory
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    }
}

fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "json" => "application/json",
        "md" => "text/markdown",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "ts" | "tsx" => "application/typescript",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_transcript_drain_maps_assistant_text_reasoning_and_tools() {
        let mut cursor = ClaudeTranscriptCursor::default();
        let dir = std::env::temp_dir().join(format!(
            "arroba-claude-transcript-test-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let transcript = dir.join("session.jsonl");
        fs::write(
            &transcript,
            [
                serde_json::json!({
                    "type": "assistant",
                    "uuid": "assistant-1",
                    "sessionId": "claude-session-1",
                    "message": {
                        "id": "msg_1",
                        "model": "claude-sonnet-4-6",
                        "role": "assistant",
                        "content": [
                            { "type": "thinking", "thinking": "considering" },
                            { "type": "text", "text": "hello" },
                            { "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "pwd" } }
                        ]
                    }
                })
                .to_string(),
                serde_json::json!({
                    "type": "user",
                    "uuid": "user-1",
                    "message": {
                        "role": "user",
                        "content": [
                            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "ok" }
                        ]
                    }
                })
                .to_string(),
            ]
            .join("\n"),
        )
        .expect("fixture should write");

        let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

        assert_eq!(drain.session_id.as_deref(), Some("claude-session-1"));
        assert_eq!(drain.model.as_deref(), Some("claude/claude-sonnet-4-6"));
        assert_eq!(drain.assistant_message_ids, vec!["msg_1"]);
        assert_eq!(drain.chunks.len(), 4);
        assert_eq!(drain.chunks[0].kind, TerminalOutputKind::ProviderReasoning);
        assert_eq!(drain.chunks[0].text, "considering");
        assert_eq!(drain.chunks[1].kind, TerminalOutputKind::ProviderOutput);
        assert_eq!(drain.chunks[1].text, "hello");
        assert_eq!(drain.chunks[2].kind, TerminalOutputKind::ProviderTool);
        assert!(drain.chunks[2].text.contains("\"tool\":\"Bash\""));
        assert!(drain.chunks[3].text.contains("\"status\":\"completed\""));

        let second = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);
        assert!(second.chunks.is_empty());
        assert!(second.assistant_message_ids.is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn claude_transcript_drain_skips_internal_and_duplicate_entries() {
        let mut cursor = ClaudeTranscriptCursor::default();
        let dir = std::env::temp_dir().join(format!(
            "arroba-claude-transcript-dedupe-test-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let transcript = dir.join("session.jsonl");
        let assistant = serde_json::json!({
            "type": "assistant",
            "uuid": "assistant-1",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "once" }]
            }
        })
        .to_string();
        fs::write(
            &transcript,
            [
                serde_json::json!({ "type": "queue-operation", "operation": "enqueue" })
                    .to_string(),
                assistant.clone(),
                assistant,
            ]
            .join("\n"),
        )
        .expect("fixture should write");

        let drain = drain_claude_transcript_file(&transcript.display().to_string(), &mut cursor);

        assert_eq!(drain.chunks.len(), 1);
        assert_eq!(drain.chunks[0].text, "once");
        assert_eq!(drain.assistant_message_ids, vec!["assistant-1"]);

        let _ = fs::remove_dir_all(dir);
    }
}
