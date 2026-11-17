use super::*;
use crate::local::{GetPersonInboxRequest, PersonInboxCard};
use crate::session::{RuntimeSession, DEFAULT_LOCAL_USER_ID};

fn room(owner: &str) -> RuntimeSession {
    let mut session = RuntimeSession::new(
        format!("inbox-room-{:016x}", rand::random::<u64>()),
        None,
        "workspace",
        "worktree",
        "machine",
        "kernel",
    );
    session.set_owner_user_id(owner);
    session
}

fn decision(id: &str) -> RuntimeInteraction {
    RuntimeInteraction::for_kernel_operation(
        id,
        &format!("operation-{id}"),
        "Install App?",
        "Review this release",
        vec![
            RuntimeInteractionChoice::new("deny", "Cancel", "deny", None),
            RuntimeInteractionChoice::new("allow", "Install", "allow", None),
        ],
    )
}

async fn inbox(router: &CommandRouter) -> Vec<PersonInboxCard> {
    let request = LocalDaemonRequest::GetPersonInbox(GetPersonInboxRequest);
    let command = KernelCommand::from_local_request("inbox", None, None, &request);
    match router.dispatch(command, request).await.unwrap() {
        LocalDaemonResponse::PersonInbox { cards, .. } => cards,
        other => panic!("expected the Inbox, got {other:?}"),
    }
}

fn card_ids(cards: &[PersonInboxCard]) -> Vec<(&str, &str)> {
    cards
        .iter()
        .map(|card| (card.session_id.as_str(), card.card_id.as_str()))
        .collect()
}

#[tokio::test]
async fn the_inbox_lists_the_owners_cards_from_every_room_newest_first_and_drops_an_answered_one() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (first, second, guest) = (
        room(DEFAULT_LOCAL_USER_ID),
        room(DEFAULT_LOCAL_USER_ID),
        room("someone-else"),
    );
    let ids = [first.id(), second.id(), guest.id()].map(str::to_owned);
    for session in [first, second, guest] {
        app.sessions_mut().restore_session(session);
    }
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let mut pending = Vec::new();
    for (session, owner, id) in [
        (&ids[0], DEFAULT_LOCAL_USER_ID, "older"),
        (&ids[2], "someone-else", "guest"),
        (&ids[1], DEFAULT_LOCAL_USER_ID, "newer"),
    ] {
        pending.push(
            router
                .runtime_state
                .create_kernel_operation_interaction(session, owner, decision(id))
                .await
                .unwrap(),
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    let cards = inbox(&router).await;
    assert_eq!(
        card_ids(&cards),
        [(ids[1].as_str(), "newer"), (ids[0].as_str(), "older")],
        "the owner's Rooms only, newest first"
    );

    let request = LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
        session_id: ids[1].clone(),
        interaction_id: "newer".into(),
        choice_id: "allow".into(),
        custom_reply: None,
        passkey: None,
        passkey_remember_minutes: None,
    });
    let mut command = KernelCommand::from_local_request("answer", None, None, &request);
    command.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
    router.dispatch(command, request).await.unwrap();
    assert_eq!(card_ids(&inbox(&router).await), [(ids[0].as_str(), "older")]);
}

#[tokio::test]
async fn a_watched_inbox_gets_a_raised_card_and_its_answer_within_two_seconds() {
    use crate::transport::kernel_resource_feed::{
        run_kernel_resource_feed, submit_watch_set, watch_inbox,
    };
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let session = room(DEFAULT_LOCAL_USER_ID);
    let session_id = session.id().to_owned();
    app.sessions_mut().restore_session(session);
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        1,
    ));
    let (watches, updates) = watch_inbox();
    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(run_kernel_resource_feed(
        Arc::clone(&router),
        crate::runtime::command::KernelCommandSource::RelayClient,
        updates,
        std::time::Duration::from_secs(60),
        move |event| {
            let sent = events_tx.send(event).is_ok();
            async move { sent }
        },
    ));
    let owner = crate::runtime::command::KernelCaller::for_relay_request(Some(
        chariox_relay::protocol::RelayCallerIdentity {
            realm_id: "test".into(),
            subject: "client-owner".into(),
            subject_kind: chariox_relay::auth::RelaySubjectKind::Client,
            expires_at_ms: u64::MAX,
            token_id: None,
            user_id: Some(DEFAULT_LOCAL_USER_ID.into()),
            public_key_thumbprint: None,
        },
    ));
    let watch = crate::local::KernelResourceWatch {
        resource: "person_inbox".into(),
        params: serde_json::json!({}),
        known_revision: None,
    };
    let refused = submit_watch_set(&watches, vec![watch], owner).await.unwrap();
    assert!(refused.is_empty(), "{refused:?}");
    let mut next_cards = || {
        let events = &mut events;
        async move {
            loop {
                let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
                    .await
                    .expect("an Inbox change within 2 s, not the 10 s unsignalled interval")
                    .unwrap();
                if let KernelEvent::KernelResourceChanged { body, .. } = event {
                    let body = serde_json::to_value(body).unwrap();
                    return body["snapshot"]["data"]["cards"]
                        .as_array()
                        .expect("an Inbox snapshot")
                        .iter()
                        .map(|card| card["card_id"].as_str().unwrap().to_owned())
                        .collect::<Vec<_>>();
                }
            }
        }
    };
    assert!(next_cards().await.is_empty());

    let _pending = router
        .runtime_state
        .create_kernel_operation_interaction(&session_id, DEFAULT_LOCAL_USER_ID, decision("raised"))
        .await
        .unwrap();
    assert_eq!(next_cards().await, ["raised"]);

    router
        .runtime_state
        .resolve_terminal_runtime_interaction(
            &session_id,
            "raised",
            "allow",
            None,
            Some(DEFAULT_LOCAL_USER_ID),
        )
        .await
        .unwrap();
    assert!(next_cards().await.is_empty());

    let session = router
        .runtime_state
        .list_session_health_snapshots()
        .into_iter()
        .find(|session| session.id() == session_id)
        .unwrap();
    let card_id = crate::runtime::person_inbox::notify_person(
        &router.runtime_state,
        &session,
        "agent",
        DEFAULT_LOCAL_USER_ID,
        crate::runtime::person_inbox::PersonNotice {
            title: "Review done".into(),
            body: None,
            link: None,
            priority: crate::local::PersonInboxPriority::Normal,
        },
    )
    .unwrap();
    assert_eq!(next_cards().await, [card_id], "a notice wakes the watched Inbox at once");
}

#[tokio::test]
async fn a_room_collaborator_is_refused_the_owners_inbox() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let request = LocalDaemonRequest::GetPersonInbox(GetPersonInboxRequest);
    let mut command = KernelCommand::from_local_request("guest-inbox", None, None, &request);
    command.source = crate::runtime::command::KernelCommandSource::RelayClient;
    command.caller = crate::runtime::command::KernelCaller::for_relay_request(Some(
        chariox_relay::protocol::RelayCallerIdentity {
            realm_id: "test".into(),
            subject: "client-member".into(),
            subject_kind: chariox_relay::auth::RelaySubjectKind::Client,
            expires_at_ms: u64::MAX,
            token_id: None,
            user_id: Some("member".into()),
            public_key_thumbprint: None,
        },
    ));
    assert!(router.dispatch(command, request).await.is_err());
}
