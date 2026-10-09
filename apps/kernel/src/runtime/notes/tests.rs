//! MD-N1–N3 / MP-08 / MP-10 / MP-11: synthetic owner data, no credentials.
use super::*;
fn anchor() -> NoteAnchor {
    NoteAnchor {
        window: NoteWindow::Panel {
            window_id: "message-42".into(),
        },
        url: None,
        document_id: None,
        hint: Some("node-42".into()),
        quote: crate::local::NoteTextQuote {
            exact: "selected Unicode 🙂 text".into(),
            prefix: "before ".into(),
            suffix: " after".into(),
        },
    }
}
#[test]
fn notes_owner_persistence_reply_resolve_and_audit() {
    let root = crate::test_support::TestWorktree::new("mdnotes-store");
    let store = NoteStore::new(root.path().to_path_buf());
    let selection = store.selection("alice", anchor(), None).unwrap();
    assert!(store
        .create("bob", &selection.selection_id, "explain")
        .is_err());
    let note = store
        .create("alice", &selection.selection_id, "explain")
        .unwrap();
    assert_eq!(
        store
            .create("alice", &selection.selection_id, "explain")
            .unwrap(),
        note
    );
    assert!(store
        .create("alice", &selection.selection_id, "changed")
        .is_err());
    assert!(store.read("bob", &note.note_id, "agent:other").is_err());
    assert!(store
        .list("bob", &anchor().window, "bob")
        .unwrap()
        .is_empty());
    let restarted = NoteStore::new(root.path().to_path_buf());
    let reply = restarted
        .reply("alice", &note.note_id, "agent:focused", "answer")
        .unwrap();
    assert_eq!(reply.replies[0].author, "agent:focused");
    assert!(
        restarted
            .resolve("alice", &note.note_id, "agent:focused")
            .unwrap()
            .resolved
    );
    assert!(restarted
        .reply("alice", &note.note_id, "agent:focused", "late")
        .is_err());
    restarted
        .with_db(|db| {
            let operations: String = db
                .query_row(
                    "SELECT group_concat(operation) FROM note_audit WHERE owner='alice'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(operations, "create,reply,resolve");
            Ok(())
        })
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(root.path().join("notes"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(root.path().join("notes/notes.sqlite3"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[test]
fn notes_quote_survives_missing_ambiguous_and_unavailable_documents() {
    let root = crate::test_support::TestWorktree::new("mdnotes-anchor");
    let store = NoteStore::new(root.path().to_path_buf());
    let selection = store.selection("alice", anchor(), None).unwrap();
    let note = store
        .create("alice", &selection.selection_id, "explain")
        .unwrap();
    for state in ["missing", "ambiguous", "unavailable", "attached"] {
        let changed = store
            .reanchor(
                "alice",
                &note.note_id,
                "alice",
                state,
                None,
                Some("new-document".into()),
            )
            .unwrap();
        assert_eq!(changed.anchor.quote, note.anchor.quote);
        assert_eq!(changed.anchor_state, state);
    }
}
#[test]
fn notes_ask_is_an_existing_marked_attachment_and_does_not_resolve() {
    use base64::Engine;
    let root = crate::test_support::TestWorktree::new("mdnotes-ask");
    let store = NoteStore::new(root.path().to_path_buf());
    let selection = store.selection("alice", anchor(), None).unwrap();
    let note = store
        .create("alice", &selection.selection_id, "please explain")
        .unwrap();
    let crate::local::NoteResult::PromptDraft {
        text, attachment, ..
    } = store.ask("alice", &note.note_id, "alice").unwrap()
    else {
        panic!("draft expected")
    };
    assert!(text.starts_with("[Chariox note "));
    assert!(text.contains(&note.anchor.quote.exact));
    assert!(text.contains("please explain"));
    assert_eq!(attachment.mime(), "text/plain");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(attachment.contents_base64().unwrap())
            .unwrap(),
        text.as_bytes()
    );
    assert!(
        !store
            .read("alice", &note.note_id, "alice")
            .unwrap()
            .resolved
    );
}
#[test]
fn notes_capture_polling_reuses_receipt_and_bounds_untrusted_input() {
    let root = crate::test_support::TestWorktree::new("mdnotes-poll");
    let store = NoteStore::new(root.path().to_path_buf());
    let first = store.selection("alice", anchor(), None).unwrap();
    for _ in 0..140 {
        assert_eq!(
            store
                .selection("alice", anchor(), None)
                .unwrap()
                .selection_id,
            first.selection_id
        );
    }
    let mut invalid = anchor();
    invalid.quote.exact = "x".repeat(16385);
    assert!(store.selection("alice", invalid, None).is_err());
    assert!(store
        .selection(
            "alice",
            anchor(),
            Some(NoteBox {
                x: f64::NAN,
                y: 0.,
                width: 1.,
                height: 1.
            })
        )
        .is_err());
    assert!(store.create("alice", &first.selection_id, "").is_err());
}

#[test]
fn notes_stable_tab_listing_survives_browser_generation_change() {
    let root = crate::test_support::TestWorktree::new("mdnotes-generation");
    let store = NoteStore::new(root.path().to_path_buf());
    let mut original = anchor();
    original.window = NoteWindow::KernelBrowser {
        tab_id: "stable-tab".into(),
        generation: 1,
    };
    let selection = store.selection("alice", original.clone(), None).unwrap();
    let note = store
        .create("alice", &selection.selection_id, "explain")
        .unwrap();
    let restored = NoteWindow::KernelBrowser {
        tab_id: "stable-tab".into(),
        generation: 2,
    };
    assert_eq!(
        store.list("alice", &restored, "alice").unwrap()[0].note_id,
        note.note_id
    );
    assert!(store.list("bob", &restored, "bob").unwrap().is_empty());
    assert!(store
        .list(
            "alice",
            &NoteWindow::KernelBrowser {
                tab_id: "other-tab".into(),
                generation: 2
            },
            "alice"
        )
        .unwrap()
        .is_empty());
    original.window = restored;
    let fresh = store.selection("alice", original, None).unwrap();
    assert_ne!(
        fresh.selection_id, selection.selection_id,
        "new selections must keep their generation fence"
    );
}
