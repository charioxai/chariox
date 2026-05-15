use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::Value;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::RuntimeProviderRun;
use crate::session::PromptAttachment;

const CLAUDE_ATTACHMENT_CONTEXT_BYTES: usize = 64 * 1024;

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

        self.inject_pending_prompt(session_id, provider_run_id, &agent_id, context_file)?;

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
                let _ = fs::write(context_file, "");
                write_claude_native_marker(context_file, "");
                let _ =
                    self.app
                        .complete_active_prompt(session_id, &agent_id, Some(provider_run_id));
            }
        }
        Ok(())
    }

    fn inject_pending_prompt(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        context_file: &str,
    ) -> Result<(), DaemonError> {
        let Some(prompt) = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
        else {
            return Ok(());
        };
        let marker = claude_native_marker(context_file);
        if marker.as_deref() == Some(&format!("typed:{}", prompt.id())) {
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
        let hidden = extract_native_hidden_instructions(prompt.prompt());
        let attachment_context =
            format_claude_attachment_context(prompt.attachments(), context_file);
        let _ = fs::write(
            context_file,
            join_claude_context([hidden, attachment_context]),
        );
        let native_attachment_suffix =
            format_claude_native_attachment_prompt_suffix(prompt.attachments(), context_file);
        let visible = redact_native_hidden_instructions(prompt.prompt())
            .trim()
            .to_string();
        let visible = join_claude_context([native_attachment_suffix, visible]);
        if !visible.is_empty() {
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, visible.as_bytes())?;
            write_claude_native_marker(context_file, &format!("injected:{}", prompt.id()));
            std::thread::sleep(std::time::Duration::from_millis(250));
            self.app
                .write_provider_pty_input_for_runtime(provider_run_id, b"\r")?;
        } else {
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
