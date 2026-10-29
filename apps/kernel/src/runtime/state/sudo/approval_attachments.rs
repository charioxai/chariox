//! MP-08 / MP-10 / MP-11: the attachments of a sudo prompt reach the elevated
//! turn, so the approval popup lists them: what the owner approves is what runs.
use base64::Engine;

const MAX_LISTED: usize = 8;
const MAX_NOTE_CHARS: usize = 600;

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

/// One line per attachment; a note's own text is shown because it becomes
/// hidden instructions for the agent. Empty when there are none.
pub(super) fn approval_text(attachments: &[crate::session::PromptAttachment]) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let mut text = format!("\n\nAttachments ({}):", attachments.len());
    for attachment in attachments.iter().take(MAX_LISTED) {
        let name = printable(attachment.filename().unwrap_or("unnamed"));
        let note = crate::prompt_notes::is_note(attachment)
            .then(|| attachment.contents_base64())
            .flatten()
            .and_then(|encoded| {
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .ok()
            })
            .map(|bytes| printable(&String::from_utf8_lossy(&bytes)));
        match note {
            Some(note) => {
                let shown: String = note.chars().take(MAX_NOTE_CHARS).collect();
                let cut = if note.chars().count() > MAX_NOTE_CHARS {
                    "…"
                } else {
                    ""
                };
                text.push_str(&format!("\n- note {name}: {shown}{cut}"));
            }
            None => text.push_str(&format!("\n- {name} ({})", printable(attachment.mime()))),
        }
    }
    if attachments.len() > MAX_LISTED {
        text.push_str(&format!("\n- and {} more", attachments.len() - MAX_LISTED));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fixture_with_options, popup};
    use super::*;

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
}
