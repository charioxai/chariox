//! Phase 5: real runtime admission across focus changes and simultaneous owners.
use super::*;
use crate::local::{KernelBrowserCommand, KernelBrowserRequest, NoteRecord};

#[test]
fn queued_browser_observation_cannot_regain_authority_after_revoke_and_refocus() {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(Box::pin(check_queued_focus()));
}
async fn check_queued_focus() {
    let workspace = crate::test_support::TestWorktree::new("md-hardening-focus");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let second = spawn_test_agent(&mut app, session.id(), "other", "dev-stub");
    let run = launch_test_provider(
        &mut app,
        session.id(),
        first.id(),
        "dev-stub",
        "dev-stub",
        "default",
    );
    let token = run.runtime_mcp_auth_token().unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let owner = router
        .runtime_state
        .provider_account_authority_owner_user_id(first.owner_user_id());
    router
        .runtime_state
        .install_kernel_browser_fixture(&owner, workspace.path());
    focus(&router, session.id(), first.id()).await;
    router
        .dispatch_authenticated_runtime_tool_call(token, "chariox.load_kernel_browser", json!({}))
        .await
        .unwrap();
    let barrier = router.runtime_state.kernel_browser_fixture_barrier(&owner);
    let guard = barrier.write_owned().await;
    let pending = router.dispatch_authenticated_runtime_tool_call(
        token,
        "chariox.kernel_browser",
        json!({"command":{"op":"state"}}),
    );
    tokio::pin!(pending);
    // Poll admission while the observation barrier is held; it cannot complete.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut pending)
            .await
            .is_err()
    );
    focus(&router, session.id(), second.id()).await;
    // MP-08/MP-11: focus changes retain grants; explicit revocation retires
    // this request's epoch before a new focus can create its successor.
    let revoke = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
        command: KernelBrowserCommand::RevokeGrants {
            agent_id: Some(first.id().into()),
        },
    });
    router
        .dispatch(terminal_command("md-notes-revoke-first", &revoke), revoke)
        .await
        .unwrap();
    focus(&router, session.id(), first.id()).await;
    drop(guard);
    assert!(
        pending.await.is_err(),
        "a previous focus epoch must never be revived"
    );
    let names = router
        .runtime_tool_specs_for_auth_token(token)
        .into_iter()
        .map(|s| s.name)
        .collect::<Vec<_>>();
    assert!(
        !names.contains(&"chariox.kernel_browser".into()),
        "refocus requires an explicit reload"
    );
    router.runtime_state.shutdown_cleanup().await.unwrap();
}

/// Native drill extension: all three real surfaces and both provider admissions.
pub(super) async fn check_native(
    router: &CommandRouter,
    session: &str,
    first: &str,
    second: &str,
    a: &str,
    b: &str,
    notes: &[NoteRecord],
) -> Vec<serde_json::Value> {
    let mut checks = Vec::new();
    let names = |token: &str| {
        router
            .runtime_tool_specs_for_auth_token(token)
            .into_iter()
            .map(|s| s.name)
            .collect::<Vec<_>>()
    };
    assert!(!names(b).contains(&"chariox.load_kernel_browser".into()));
    assert!(!names(b).contains(&"chariox.load_notes".into()));
    assert!(router
        .dispatch_authenticated_runtime_tool_call(b, "chariox.load_notes", json!({}))
        .await
        .is_err_and(|error| matches!(error, DaemonError::UserDomainRefused { .. })));
    checks.push(json!({"case":"Room agent has no user-domain loader without explicit focus","surface":"browser/notes","client":"unfocused-agent","status":"PASS","refusals_attributed":true}));
    focus(router, session, second).await;
    for loader in ["chariox.load_notes", "chariox.load_kernel_browser"] {
        assert!(!names(a).contains(&loader.into()));
        assert!(router
            .dispatch_authenticated_runtime_tool_call(a, loader, json!({}))
            .await
            .is_err_and(|error| matches!(error, DaemonError::UserDomainRefused { .. })));
    }
    assert!(!names(b).contains(&"chariox.read_note".into()));
    router
        .dispatch_authenticated_runtime_tool_call(b, "chariox.load_kernel_browser", json!({}))
        .await
        .unwrap();
    assert!(!names(b).contains(&"chariox.read_note".into()));
    router
        .dispatch_authenticated_runtime_tool_call(b, "chariox.load_notes", json!({}))
        .await
        .unwrap();
    for note in notes {
        assert!(router
            .dispatch_authenticated_runtime_tool_call(
                a,
                "chariox.read_note",
                json!({"note_id":note.note_id})
            )
            .await
            .is_err_and(|error| matches!(error, DaemonError::UserDomainRefused { .. })));
        let current = router
            .dispatch_authenticated_runtime_tool_call(
                b,
                "chariox.read_note",
                json!({"note_id":note.note_id}),
            )
            .await
            .unwrap();
        assert_eq!(current.payload["note"]["note_id"], note.note_id);
        if let NoteWindow::KernelBrowser { tab_id, generation } = &note.anchor.window {
            let state = router
                .dispatch_authenticated_runtime_tool_call(
                    b,
                    "chariox.kernel_browser",
                    json!({"command":{"op":"state"}}),
                )
                .await
                .unwrap();
            let document = state.payload["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|tab| tab["tab_id"] == *tab_id)
                .unwrap()["document_id"]
                .clone();
            assert!(router
                .dispatch_authenticated_runtime_tool_call(
                    a,
                    "chariox.kernel_browser",
                    json!({"command":{"op":"snapshot","tab_id":tab_id,"generation":generation}})
                )
                .await
                .is_err_and(|error| matches!(error, DaemonError::UserDomainRefused { .. })));
            assert!(router
                .dispatch_authenticated_runtime_tool_call(
                    b,
                    "chariox.kernel_browser",
                    json!({"command":{"op":"snapshot","tab_id":tab_id,"generation":generation}})
                )
                .await
                .is_ok());
            if note
                .anchor
                .url
                .as_deref()
                .is_some_and(|url| url.starts_with("https://app."))
            {
                assert!(router.dispatch_authenticated_runtime_tool_call(b,"chariox.kernel_browser",json!({"command":{"op":"input","tab_id":tab_id,"generation":generation,"input":{"kind":"key","key":"Tab"}},"document_id":document})).await.is_err_and(|error| matches!(error, DaemonError::UserDomainRefused { .. })), "focused browser tools must not act as the human App frontend");
                checks.push(json!({"case":"focused agent can observe App host but cannot spoof human App input","surface":"user-App-host","client":"focused-agent/unfocused-agent","status":"PASS","refusals_attributed":true}));
            }
        }
        let request = LocalDaemonRequest::Notes(NotesRequest {
            command: NoteCommand::Read {
                note_id: note.note_id.clone(),
            },
        });
        let mut caller = terminal_command("MD-H-foreign-note", &request);
        caller.caller.user_id = Some("collaborator-fixture".into());
        assert!(
            router
                .dispatch(caller, request)
                .await
                .is_err_and(|error| matches!(error, DaemonError::UserDomainRefused { .. })),
            "Room membership does not grant private notes"
        );
        let list = LocalDaemonRequest::Notes(NotesRequest {
            command: NoteCommand::List {
                window: note.anchor.window.clone(),
            },
        });
        let mut caller = terminal_command("MD-H-foreign-list", &list);
        caller.caller.user_id = Some("collaborator-fixture".into());
        let LocalDaemonResponse::Notes {
            result: NoteResult::NotesListed { notes: listed },
        } = router.dispatch(caller, list).await.unwrap()
        else {
            panic!("notes list expected")
        };
        assert!(listed.is_empty());
        checks.push(json!({"case":"focus moves private notes atomically; Room collaborator cannot read owner records","surface":note.anchor.window,"client":"focused-agent/unfocused-agent/Room-member","status":"PASS","refusals_attributed":true}));
    }
    focus(router, session, first).await;
    assert!(!names(a).contains(&"chariox.read_note".into()));
    assert!(!names(a).contains(&"chariox.kernel_browser".into()));
    for note in notes {
        assert!(router
            .dispatch_authenticated_runtime_tool_call(
                b,
                "chariox.read_note",
                json!({"note_id":note.note_id})
            )
            .await
            .is_err_and(|error| matches!(error, DaemonError::UserDomainRefused { .. })));
        assert!(router
            .dispatch_authenticated_runtime_tool_call(
                a,
                "chariox.read_note",
                json!({"note_id":note.note_id})
            )
            .await
            .is_err_and(|error| matches!(error, DaemonError::UserDomainRefused { .. })));
    }
    router
        .dispatch_authenticated_runtime_tool_call(a, "chariox.load_notes", json!({}))
        .await
        .unwrap();
    let request = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
        command: KernelBrowserCommand::State,
    });
    let command = KernelCommand::from_local_request_with_source(
        "MD-H-forged-provider",
        KernelCommandSource::RelayPeer,
        None,
        None,
        &request,
    );
    assert!(router
        .dispatch(command, request)
        .await
        .is_err_and(|error| matches!(error, DaemonError::UserDomainRefused { .. })));
    checks.push(json!({"case":"refocus clears both lazy tool grants; forged provider transport rejected","surface":"browser/notes","client":"focused-agent/relay-peer","status":"PASS","refusals_attributed":true}));
    checks
}
