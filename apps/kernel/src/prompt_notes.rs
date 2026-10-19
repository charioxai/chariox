//! Notes the user attached to a prompt (selected text + comment + where it came from).
//! They travel as inline prompt attachments so the visible transcript and history keep
//! showing chips; providers never see them as files, they get hidden prompt context.

use base64::Engine;
use serde::Deserialize;

use crate::error::DaemonError;
use crate::prompt_assembly::{assembled_prompt_component, prompt_component};
use crate::session::PromptAttachment;

pub const NOTE_ATTACHMENT_MIME: &str = "application/vnd.chariox.note+json";
const MAX_NOTES: usize = 16;
const MAX_NOTE_BYTES: usize = 48 * 1024;

#[derive(Debug, Deserialize)]
struct NoteSource {
    kind: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    room: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Note {
    quote: String,
    #[serde(default)]
    comment: String,
    source: NoteSource,
}

pub fn is_note(attachment: &PromptAttachment) -> bool {
    attachment.mime() == NOTE_ATTACHMENT_MIME
}

fn parse(attachment: &PromptAttachment) -> Result<Note, String> {
    let encoded = attachment.contents_base64().ok_or("note has no contents")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_NOTE_BYTES {
        return Err("note is too large".to_string());
    }
    let note: Note = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if note.quote.trim().is_empty() {
        return Err("note has no selected text".to_string());
    }
    Ok(note)
}

/// Rejects malformed notes at admission, so the user sees the error instead of losing context.
pub fn validate(attachments: &[PromptAttachment]) -> Result<(), DaemonError> {
    let notes = attachments.iter().filter(|attachment| is_note(attachment));
    let mut count = 0;
    for attachment in notes {
        count += 1;
        parse(attachment).map_err(|message| DaemonError::LocalTransport {
            operation: "validate prompt note",
            message,
        })?;
    }
    if count > MAX_NOTES {
        return Err(DaemonError::LocalTransport {
            operation: "validate prompt note",
            message: format!("at most {MAX_NOTES} notes can be attached to one prompt"),
        });
    }
    Ok(())
}

fn describe(source: &NoteSource) -> String {
    let label = match source.kind.as_str() {
        "browser" => "Browser page",
        "room" => "Room",
        _ => "Panel",
    };
    let mut parts = Vec::new();
    for value in [&source.title, &source.room, &source.url].into_iter().flatten() {
        if !value.trim().is_empty() {
            parts.push(value.trim().to_string());
        }
    }
    if parts.is_empty() {
        label.to_string()
    } else {
        format!("{label}: {}", parts.join(" | "))
    }
}

/// Removes note attachments from the provider-bound list and renders them as hidden context.
pub(crate) fn split_for_provider(
    attachments: Vec<PromptAttachment>,
) -> (Vec<PromptAttachment>, Option<String>) {
    let (notes, rest): (Vec<_>, Vec<_>) = attachments.into_iter().partition(is_note);
    let blocks: Vec<String> = notes
        .iter()
        .filter_map(|attachment| parse(attachment).ok())
        .take(MAX_NOTES)
        .map(|note| {
            let mut body = format!("Source: {}\n", describe(&note.source));
            body.push_str(&prompt_component("selected-text", &note.quote));
            if !note.comment.trim().is_empty() {
                body.push('\n');
                body.push_str(&prompt_component("comment", &note.comment));
            }
            assembled_prompt_component("note", &body)
        })
        .collect();
    if blocks.is_empty() {
        return (rest, None);
    }
    let context = format!(
        "The user attached the notes below to this message: each quotes text they selected and adds their comment. Treat them as part of the request.\n{}",
        blocks.join("\n")
    );
    (rest, Some(assembled_prompt_component("user-notes", &context)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(json: &str) -> PromptAttachment {
        PromptAttachment::new("chariox-terminal://note/1", NOTE_ATTACHMENT_MIME, Some("Note".into()))
            .with_contents_base64(base64::engine::general_purpose::STANDARD.encode(json))
    }

    #[test]
    fn notes_become_hidden_context_with_their_source_and_leave_the_attachments() {
        let image = PromptAttachment::new("file:///a.png", "image/png", None);
        let (rest, context) = split_for_provider(vec![
            image.clone(),
            note(r#"{"quote":"hello","comment":"fix </selected-text> now","source":{"kind":"browser","title":"Docs","url":"https://example.test/a"}}"#),
            note(r#"{"quote":"room text","source":{"kind":"room","room":"Design"}}"#),
        ]);
        assert_eq!(rest, vec![image]);
        let context = context.unwrap();
        assert!(context.starts_with("<user-notes>"));
        assert!(context.contains("Browser page: Docs | https://example.test/a"));
        assert!(context.contains("Room: Design"));
        assert!(context.contains("hello") && context.contains("room text"));
        assert!(context.contains("fix &lt;/selected-text&gt; now"), "delimiters in user text are escaped");
        assert_eq!(context.matches("<note>").count(), 2);
    }

    #[test]
    fn no_notes_leaves_everything_untouched() {
        let image = PromptAttachment::new("file:///a.png", "image/png", None);
        assert_eq!(split_for_provider(vec![image.clone()]), (vec![image], None));
    }

    #[test]
    fn admission_rejects_malformed_or_empty_notes() {
        assert!(validate(&[note(r#"{"quote":"x","source":{"kind":"panel"}}"#)]).is_ok());
        assert!(validate(&[note("not json")]).is_err());
        assert!(validate(&[note(r#"{"quote":"  ","source":{"kind":"panel"}}"#)]).is_err());
        let many = vec![note(r#"{"quote":"x","source":{"kind":"panel"}}"#); MAX_NOTES + 1];
        assert!(validate(&many).is_err());
    }
}
