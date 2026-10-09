//! MD-N3 / MP-08 / MP-11: protocol 447 freezes every note request/event seam.
use super::*;
use crate::local::{NoteAnchor, NoteCommand, NoteResult, NoteTextQuote, NoteWindow, NotesRequest};
#[test]
fn notes_protocol_443_shapes_and_rejected_authority_claims() {
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 475);
    let window = NoteWindow::Panel {
        window_id: "message-42".into(),
    };
    let anchor = NoteAnchor {
        window: window.clone(),
        url: None,
        document_id: None,
        hint: None,
        quote: NoteTextQuote {
            exact: "quote".into(),
            prefix: "before".into(),
            suffix: "after".into(),
        },
    };
    let commands = vec![
        NoteCommand::CaptureSelection {
            window: window.clone(),
        },
        NoteCommand::ReportSelection {
            anchor,
            box_css: None,
        },
        NoteCommand::Create {
            selection_id: "s".into(),
            comment: "explain".into(),
        },
        NoteCommand::List { window },
        NoteCommand::Read {
            note_id: "n".into(),
        },
        NoteCommand::Reply {
            note_id: "n".into(),
            comment: "answer".into(),
        },
        NoteCommand::Resolve {
            note_id: "n".into(),
        },
        NoteCommand::Reanchor {
            note_id: "n".into(),
        },
        NoteCommand::Ask {
            note_id: "n".into(),
        },
    ];
    let values = commands
        .into_iter()
        .map(|command| {
            serde_json::to_value(LocalDaemonRequest::Notes(NotesRequest { command })).unwrap()
        })
        .collect::<Vec<_>>();
    let expected: serde_json::Value = serde_json::from_str(include_str!("notes-443.json")).unwrap();
    assert_eq!(serde_json::json!(values), expected);
    for value in values {
        let request: LocalDaemonRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), value);
    }
    for invalid in [
        serde_json::json!({"Notes":{"user_id":"forged","command":{"op":"list","window":{"kind":"panel","window_id":"x"}}}}),
        serde_json::json!({"Notes":{"command":{"op":"resolve","note_id":"n","agent_id":"forged"}}}),
        serde_json::json!({"Notes":{"command":{"op":"capture_selection","window":{"kind":"kernel_browser","tab_id":"x"}}}}),
    ] {
        assert!(serde_json::from_value::<LocalDaemonRequest>(invalid).is_err());
    }
    let response = LocalDaemonResponse::Notes {
        result: NoteResult::SelectionChanged { selection: None },
    };
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        serde_json::json!({"Notes":{"result":{"event":"selection_changed","selection":null}}})
    );
}

#[test]
fn notes_protocol_443_response_snapshots() {
    use crate::local::{NoteRecord, NoteReply, NoteSelection, NoteSummary};
    assert_eq!(LOCAL_DAEMON_PROTOCOL_VERSION, 475);
    let window = NoteWindow::Panel {
        window_id: "message-42".into(),
    };
    let anchor = NoteAnchor {
        window: window.clone(),
        url: None,
        document_id: None,
        hint: Some("node-42".into()),
        quote: NoteTextQuote {
            exact: "quote".into(),
            prefix: "before".into(),
            suffix: "after".into(),
        },
    };
    let note = NoteRecord {
        note_id: "n".into(),
        author: "alice".into(),
        domain: "user".into(),
        anchor: anchor.clone(),
        comment: "explain".into(),
        replies: vec![NoteReply {
            author: "agent:focused".into(),
            comment: "answer".into(),
            created_at_ms: 101,
        }],
        resolved: false,
        anchor_state: "attached".into(),
        box_css: None,
        created_at_ms: 100,
        updated_at_ms: 101,
    };
    let results = vec![
        NoteResult::SelectionChanged {
            selection: Some(NoteSelection {
                selection_id: "s".into(),
                anchor,
                box_css: None,
            }),
        },
        NoteResult::NoteChanged { note },
        NoteResult::NotesListed {
            notes: vec![NoteSummary {
                note_id: "n".into(),
                window,
                resolved: false,
                anchor_state: "attached".into(),
                updated_at_ms: 101,
            }],
        },
        NoteResult::PromptDraft {
            note_id: "n".into(),
            text: "[Chariox note n]".into(),
            attachment: crate::session::PromptAttachment::new(
                "chariox-terminal://prompt-attachment/n/note.txt",
                "text/plain",
                Some("Chariox note.txt".into()),
            )
            .with_contents_base64("bm90ZQ=="),
        },
    ];
    let values = results
        .into_iter()
        .map(|result| serde_json::to_value(LocalDaemonResponse::Notes { result }).unwrap())
        .collect::<Vec<_>>();
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("notes-events-443.json")).unwrap();
    assert_eq!(serde_json::json!(values), expected);
}
