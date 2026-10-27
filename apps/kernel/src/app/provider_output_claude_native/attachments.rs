use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;

use crate::session::PromptAttachment;

use super::CLAUDE_ATTACHMENT_CONTEXT_BYTES;

pub(super) fn join_claude_context(parts: impl IntoIterator<Item = String>) -> String {
    parts
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn format_claude_attachment_context(
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

pub(super) fn format_claude_native_attachment_prompt_suffix(
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

pub(super) fn extract_claude_native_prompt_attachments(
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
