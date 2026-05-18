use base64::Engine as _;
use serde_json::{json, Value};

use crate::session::PromptAttachment;

pub(super) fn claude_user_content(prompt: &str, attachments: &[PromptAttachment]) -> Vec<Value> {
    let mut content = Vec::new();
    if !prompt.trim().is_empty() {
        content.push(json!({ "type": "text", "text": prompt }));
    }
    for attachment in attachments {
        content.extend(claude_attachment_content(attachment));
    }
    if content.is_empty() {
        content.push(json!({ "type": "text", "text": "" }));
    }
    content
}

fn claude_attachment_content(attachment: &PromptAttachment) -> Vec<Value> {
    let label = attachment
        .filename()
        .map(str::to_string)
        .unwrap_or_else(|| attachment.url().to_string());
    if let Some(data) = attachment.contents_base64() {
        if attachment.mime().starts_with("image/") {
            return vec![json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": attachment.mime(),
                    "data": data,
                }
            })];
        }
        if attachment_is_textual(attachment.mime()) {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
                if let Ok(text) = String::from_utf8(bytes) {
                    return vec![json!({
                        "type": "text",
                        "text": format!(
                            "Attachment: {label} ({}) at {}\n\n{}",
                            attachment.mime(),
                            attachment.url(),
                            text,
                        ),
                    })];
                }
            }
        }
    }
    vec![json!({
        "type": "text",
        "text": format!(
            "Attachment: {label} ({}) at {}",
            attachment.mime(),
            attachment.url(),
        ),
    })]
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
