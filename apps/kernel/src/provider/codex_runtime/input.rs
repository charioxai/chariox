//! Codex provider input assembly and local attachment URL handling.

use serde_json::{json, Value};

use crate::session::PromptAttachment;

pub(super) fn codex_input(prompt: &str, attachments: &[PromptAttachment]) -> Vec<Value> {
    let mut input = Vec::new();
    if !prompt.trim().is_empty() {
        input.push(json!({
            "type": "text",
            "text": prompt,
        }));
    }

    let mut attachment_notes = Vec::new();
    for attachment in attachments {
        if attachment.mime().starts_with("image/") {
            if let Some(local_path) = resolve_local_attachment_path(attachment.url()) {
                input.push(json!({
                    "type": "localImage",
                    "path": local_path,
                }));
            } else {
                input.push(json!({
                    "type": "image",
                    "url": attachment.url(),
                }));
            }
            continue;
        }
        let label = attachment
            .filename()
            .map(str::to_string)
            .unwrap_or_else(|| attachment.url().to_string());
        attachment_notes.push(format!(
            "Attachment: {label} ({}) at {}",
            attachment.mime(),
            attachment.url()
        ));
    }

    if !attachment_notes.is_empty() {
        input.push(json!({
            "type": "text",
            "text": attachment_notes.join("\n"),
        }));
    }

    input
}

fn resolve_local_attachment_path(url: &str) -> Option<String> {
    if url.starts_with('/') {
        return Some(url.to_string());
    }

    let stripped = url
        .strip_prefix("file://localhost")
        .or_else(|| url.strip_prefix("file://"))?;
    if !stripped.starts_with('/') {
        return None;
    }

    Some(percent_decode_path(stripped))
}

fn percent_decode_path(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hi = decode_hex_nibble(bytes[index + 1]);
            let lo = decode_hex_nibble(bytes[index + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                decoded.push((hi << 4) | lo);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::session::PromptAttachment;

    use super::{codex_input, resolve_local_attachment_path};

    #[test]
    fn codex_input_treats_file_url_images_as_local_images() {
        let input = codex_input(
            "describe this image",
            &[PromptAttachment::new(
                "file:///tmp/capture%20one.png",
                "image/png",
                Some("capture one.png".to_string()),
            )],
        );

        assert_eq!(
            input,
            vec![
                json!({
                    "type": "text",
                    "text": "describe this image",
                }),
                json!({
                    "type": "localImage",
                    "path": "/tmp/capture one.png",
                }),
            ]
        );
    }

    #[test]
    fn resolve_local_attachment_path_accepts_file_urls_and_decodes_percent_escapes() {
        assert_eq!(
            resolve_local_attachment_path("file:///tmp/a%20b.png"),
            Some("/tmp/a b.png".to_string())
        );
        assert_eq!(
            resolve_local_attachment_path("file://localhost/tmp/a%20b.png"),
            Some("/tmp/a b.png".to_string())
        );
        assert_eq!(
            resolve_local_attachment_path("/tmp/a b.png"),
            Some("/tmp/a b.png".to_string())
        );
        assert_eq!(
            resolve_local_attachment_path("https://example.com/a.png"),
            None
        );
    }
}
