//! MP-08 / MP-10 / MP-11: the attachments of a sudo prompt reach the elevated
//! turn, so the approval popup lists them: what the owner approves is what runs.
use crate::session::PromptAttachment;

const MAX_LISTED: usize = 8;
const QUOTE_PREVIEW_CHARS: usize = 160;
const COMMENT_PREVIEW_CHARS: usize = 600;

fn printable(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect()
}

fn preview(text: &str, limit: usize) -> String {
    let text = printable(text);
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text,
    }
}

fn name(attachment: &PromptAttachment) -> String {
    printable(attachment.filename().unwrap_or("unnamed"))
}

/// What the popup message adds for the prompt's attachments: one entry per
/// attachment, a note as a short preview of the selected text and its comment.
/// Empty when there are none.
pub(super) fn approval_text(attachments: &[PromptAttachment]) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let mut text = format!("\n\nAttachments ({}):", attachments.len());
    for attachment in attachments.iter().take(MAX_LISTED) {
        let name = name(attachment);
        match crate::prompt_notes::quote_and_comment(attachment) {
            Some((quote, comment)) => {
                text.push_str(&format!(
                    "\n- note {name}\n  Selected text: {}",
                    preview(&quote, QUOTE_PREVIEW_CHARS)
                ));
                if !comment.trim().is_empty() {
                    text.push_str(&format!(
                        "\n  Comment: {}",
                        preview(&comment, COMMENT_PREVIEW_CHARS)
                    ));
                }
            }
            None => text.push_str(&format!("\n- {name} ({})", printable(attachment.mime()))),
        }
    }
    if attachments.len() > MAX_LISTED {
        text.push_str(&format!("\n- and {} more", attachments.len() - MAX_LISTED));
    }
    text
}

/// The complete notes for the scrollable details, which clients show under
/// the requested task: the owner can read everything the agent is told.
pub(super) fn details_text(attachments: &[PromptAttachment]) -> String {
    let multiline = |text: &str| -> String {
        text.chars()
            .filter(|c| {
                (!c.is_control() || *c == '\n')
                    && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
            .collect()
    };
    attachments
        .iter()
        .filter_map(|attachment| {
            let (quote, comment) = crate::prompt_notes::quote_and_comment(attachment)?;
            let mut note = format!(
                "\n\nNote {}\nSelected text: {}",
                name(attachment),
                multiline(&quote)
            );
            if !comment.trim().is_empty() {
                note.push_str(&format!("\nComment: {}", multiline(&comment)));
            }
            Some(note)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture_with_options, popup};
    use super::*;
    use base64::Engine;

    #[tokio::test]
    async fn the_approval_popup_lists_the_attachments_the_elevated_turn_receives() {
        let mut f = fixture_with_options(None, true);
        let note = base64::engine::general_purpose::STANDARD
            .encode(r#"{"quote":"alias every session","comment":"also pair a new phone"}"#);
        f.request.attachments = vec![crate::session::PromptAttachment::new(
            "inline:note",
            crate::prompt_notes::NOTE_ATTACHMENT_MIME,
            Some("note.json".into()),
        )
        .with_contents_base64(note)];
        let state = f.state.clone();
        let request = f.request.clone();
        let starting = tokio::spawn(async move {
            state
                .submit_sudo_prompt(request, "local", "sudo-terminal")
                .await
        });
        let consent = popup(&f.state).await;
        starting.abort();
        assert!(
            consent.message.contains("also pair a new phone"),
            "{}",
            consent.message
        );
        assert!(consent.message.contains("note.json"), "{}", consent.message);
    }

    #[tokio::test]
    async fn a_long_quotation_does_not_hide_the_comment_or_the_complete_note() {
        let mut f = fixture_with_options(None, true);
        let quote = format!("{} END-OF-QUOTE", "selected words ".repeat(80));
        let note = base64::engine::general_purpose::STANDARD
            .encode(serde_json::json!({"quote": quote, "comment": "make this change"}).to_string());
        f.request.attachments = vec![crate::session::PromptAttachment::new(
            "inline:note",
            crate::prompt_notes::NOTE_ATTACHMENT_MIME,
            Some("note.json".into()),
        )
        .with_contents_base64(note)];
        let state = f.state.clone();
        let request = f.request.clone();
        let starting = tokio::spawn(async move {
            state
                .submit_sudo_prompt(request, "local", "sudo-terminal")
                .await
        });
        let consent = popup(&f.state).await;
        starting.abort();
        assert!(
            consent.message.contains("Comment: make this change"),
            "{}",
            consent.message
        );
        let task = consent.sudo_request.expect("sudo presentation").task;
        assert!(task.contains("END-OF-QUOTE"), "{task}");
        assert!(task.contains("make this change"), "{task}");
    }
}
