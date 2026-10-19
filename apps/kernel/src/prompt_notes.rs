//! Notes the user attached to a prompt (selected text + comment + where it came from).
//! They travel as inline prompt attachments so the visible transcript and history keep
//! showing chips; providers never see them as files, they get hidden prompt context.

use base64::Engine;
use serde::Deserialize;

use crate::error::DaemonError;
use crate::prompt_assembly::{assembled_prompt_component, prompt_component};
use crate::session::PromptAttachment;

pub const NOTE_ATTACHMENT_MIME: &str = "application/vnd.chariox.note+json";
/// Typed into providers that need visible input to start a turn when a message is notes only.
pub(crate) const NOTE_ONLY_PROMPT: &str = "Please read the attached notes.";
const MAX_NOTES: usize = 16;
const MAX_NOTE_BYTES: usize = 48 * 1024;

/// Where the selected text lives. Everything but `kind` is optional; the kernel only forwards
/// a reference, never the surrounding content (the agent fetches that with an existing tool).
#[derive(Debug, Deserialize)]
struct NoteSource {
    /// `kernel_browser`, `room_browser`, `room`, `agent` or `panel`; absent for a note on the
    /// focused agent's own trace.
    #[serde(default)]
    kind: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    document_id: Option<String>,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    suffix: Option<String>,
    #[serde(default)]
    room: Option<String>,
    #[serde(default)]
    item: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    /// Instance id of the agent whose pane it was; the same agent receiving the note needs no source.
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    turn_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Note {
    quote: String,
    #[serde(default)]
    comment: String,
    #[serde(default)]
    source: Option<NoteSource>,
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

fn field(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|value| !value.is_empty())
}

fn quoted(value: &Option<String>) -> Option<String> {
    value.as_deref().filter(|value| !value.is_empty()).map(|value| format!("{value:?}"))
}

/// Source, anchor and the one tool that reads more of it; empty for a note on the agent's own trace.
fn describe(source: &NoteSource) -> Option<String> {
    let join = |parts: &[(&str, Option<&str>)]| {
        parts
            .iter()
            .filter_map(|(label, value)| value.map(|value| format!("{label} {value}")))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (label, anchor, more) = match source.kind.as_str() {
        "kernel_browser" | "room_browser" => (
            "Browser page",
            join(&[
                ("document", field(&source.document_id)),
                ("text before", quoted(&source.prefix).as_deref()),
                ("text after", quoted(&source.suffix).as_deref()),
            ]),
            if source.kind == "kernel_browser" {
                "read the tab at this URL with chariox.kernel_browser"
            } else {
                "read the page with chariox.slice_browser_text or chariox.slice_browser_find"
            },
        ),
        "room" => (
            "Room",
            join(&[
                ("room", field(&source.room)),
                ("item", field(&source.item)),
                ("document", field(&source.document_id)),
                ("text before", quoted(&source.prefix).as_deref()),
                ("text after", quoted(&source.suffix).as_deref()),
            ]),
            "read it with chariox.room.overview, or the page with chariox.slice_browser_text / chariox.slice_browser_find",
        ),
        "agent" => (
            "Agent message",
            join(&[
                ("session", field(&source.session_id)),
                ("turn", field(&source.turn_id)),
            ]),
            "read it with chariox.history.turn or chariox.history.read",
        ),
        "panel" => ("Panel", String::new(), ""),
        _ => return None,
    };
    let names: Vec<&str> = [
        field(&source.agent),
        field(&source.title),
        field(&source.room),
        field(&source.item),
        field(&source.url),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut text = format!("Source: {label}{}{}", if names.is_empty() { "" } else { ": " }, names.join(" | "));
    if !anchor.is_empty() {
        text.push_str(&format!("\nAnchor: {anchor}"));
    }
    if !more.is_empty() {
        text.push_str(&format!("\nMore: {more}"));
    }
    Some(text)
}

/// Removes note attachments from the provider-bound list and renders them as hidden context.
pub(crate) fn split_for_provider(
    attachments: Vec<PromptAttachment>,
    target_agent: Option<&str>,
) -> (Vec<PromptAttachment>, Option<String>) {
    let (notes, rest): (Vec<_>, Vec<_>) = attachments.into_iter().partition(is_note);
    let blocks: Vec<String> = notes
        .iter()
        .take(MAX_NOTES)
        .map(|attachment| {
            // An unreadable note is still told to the agent instead of vanishing.
            let Ok(note) = parse(attachment) else {
                return assembled_prompt_component("note", "The user attached a note that could not be read.");
            };
            let mut body = note
                .source
                .as_ref()
                .filter(|source| target_agent.is_none() || field(&source.agent_id) != target_agent)
                .and_then(describe)
                .map(|text| format!("{text}\n"))
                .unwrap_or_default();
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
        "The user attached the notes below to this message: each quotes text they selected and adds their comment. Treat them as part of the request; a \"More\" line names the tool that reads the surrounding content if you need it.\n{}",
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
    fn notes_become_hidden_context_with_source_anchor_and_tool_and_leave_the_attachments() {
        let image = PromptAttachment::new("file:///a.png", "image/png", None);
        let (rest, context) = split_for_provider(vec![
            image.clone(),
            note(r#"{"quote":"hello","comment":"fix </selected-text> now","source":{"kind":"kernel_browser","title":"Docs","url":"https://example.test/a","document_id":"doc-7","prefix":"say ","suffix":" world"}}"#),
            note(r#"{"quote":"room text","source":{"kind":"room","room":"Design","item":"tab-2","document_id":"doc-9","prefix":"a "}}"#),
            note(r#"{"quote":"earlier answer","source":{"kind":"agent","agent":"alice","agent_id":"agent-a","session_id":"s1","turn_id":"turn-9"}}"#),
        ], Some("agent-b"));
        assert_eq!(rest, vec![image]);
        let context = context.unwrap();
        assert!(context.starts_with("<user-notes>"));
        assert!(context.contains("Source: Browser page: Docs | https://example.test/a"));
        assert!(context.contains("Anchor: document doc-7, text before \"say \", text after \" world\""));
        assert!(context.contains("chariox.kernel_browser"));
        assert!(context.contains("Source: Room: Design | tab-2") && context.contains("chariox.room.overview"));
        assert!(context.contains("Anchor: room Design, item tab-2, document doc-9, text before \"a \""));
        assert!(context.contains("Source: Agent message: alice") && context.contains("Anchor: session s1, turn turn-9"));
        assert!(context.contains("chariox.history.turn"));
        assert!(context.contains("hello") && context.contains("room text"));
        assert!(context.contains("fix &lt;/selected-text&gt; now"), "delimiters in user text are escaped");
        assert_eq!(context.matches("<note>").count(), 3);
    }

    #[test]
    fn a_note_on_the_receiving_agents_own_trace_carries_only_quote_and_comment() {
        let (_, context) = split_for_provider(vec![
            note(r#"{"quote":"mine","comment":"c","source":{"kind":"agent","agent":"alice","agent_id":"agent-a","turn_id":"turn-9"}}"#),
            note(r#"{"quote":"bare"}"#),
        ], Some("agent-a"));
        let context = context.unwrap();
        assert!(!context.contains("Source:") && !context.contains("turn-9"));
        assert!(context.contains("mine") && context.contains("bare"));
    }

    #[test]
    fn an_unreadable_note_is_reported_not_dropped() {
        let (rest, context) = split_for_provider(vec![note("not json")], None);
        assert!(rest.is_empty());
        assert!(context.unwrap().contains("could not be read"));
    }

    #[test]
    fn no_notes_leaves_everything_untouched() {
        let image = PromptAttachment::new("file:///a.png", "image/png", None);
        assert_eq!(split_for_provider(vec![image.clone()], None), (vec![image], None));
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
