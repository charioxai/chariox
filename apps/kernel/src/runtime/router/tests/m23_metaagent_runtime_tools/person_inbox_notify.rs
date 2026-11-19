//! NC-05 / NC-07: an agent notifies the person with `chariox.notify_person`, and the person
//! reads, snoozes and mutes cards through the kernel so every client shows the same Inbox.
use super::*;
use crate::local::{
    GetPersonInboxRequest, InboxCardChange, PersonInboxCard, PersonInboxCardBody,
    PersonInboxOrigin, PersonInboxPriority, SetInboxCardStateRequest, SetInboxMuteRequest,
};

struct Room {
    _env: TestMetaRuntimeEnv,
    router: CommandRouter,
    auth: String,
    session_id: String,
    agent_id: String,
}

fn room(label: &str) -> Room {
    let env = TestMetaRuntimeEnv::new(label);
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, actor) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let run = launch_test_provider(
        &mut app,
        session.id(),
        actor.id(),
        "dev-stub",
        "dev-stub",
        "room-model",
    );
    Room {
        _env: env,
        auth: run.runtime_mcp_auth_token().unwrap().to_string(),
        session_id: session.id().to_owned(),
        agent_id: actor.id().to_owned(),
        router: CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4),
    }
}

impl Room {
    async fn notify(&self, arguments: serde_json::Value) -> Result<String, DaemonError> {
        let result = self
            .router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &self.auth,
                "mcp__chariox__chariox_notify_person",
                arguments,
            )
            .await?;
        assert!(result.ok, "{:?}", result.payload);
        Ok(result.payload["card_id"].as_str().unwrap().to_owned())
    }

    async fn request(
        &self,
        request: LocalDaemonRequest,
    ) -> Result<(Vec<PersonInboxCard>, Vec<PersonInboxOrigin>), DaemonError> {
        let command = KernelCommand::from_local_request("inbox", None, None, &request);
        match self.router.dispatch(command, request).await? {
            LocalDaemonResponse::PersonInbox { cards, mutes } => Ok((cards, mutes)),
            other => panic!("expected the Inbox, got {other:?}"),
        }
    }

    async fn inbox(&self) -> Vec<PersonInboxCard> {
        self.request(LocalDaemonRequest::GetPersonInbox(GetPersonInboxRequest))
            .await
            .unwrap()
            .0
    }

    async fn set(&self, card_ids: &[&str], change: InboxCardChange) -> Result<(), DaemonError> {
        self.request(LocalDaemonRequest::SetInboxCardState(
            SetInboxCardStateRequest {
                card_ids: card_ids.iter().map(|id| id.to_string()).collect(),
                change,
            },
        ))
        .await
        .map(drop)
    }

    async fn mute(&self, muted: bool) -> Vec<PersonInboxOrigin> {
        self.request(LocalDaemonRequest::SetInboxMute(SetInboxMuteRequest {
            origin: PersonInboxOrigin {
                session_id: self.session_id.clone(),
                agent_id: self.agent_id.clone(),
            },
            muted,
        }))
        .await
        .unwrap()
        .1
    }
}

fn notice(card: &PersonInboxCard) -> &crate::local::PersonInboxNotice {
    match &card.body {
        PersonInboxCardBody::Notice(notice) => notice,
        other => panic!("expected a notice, got {other:?}"),
    }
}

#[tokio::test]
async fn an_agent_notifies_the_owner_and_the_card_reaches_the_inbox_unread() {
    let room = room("notify-person");
    let card_id = room
        .notify(
            serde_json::json!({"title":"PR review done","body":"Two comments","priority":"high"}),
        )
        .await
        .unwrap();

    let cards = room.inbox().await;
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].card_id, card_id);
    assert_eq!(cards[0].session_id, room.session_id);
    let sent = notice(&cards[0]);
    assert_eq!(
        (
            sent.title.as_str(),
            sent.body.as_deref(),
            sent.priority,
            sent.read
        ),
        (
            "PR review done",
            Some("Two comments"),
            PersonInboxPriority::High,
            false
        )
    );
    assert_eq!(sent.agent_id, room.agent_id);
    assert!(sent.agent_name.is_some(), "the card names the agent");

    for refused in [
        serde_json::json!({"title":"Done","to":"someone"}),
        serde_json::json!({"title":"Two\nlines"}),
        serde_json::json!({"title":"Done","priority":"urgent"}),
    ] {
        assert!(room.notify(refused.clone()).await.is_err(), "{refused}");
    }
    assert_eq!(room.inbox().await.len(), 1);
}

#[tokio::test]
async fn read_snooze_and_mute_are_kept_by_the_kernel_and_a_needs_you_card_is_only_snoozed() {
    let room = room("inbox-card-state");
    let first = room
        .notify(serde_json::json!({"title":"First"}))
        .await
        .unwrap();
    let second = room
        .notify(serde_json::json!({"title":"Second"}))
        .await
        .unwrap();
    let _pending = room
        .router
        .runtime_state
        .create_kernel_operation_interaction(
            &room.session_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            RuntimeInteraction::for_kernel_operation(
                "waiting",
                "operation-waiting",
                "Install App?",
                "Review this release",
                vec![RuntimeInteractionChoice::new(
                    "allow", "Install", "allow", None,
                )],
            ),
        )
        .await
        .unwrap();
    let read_of = |cards: &[PersonInboxCard], id: &str| {
        notice(cards.iter().find(|card| card.card_id == id).unwrap()).read
    };

    room.set(&[&first, &second], InboxCardChange::Read { read: true })
        .await
        .unwrap();
    let cards = room.inbox().await;
    assert!(read_of(&cards, &first) && read_of(&cards, &second));
    room.set(&[&second], InboxCardChange::Read { read: false })
        .await
        .unwrap();
    assert!(!read_of(&room.inbox().await, &second));
    assert!(
        room.set(&["waiting"], InboxCardChange::Read { read: true })
            .await
            .is_err(),
        "a needs-you card leaves when it is answered, not when it is read"
    );
    assert!(room
        .set(&["no-such-card"], InboxCardChange::Read { read: true })
        .await
        .is_err());

    let until = crate::session::unix_epoch_ms() + 3_600_000;
    room.set(
        &["waiting", &first],
        InboxCardChange::Snooze {
            until_ms: Some(until),
        },
    )
    .await
    .unwrap();
    let cards = room.inbox().await;
    let snoozed = |cards: &[PersonInboxCard], id: &str| {
        cards
            .iter()
            .find(|card| card.card_id == id)
            .unwrap()
            .snoozed_until_ms
    };
    assert_eq!(
        snoozed(&cards, "waiting"),
        Some(until),
        "a snoozed needs-you card stays listed"
    );
    assert_eq!(snoozed(&cards, &first), Some(until));
    assert!(room
        .set(&["waiting"], InboxCardChange::Snooze { until_ms: Some(1) })
        .await
        .is_err());
    room.set(&["waiting"], InboxCardChange::Snooze { until_ms: None })
        .await
        .unwrap();
    assert_eq!(snoozed(&room.inbox().await, "waiting"), None);

    assert_eq!(room.mute(true).await.len(), 1);
    let quiet = room
        .notify(serde_json::json!({"title":"Quiet"}))
        .await
        .unwrap();
    let cards = room.inbox().await;
    assert!(
        read_of(&cards, &quiet),
        "a muted agent's notice arrives read"
    );
    assert!(
        cards.iter().any(|card| card.card_id == "waiting"),
        "a mute never hides a needs-you card"
    );
    assert!(room.mute(false).await.is_empty());
    let loud = room
        .notify(serde_json::json!({"title":"Loud"}))
        .await
        .unwrap();
    assert!(!read_of(&room.inbox().await, &loud));
}
