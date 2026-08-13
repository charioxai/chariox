use std::fs;
use std::path::Path;

use base64::Engine;

use crate::error::DaemonError;
use crate::session::PromptAttachment;

pub(super) const INLINE_PROMPT_ATTACHMENT_DIR: &str = "chariox-terminal-prompt-attachments";

pub(super) fn materialize_inline_prompt_attachments(
    session_id: &str,
    agent_id: &str,
    attachments: Vec<PromptAttachment>,
) -> Result<Vec<PromptAttachment>, DaemonError> {
    attachments
        .into_iter()
        .enumerate()
        .map(|(index, attachment)| {
            let Some(contents_base64) = attachment.contents_base64() else {
                return Ok(attachment);
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(contents_base64)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "decode inline prompt attachment",
                    message: error.to_string(),
                })?;
            let filename = attachment
                .filename()
                .map(sanitize_attachment_filename)
                .unwrap_or_else(|| format!("attachment-{index}"));
            let root = std::env::temp_dir()
                .join(INLINE_PROMPT_ATTACHMENT_DIR)
                .join(sanitize_path_component(session_id))
                .join(sanitize_path_component(agent_id));
            fs::create_dir_all(&root).map_err(|error| DaemonError::LocalTransport {
                operation: "create inline prompt attachment directory",
                message: error.to_string(),
            })?;
            let path = root.join(format!(
                "{}-{}-{}",
                crate::session::unix_epoch_ms(),
                index,
                filename
            ));
            fs::write(&path, bytes).map_err(|error| DaemonError::LocalTransport {
                operation: "write inline prompt attachment",
                message: error.to_string(),
            })?;
            Ok(PromptAttachment::new(
                format!("file://{}", path.display()),
                attachment.mime().to_string(),
                Some(filename),
            ))
        })
        .collect()
}

fn sanitize_attachment_filename(value: &str) -> String {
    let file_name = Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment");
    sanitize_path_component(file_name)
}

fn sanitize_path_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches(['.', '-']);
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}
