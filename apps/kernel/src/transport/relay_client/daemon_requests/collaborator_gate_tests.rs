use super::*;

// MP-08 / MP-10 / MP-11: a Room member who is not this kernel's owner reaches it through the
// relay to work in that Room. Kernel-global requests (no session scope) are the owner's: the
// collaborator's relay client must be refused before any of them runs, whatever its Room level.
#[tokio::test]
async fn a_room_collaborator_over_the_relay_cannot_use_kernel_global_requests() {
    let mut app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace",
            "worktree",
        ))
        .unwrap();
    let mut room = app.session_state_store().get_session(session.id()).unwrap();
    room.add_member(
        "private-member",
        None,
        crate::session::CollaborationLevel::Private,
    );
    room.add_member(
        "full-member",
        None,
        crate::session::CollaborationLevel::Full,
    );
    app.sessions_mut().restore_session(room);
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let cache = CommandResultCache::default();
    let kernel_global = [
        serde_json::json!({ "ListCredentials": null }),
        serde_json::json!({ "GetCredential": { "id": "any" } }),
        serde_json::json!({ "ListPairedClients": null }),
        serde_json::json!({ "ListRemoteMachines": null }),
        serde_json::json!({ "GetUserConfig": null }),
        serde_json::json!({ "RelayStatus": null }),
        serde_json::json!({ "ListMcpServers": {} }),
        serde_json::json!({ "GetWorkspaceGitOverview": { "workspace_id": "workspace", "worktree_id": "worktree" } }),
    ];
    // Refused before they run, so a missing gate would act on this test's kernel.
    let destructive = [
        serde_json::json!({ "DeleteKernel": null }),
        serde_json::json!({ "ConfigureRelay": {} }),
        serde_json::json!({ "SetUserConfigValue": { "path": "relay.url", "value": "ws://elsewhere" } }),
    ];
    let mut requests: Vec<LocalDaemonRequest> = kernel_global
        .into_iter()
        .chain(destructive)
        .map(|request| serde_json::from_value(request).unwrap())
        .collect();
    requests.push(LocalDaemonRequest::KernelBrowser(
        crate::local::KernelBrowserRequest {
            command: crate::local::KernelBrowserCommand::State,
        },
    ));
    let mut reached = Vec::new();
    for member in ["private-member", "full-member"] {
        for (index, request) in requests.iter().enumerate() {
            let outcome = dispatch_relay_client_request(
                &router,
                &AtomicU64::new(1),
                KernelCaller::for_relay_request(Some(relay_user(member))),
                request.clone(),
                Some(format!("{member}-{index}")),
                &cache,
            )
            .await;
            let refused = matches!(&outcome, RelayDispatchOutcome::RelayError(error)
                if error.message.contains("outside a room"));
            if !refused {
                reached.push(format!("{member}: {request:?}"));
            }
        }
    }
    assert!(
        reached.is_empty(),
        "a collaborator's relay client reached kernel-global requests: {reached:#?}"
    );

    // The owner keeps the kernel; a member keeps what its Room client sends outside a session.
    for (user_id, request) in [
        ("local", serde_json::json!({ "ListCredentials": null })),
        ("local", serde_json::json!({ "GetUserConfig": null })),
        (
            "private-member",
            serde_json::json!({ "GetDaemonHealth": null }),
        ),
        (
            "full-member",
            serde_json::json!({ "GetDaemonHealth": null }),
        ),
    ] {
        let request: LocalDaemonRequest = serde_json::from_value(request).unwrap();
        let outcome = dispatch_relay_client_request(
            &router,
            &AtomicU64::new(1),
            KernelCaller::for_relay_request(Some(relay_user(user_id))),
            request.clone(),
            Some(format!("allowed-{user_id}-{}", request_kind_name(&request))),
            &cache,
        )
        .await;
        assert!(
            matches!(outcome, RelayDispatchOutcome::Response(_)),
            "{user_id} was refused {request:?}"
        );
    }
}

// MP-08 / MP-10 / MP-11: the session picker's older-sessions page is a Room client read like the
// waiting-room snapshot: it reaches the handler, which lists only the caller's own Rooms.
#[tokio::test]
async fn a_room_collaborator_lists_only_its_own_rooms_in_the_older_sessions_page() {
    let mut app = crate::DaemonApp::bootstrap(crate::DaemonConfig::for_tests()).unwrap();
    let mut session_ids = Vec::new();
    for member in [None, Some("full-member"), Some("another-user")] {
        let (session, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace",
                "worktree",
            ))
            .unwrap();
        let mut room = app.session_state_store().get_session(session.id()).unwrap();
        if let Some(member) = member {
            room.add_member(member, None, crate::session::CollaborationLevel::Full);
        }
        app.sessions_mut().restore_session(room);
        session_ids.push(session.id().to_string());
    }
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 8);
    let outcome = dispatch_relay_client_request(
        &router,
        &AtomicU64::new(1),
        KernelCaller::for_relay_request(Some(relay_user("full-member"))),
        LocalDaemonRequest::ListWaitingRoomSessions(crate::local::ListWaitingRoomSessionsRequest {
            inventory_version: None,
            cursor: None,
            limit: None,
        }),
        Some("older-sessions".into()),
        &CommandResultCache::default(),
    )
    .await;
    let response: crate::local::LocalDaemonResponse = match outcome {
        RelayDispatchOutcome::Response(value) => serde_json::from_value(value).unwrap(),
        RelayDispatchOutcome::RelayError(error) => {
            panic!(
                "the collaborator's older-sessions page was refused: {}",
                error.message
            )
        }
    };
    let crate::local::LocalDaemonResponse::WaitingRoomSessionsListed { page } = response else {
        panic!("unexpected response: {response:?}");
    };
    let listed: Vec<&str> = page
        .sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    assert_eq!(listed, vec![session_ids[1].as_str()]);
    assert_eq!(page.total, 1);
}

fn request_kind_name(request: &LocalDaemonRequest) -> String {
    format!("{request:?}")
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect()
}

fn relay_user(user_id: &str) -> RelayCallerIdentity {
    RelayCallerIdentity {
        realm_id: "test".into(),
        subject: format!("client-{user_id}"),
        subject_kind: chariox_relay::auth::RelaySubjectKind::Client,
        expires_at_ms: u64::MAX,
        token_id: None,
        user_id: Some(user_id.into()),
        public_key_thumbprint: None,
    }
}
