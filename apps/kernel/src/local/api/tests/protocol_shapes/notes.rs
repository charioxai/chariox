//! MD-N3 / MP-08 / MP-11: protocol 447 freezes every note request/event seam.
use super::*;
use crate::local::{NoteAnchor, NoteCommand, NoteResult, NoteTextQuote, NoteWindow, NotesRequest};
#[test]
fn notes_protocol_443_shapes_and_rejected_authority_claims() {
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
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
        NoteCommand::CaptureSelection { window },
        NoteCommand::ReportSelection {
            anchor,
            box_css: None,
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
        serde_json::json!({"Notes":{"user_id":"forged","command":{"op":"capture_selection","window":{"kind":"panel","window_id":"x"}}}}),
        serde_json::json!({"Notes":{"command":{"op":"resolve","note_id":"n"}}}),
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
    use crate::local::NoteSelection;
    assert_eq!(
        LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
    let window = NoteWindow::Panel {
        window_id: "message-42".into(),
    };
    let anchor = NoteAnchor {
        window,
        url: None,
        document_id: None,
        hint: Some("node-42".into()),
        quote: NoteTextQuote {
            exact: "quote".into(),
            prefix: "before".into(),
            suffix: "after".into(),
        },
    };
    let results = vec![NoteResult::SelectionChanged {
        selection: Some(NoteSelection {
            anchor,
            box_css: None,
        }),
    }];
    let values = results
        .into_iter()
        .map(|result| serde_json::to_value(LocalDaemonResponse::Notes { result }).unwrap())
        .collect::<Vec<_>>();
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("notes-events-443.json")).unwrap();
    assert_eq!(serde_json::json!(values), expected);
}

// Notes in prompts (allocated 566/174): a note is an inline `PromptAttachment` with the note
// mime. The kernel turns it into hidden context and keeps the attachment for history chips.
#[test]
fn prompt_notes_566_snapshot() {
    use sha2::{Digest, Sha256};
    const {
        assert!(LOCAL_DAEMON_PROTOCOL_VERSION >= 566);
    };
    const {
        assert!(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION >= 174);
    };
    let bytes = include_bytes!("prompt-notes-566.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "f6b116624c6591676461cc42518775102004b2a2e717e9d2a140b001ae406849"
    );
    let snapshot: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    let request: crate::local::LocalDaemonRequest =
        serde_json::from_value(snapshot["request"].clone()).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), snapshot["request"]);
    let crate::local::LocalDaemonRequest::SubmitPrompt(submit) = request else {
        panic!("expected SubmitPrompt")
    };
    assert_eq!(
        submit.attachments[0].mime(),
        crate::prompt_notes::NOTE_ATTACHMENT_MIME
    );
    crate::prompt_notes::admit(submit.prompt, &submit.attachments).unwrap();
}
