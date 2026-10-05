use super::*;
use crate::{
    durable_state::app_host_actions::{HostActionCommand, HostOffer, OFFER_MS},
    local::{AcceptAppHostActionRequest, AppHostAction, AppRequestErrorCode},
    runtime::command::{KernelCallerKind, KernelCommandSource},
    session::RuntimeSession,
};

fn setup() -> (
    CommandRouter,
    String,
    crate::durable_state::DurableKernelStateStore,
) {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let mut session = RuntimeSession::new(
        format!("host-offer-{:016x}", rand::random::<u64>()),
        None,
        "workspace",
        "worktree",
        "machine",
        "kernel",
    );
    session.set_owner_user_id("alice");
    let id = session.id().to_owned();
    app.sessions_mut().restore_session(session);
    let store = app.durable_state_store();
    rusqlite::Connection::open(store.path()).unwrap().execute_batch(
        "INSERT INTO app_installations(installation_id,app_id,owner_id,generation,allocated_generation,active_json)
         VALUES('docs','com.example.docs','alice',3,3,'{}')"
    ).unwrap();
    (
        CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1),
        id,
        store,
    )
}
fn offer(store: &crate::durable_state::DurableKernelStateStore, id: &str, expires_ms: u64) {
    store
        .app_host_action(HostActionCommand::Create(HostOffer {
            operation_id: id.into(),
            owner: "alice".into(),
            installation: "docs".into(),
            generation: 3,
            action: AppHostAction::ClipboardWrite {
                text: "copy fixture".into(),
            },
            expires_ms,
        }))
        .unwrap();
}
fn request(session: &str, id: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::AcceptAppHostAction(AcceptAppHostActionRequest {
        session_id: session.into(),
        operation_id: id.into(),
    })
}
async fn accept(
    router: &CommandRouter,
    session: &str,
    id: &str,
    user: &str,
    source: KernelCommandSource,
    kind: KernelCallerKind,
) -> LocalDaemonResponse {
    let request = request(session, id);
    let mut command = KernelCommand::from_local_request("take-host-offer", None, None, &request);
    command.source = source;
    command.caller.caller_kind = kind;
    command.caller.user_id = Some(user.into());
    router.dispatch(command, request).await.unwrap()
}

#[tokio::test]
async fn app_host_accept_projects_to_terminals_and_is_owner_only_human_only_and_one_shot() {
    let (router, session, store) = setup();
    offer(&store, "copy", crate::session::unix_epoch_ms() + OFFER_MS);
    router
        .runtime_state
        .app_host_action_pass(crate::session::unix_epoch_ms())
        .await;
    let snapshot = router
        .runtime_state
        .session_snapshot(&session)
        .await
        .unwrap();
    assert!(snapshot.agents().is_empty());
    assert_eq!(snapshot.active_interactions().len(), 1);
    assert_eq!(
        snapshot.active_interactions()[0].kernel_operation_id(),
        Some("host_action:copy")
    );
    assert!(snapshot.active_interactions()[0]
        .message()
        .contains("copy fixture"));
    assert!(matches!(
        accept(
            &router,
            &session,
            "copy",
            "bob",
            KernelCommandSource::RelayClient,
            KernelCallerKind::RemoteClient
        )
        .await,
        LocalDaemonResponse::AppRequestFailed { .. }
    ));
    for (source, kind) in [
        (
            KernelCommandSource::RelayPeer,
            KernelCallerKind::RemoteKernel,
        ),
        (
            KernelCommandSource::DaemonBackground,
            KernelCallerKind::Metaagent,
        ),
    ] {
        assert!(matches!(
            accept(&router, &session, "copy", "alice", source, kind).await,
            LocalDaemonResponse::AppRequestFailed {
                code: AppRequestErrorCode::Unauthorized
            }
        ));
    }
    assert!(router
        .runtime_state
        .resolve_runtime_interaction(&session, "app_host_copy", "accept_host_action", None)
        .await
        .is_err());
    assert!(router
        .runtime_state
        .resolve_terminal_runtime_interaction(
            &session,
            "app_host_copy",
            "accept_host_action",
            None,
            Some("alice")
        )
        .await
        .is_err());
    assert!(matches!(
        accept(
            &router,
            "wrong-session",
            "copy",
            "alice",
            KernelCommandSource::LocalIpc,
            KernelCallerKind::LocalClient
        )
        .await,
        LocalDaemonResponse::AppRequestFailed { .. }
    ));
    let (a, b) = tokio::join!(
        accept(
            &router,
            &session,
            "copy",
            "alice",
            KernelCommandSource::LocalIpc,
            KernelCallerKind::LocalClient
        ),
        accept(
            &router,
            &session,
            "copy",
            "alice",
            KernelCommandSource::RelayClient,
            KernelCallerKind::RemoteClient
        ),
    );
    assert_eq!(
        [&a, &b]
            .iter()
            .filter(|reply| matches!(reply, LocalDaemonResponse::AppHostActionAccepted { .. }))
            .count(),
        1
    );
    let accepted = if matches!(a, LocalDaemonResponse::AppHostActionAccepted { .. }) {
        a
    } else {
        b
    };
    assert_eq!(
        accepted,
        LocalDaemonResponse::AppHostActionAccepted {
            operation_id: "copy".into(),
            action: AppHostAction::ClipboardWrite {
                text: "copy fixture".into()
            }
        }
    );
    assert!(router
        .runtime_state
        .session_snapshot(&session)
        .await
        .unwrap()
        .active_interactions()
        .is_empty());
}

#[tokio::test]
async fn app_host_decline_and_expiry_never_release_the_action() {
    let (router, session, store) = setup();
    offer(
        &store,
        "decline",
        crate::session::unix_epoch_ms() + OFFER_MS,
    );
    router
        .runtime_state
        .app_host_action_pass(crate::session::unix_epoch_ms())
        .await;
    router
        .runtime_state
        .resolve_terminal_runtime_interaction(
            &session,
            "app_host_decline",
            "decline",
            None,
            Some("alice"),
        )
        .await
        .unwrap();
    // No wait for the durable decline: the shared interaction claim already
    // prevents a late Accept command from winning the decision.
    assert!(matches!(
        accept(
            &router,
            &session,
            "decline",
            "alice",
            KernelCommandSource::LocalIpc,
            KernelCallerKind::LocalClient
        )
        .await,
        LocalDaemonResponse::AppRequestFailed { .. }
    ));
    offer(&store, "expired", crate::session::unix_epoch_ms() - 1);
    router
        .runtime_state
        .app_host_action_pass(crate::session::unix_epoch_ms())
        .await;
    assert!(matches!(
        accept(
            &router,
            &session,
            "expired",
            "alice",
            KernelCommandSource::LocalIpc,
            KernelCallerKind::LocalClient
        )
        .await,
        LocalDaemonResponse::AppRequestFailed { .. }
    ));
}

#[tokio::test]
async fn app_host_take_cannot_settle_a_different_kernel_subject_with_the_same_id() {
    let (router, session, store) = setup();
    offer(&store, "copy", crate::session::unix_epoch_ms() + OFFER_MS);
    let receiver = router
        .runtime_state
        .create_kernel_operation_interaction(
            &session,
            "alice",
            RuntimeInteraction::for_kernel_operation(
                "app_host_copy",
                "validation:copy",
                "Another decision",
                "Unrelated prompt",
                vec![RuntimeInteractionChoice::new(
                    "accept_host_action",
                    "Allow",
                    "allow",
                    None,
                )],
            ),
        )
        .await
        .unwrap();
    assert!(matches!(
        accept(
            &router,
            &session,
            "copy",
            "alice",
            KernelCommandSource::LocalIpc,
            KernelCallerKind::LocalClient
        )
        .await,
        LocalDaemonResponse::AppRequestFailed { .. }
    ));
    assert_eq!(
        router
            .runtime_state
            .session_snapshot(&session)
            .await
            .unwrap()
            .active_interactions()
            .len(),
        1
    );
    router
        .runtime_state
        .timeout_runtime_interaction(&session, "app_host_copy")
        .await
        .unwrap();
    assert_eq!(receiver.await.unwrap().status, "timed_out");
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
fn detached_view(router: &CommandRouter) {
    router
        .runtime_state
        .app_control()
        .user_views()
        .open(
            "alice",
            crate::runtime::app_views::AppViewBinding {
                owner: "alice".into(),
                installation: "docs".into(),
                generation: 3,
                logical_tab: None,
                panel: Default::default(),
            },
            "fixture",
        )
        .unwrap();
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
#[tokio::test]
async fn detached_host_offers_accept_clipboard_and_link_once_without_generic_answer_authority() {
    let (router, room, store) = setup();
    detached_view(&router);
    for (id, action) in [
        (
            "detached-copy",
            AppHostAction::ClipboardWrite {
                text: "copy fixture".into(),
            },
        ),
        (
            "detached-link",
            AppHostAction::OpenLink {
                url: "https://example.invalid/fixture".into(),
            },
        ),
    ] {
        store
            .app_host_action(HostActionCommand::Create(HostOffer {
                operation_id: id.into(),
                owner: "alice".into(),
                installation: "docs".into(),
                generation: 3,
                action: action.clone(),
                expires_ms: crate::session::unix_epoch_ms() + OFFER_MS,
            }))
            .unwrap();
        router
            .runtime_state
            .app_host_action_pass(crate::session::unix_epoch_ms())
            .await;
        assert!(router
            .runtime_state
            .session_snapshot(&room)
            .await
            .unwrap()
            .active_interactions()
            .is_empty());
        let interaction = format!("app_host_{id}");
        assert!(router
            .runtime_state
            .resolve_terminal_runtime_interaction(
                "",
                &interaction,
                "accept_host_action",
                None,
                Some("alice")
            )
            .await
            .is_err());
        assert!(matches!(
            accept(
                &router,
                "",
                id,
                "bob",
                KernelCommandSource::LocalIpc,
                KernelCallerKind::LocalClient
            )
            .await,
            LocalDaemonResponse::AppRequestFailed { .. }
        ));
        assert!(matches!(
            accept(
                &router,
                &room,
                id,
                "alice",
                KernelCommandSource::LocalIpc,
                KernelCallerKind::LocalClient
            )
            .await,
            LocalDaemonResponse::AppRequestFailed { .. }
        ));
        assert_eq!(
            accept(
                &router,
                "",
                id,
                "alice",
                KernelCommandSource::LocalIpc,
                KernelCallerKind::LocalClient
            )
            .await,
            LocalDaemonResponse::AppHostActionAccepted {
                operation_id: id.into(),
                action,
            }
        );
        assert!(matches!(
            accept(
                &router,
                "",
                id,
                "alice",
                KernelCommandSource::RelayClient,
                KernelCallerKind::RemoteClient
            )
            .await,
            LocalDaemonResponse::AppRequestFailed { .. }
        ));
    }
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
#[tokio::test]
async fn detached_host_decline_and_accept_arbitrate_before_one_use_payload_delivery() {
    let (router, _, store) = setup();
    detached_view(&router);
    offer(
        &store,
        "detached-declined",
        crate::session::unix_epoch_ms() + OFFER_MS,
    );
    router
        .runtime_state
        .app_host_action_pass(crate::session::unix_epoch_ms())
        .await;
    router
        .runtime_state
        .resolve_terminal_runtime_interaction(
            "",
            "app_host_detached-declined",
            "decline",
            None,
            Some("alice"),
        )
        .await
        .unwrap();
    assert!(matches!(
        accept(
            &router,
            "",
            "detached-declined",
            "alice",
            KernelCommandSource::LocalIpc,
            KernelCallerKind::LocalClient
        )
        .await,
        LocalDaemonResponse::AppRequestFailed { .. }
    ));
    offer(
        &store,
        "detached-race",
        crate::session::unix_epoch_ms() + OFFER_MS,
    );
    router
        .runtime_state
        .app_host_action_pass(crate::session::unix_epoch_ms())
        .await;
    // Decline arbitrates immediately but updates durable state asynchronously.
    // The pump shows only the oldest pending offer for an installation, so
    // advance it until the terminal can actually see the next offer.
    let subscription =
        LocalDaemonRequest::SubscribeUserAppViews(crate::local::SubscribeUserAppViewsRequest {
            after: None,
            wait_ms: 0,
        });
    let mut observer =
        KernelCommand::from_local_request("observe-host-offer", None, None, &subscription);
    observer.source = KernelCommandSource::LocalIpc;
    observer.caller.caller_kind = KernelCallerKind::LocalClient;
    observer.caller.user_id = Some("alice".into());
    observer.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            router
                .runtime_state
                .app_host_action_pass(crate::session::unix_epoch_ms())
                .await;
            let reply = router
                .runtime_state
                .execute_user_app_view_request(&observer, &subscription)
                .await
                .unwrap()
                .unwrap();
            let LocalDaemonResponse::UserAppViewsChanged { interactions, .. } = reply else {
                panic!("expected owner decision projection");
            };
            if interactions
                .iter()
                .any(|interaction| interaction.id() == "app_host_detached-race")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("detached offer must be projected before racing replies");
    let (taken, declined) = tokio::join!(
        accept(
            &router,
            "",
            "detached-race",
            "alice",
            KernelCommandSource::LocalIpc,
            KernelCallerKind::LocalClient
        ),
        router.runtime_state.resolve_terminal_runtime_interaction(
            "",
            "app_host_detached-race",
            "decline",
            None,
            Some("alice")
        ),
    );
    assert_eq!(
        matches!(taken, LocalDaemonResponse::AppHostActionAccepted { .. }),
        declined.is_err(),
        "take result: {taken:?}; decline result: {declined:?}"
    );
    assert!(matches!(
        accept(
            &router,
            "",
            "detached-race",
            "alice",
            KernelCommandSource::LocalIpc,
            KernelCallerKind::LocalClient
        )
        .await,
        LocalDaemonResponse::AppRequestFailed { .. }
    ));
}
