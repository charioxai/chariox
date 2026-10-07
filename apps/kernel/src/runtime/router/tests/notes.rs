//! MD-N3–N5 / MP-08 / MP-10 / MP-11: exercise kernel admission, not a parallel test store.
use super::*;
use crate::local::{NoteAnchor, NoteCommand, NoteResult, NoteTextQuote, NoteWindow, NotesRequest};
use serde_json::json;

// MD-N5: exercise the Apps 416 authenticated human terminal boundary.
fn terminal_command(id: &str, request: &LocalDaemonRequest) -> KernelCommand {
    let mut command = KernelCommand::from_local_request(id, None, None, request);
    command.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
    command
}

async fn request(router: &CommandRouter, command: NoteCommand) -> NoteResult {
    let request = LocalDaemonRequest::Notes(NotesRequest { command });
    let command = terminal_command(&format!("MD-notes-{}", rand::random::<u64>()), &request);
    let LocalDaemonResponse::Notes { result } = router.dispatch(command, request).await.unwrap()
    else {
        panic!("note result expected")
    };
    result
}
async fn focus(router: &CommandRouter, session: &str, agent: &str) {
    let request = LocalDaemonRequest::FocusAgent(FocusAgentRequest {
        session_id: session.into(),
        agent_id: agent.into(),
    });
    router
        .dispatch(
            terminal_command(&format!("MD-notes-focus-{agent}"), &request),
            request,
        )
        .await
        .unwrap();
}
#[test]
fn notes_focused_mcp_owner_terminal_and_prompt_draft() {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(check());
}
async fn check() {
    let workspace = crate::test_support::TestWorktree::new("mdnotes-mcp");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let second = spawn_test_agent(&mut app, session.id(), "second", "dev-stub");
    let first_run = launch_test_provider(
        &mut app,
        session.id(),
        first.id(),
        "dev-stub",
        "dev-stub",
        "default",
    );
    let second_run = launch_test_provider(
        &mut app,
        session.id(),
        second.id(),
        "dev-stub",
        "dev-stub",
        "default",
    );
    let a = first_run.runtime_mcp_auth_token().unwrap();
    let b = second_run.runtime_mcp_auth_token().unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let app_future = router.runtime_state.notes_drill_app_view(
        session.id(),
        crate::runtime::browser_controller_app_view::BrowserAppViewRequest::Calls,
    );
    assert!(
        std::mem::size_of_val(&app_future) < 64 * 1024,
        "MD-N5: App fixture dispatch must fit ordinary caller stacks"
    );
    drop(app_future);
    focus(&router, session.id(), first.id()).await;
    let names = |token: &str| {
        router
            .runtime_tool_specs_for_auth_token(token)
            .into_iter()
            .map(|s| s.name)
            .collect::<Vec<_>>()
    };
    assert!(names(a).contains(&"chariox.load_notes".into()));
    assert!(!names(a).contains(&"chariox.read_note".into()));
    assert!(!names(b).contains(&"chariox.load_notes".into()));
    router
        .dispatch_authenticated_runtime_tool_call(a, "chariox.load_notes", json!({}))
        .await
        .unwrap();
    assert!(
        !names(a).contains(&"chariox.kernel_browser".into()),
        "MD-N4: loading notes must not load browser tools"
    );
    let window = NoteWindow::Panel {
        window_id: "transcript-message-42".into(),
    };
    let NoteResult::SelectionChanged {
        selection: Some(selection),
    } = request(
        &router,
        NoteCommand::ReportSelection {
            anchor: NoteAnchor {
                window: window.clone(),
                url: None,
                document_id: None,
                hint: Some("message-42".into()),
                quote: NoteTextQuote {
                    exact: "selected quote".into(),
                    prefix: "before".into(),
                    suffix: "after".into(),
                },
            },
            box_css: None,
        },
    )
    .await
    else {
        panic!("selection expected")
    };
    let NoteResult::NoteChanged { note } = request(
        &router,
        NoteCommand::Create {
            selection_id: selection.selection_id,
            comment: "explain this".into(),
        },
    )
    .await
    else {
        panic!("note expected")
    };
    let list = router
        .dispatch_authenticated_runtime_tool_call(a, "chariox.list_notes", json!({"window":window}))
        .await
        .unwrap();
    assert_eq!(list.payload["notes"][0]["note_id"], note.note_id);
    assert!(
        list.payload["notes"][0].get("comment").is_none(),
        "listing is a bounded summary"
    );
    let read = router
        .dispatch_authenticated_runtime_tool_call(
            a,
            "chariox.read_note",
            json!({"note_id":note.note_id}),
        )
        .await
        .unwrap();
    assert_eq!(read.payload["note"]["comment"], "explain this");
    for name in [
        "chariox.list_notes",
        "chariox.read_note",
        "chariox.reply_to_note",
        "chariox.resolve_note",
    ] {
        assert!(router
            .dispatch_authenticated_runtime_tool_call(
                b,
                name,
                json!({"note_id":note.note_id,"window":window,"comment":"forbidden"})
            )
            .await
            .is_err());
    }
    router
        .dispatch_authenticated_runtime_tool_call(
            a,
            "chariox.reply_to_note",
            json!({"note_id":note.note_id,"comment":"answer"}),
        )
        .await
        .unwrap();
    let NoteResult::PromptDraft {
        text, attachment, ..
    } = request(
        &router,
        NoteCommand::Ask {
            note_id: note.note_id.clone(),
        },
    )
    .await
    else {
        panic!("draft expected")
    };
    assert!(text.contains("selected quote") && text.contains("explain this"));
    assert_eq!(attachment.mime(), "text/plain");
    assert!(
        !router
            .runtime_state
            .session_snapshot(session.id())
            .await
            .unwrap()
            .has_active_prompt(),
        "Ask never sends a prompt"
    );
    focus(&router, session.id(), second.id()).await;
    assert!(router
        .dispatch_authenticated_runtime_tool_call(
            a,
            "chariox.read_note",
            json!({"note_id":note.note_id})
        )
        .await
        .is_ok());
    // MP-11: retained note reads and authored effects survive focus change.
    for (name, args) in [
        (
            "chariox.reply_to_note",
            json!({"note_id":note.note_id,"comment":"retained reply"}),
        ),
        ("chariox.resolve_note", json!({"note_id":note.note_id})),
    ] {
        router
            .dispatch_authenticated_runtime_tool_call(a, name, args)
            .await
            .unwrap();
    }
    router
        .dispatch_authenticated_runtime_tool_call(b, "chariox.load_kernel_browser", json!({}))
        .await
        .unwrap();
    assert!(
        !names(b).contains(&"chariox.read_note".into()),
        "MD-N4: loading browser must not load notes"
    );
    assert!(router
        .dispatch_authenticated_runtime_tool_call(
            b,
            "chariox.read_note",
            json!({"note_id":note.note_id})
        )
        .await
        .is_err());
    router
        .dispatch_authenticated_runtime_tool_call(b, "chariox.load_notes", json!({}))
        .await
        .unwrap();
    let resolved = router
        .dispatch_authenticated_runtime_tool_call(
            b,
            "chariox.resolve_note",
            json!({"note_id":note.note_id}),
        )
        .await
        .unwrap();
    assert_eq!(resolved.payload["note"]["resolved"], true);
    let request = LocalDaemonRequest::Notes(NotesRequest {
        command: NoteCommand::Read {
            note_id: note.note_id,
        },
    });
    let command = KernelCommand::from_local_request_with_source(
        "MD-notes-provider-forgery",
        KernelCommandSource::RelayPeer,
        None,
        None,
        &request,
    );
    assert!(router.dispatch(command, request).await.is_err());
    router.runtime_state.shutdown_cleanup().await.unwrap();
}

#[path = "notes/live.rs"]
mod live;

#[path = "notes/hardening.rs"]
mod hardening;
