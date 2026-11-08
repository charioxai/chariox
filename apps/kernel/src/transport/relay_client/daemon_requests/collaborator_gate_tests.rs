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
    let mut reached = Vec::new();
    for member in ["private-member", "full-member"] {
        for (index, request) in kernel_global.iter().enumerate() {
            let request: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
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
