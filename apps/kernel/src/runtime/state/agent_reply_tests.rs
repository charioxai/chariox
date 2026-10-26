//! MP-08 / MP-09 / MP-10 / MP-11: failed delegates cannot orphan requested replies.
use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, InboxEvent, Operation, Outcome};

struct Fixture {
    runtime: KernelRuntimeState,
    room: String,
    parent: String,
    child: String,
    owner: String,
    _worktree: crate::test_support::TestWorktree,
}

impl Fixture {
    fn new() -> Self {
        let worktree = crate::test_support::TestWorktree::new("delegate-reply");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let (room, parent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let child = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(crate::agent::CreateAgentRequest::new(room.id(), "codex"))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::for_user(
                room.id(),
                "client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
                room.owner_user_id(),
            ))
            .unwrap();
        app.prompt_owner_submit_prepared_prompt(
            room.id(),
            crate::session::PromptQueueItem::new(
                "parent-turn",
                attachment.id(),
                parent.id(),
                "work",
                crate::session::PromptStatus::Queued,
            ),
            false,
        )
        .unwrap();
        let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        )
        .runtime_state();
        runtime
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Begin {
                owner: room.owner_user_id().into(),
                room: room.id().into(),
                agent: parent.id().into(),
                prompt: "parent-turn".into(),
                run: None,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        Self {
            runtime,
            room: room.id().into(),
            parent: parent.id().into(),
            child: child.id().into(),
            owner: room.owner_user_id().into(),
            _worktree: worktree,
        }
    }

    fn request(&self, key: &str) -> InboxEvent {
        let Outcome::Event(event) = self
            .runtime
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Send {
                task: "parent-turn".into(),
                prompt: "parent-turn".into(),
                event: InboxEvent {
                    sequence: 0,
                    room_id: self.room.clone(),
                    agent_id: self.child.clone(),
                    source_id: self.parent.clone(),
                    occurrence_id: key.into(),
                    kind: "message".into(),
                    payload: serde_json::json!({"message":"return findings"}),
                    urgent: false,
                    reply_requested: true,
                    state: "pending".into(),
                    prompt_id: None,
                    target_prompt_id: None,
                    provider_run_id: None,
                    attempted_at_ms: None,
                    submit_epoch: None,
                },
            })
            .unwrap()
        else {
            panic!("event")
        };
        event
    }

    fn replies(&self) -> Vec<InboxEvent> {
        self.runtime
            .owned
            .durable_state_store
            .agent_inbox(&self.room, &self.parent, 0)
            .unwrap()
    }

    fn assert_gone(&self, count: usize) {
        let events = self.replies();
        assert_eq!(
            events.len(),
            count,
            "every requested reply needs a terminal result"
        );
        for event in &events {
            assert_eq!(event.kind, "source_lost");
            assert_eq!(event.payload["public_answer"]["kind"], "delegate_gone");
            assert_eq!(event.payload["public_answer"]["delegate_id"], self.child);
        }
        let tasks = self
            .runtime
            .owned
            .durable_state_store
            .agent_tasks(Some(&self.room), Some(&self.parent))
            .unwrap();
        assert!(tasks[0].obligations.iter().all(|o| o.status == "failed"));
        assert!(self
            .runtime
            .owned
            .durable_state_store
            .agent_registrations("parent-turn")
            .unwrap()
            .iter()
            .all(|r| !r.live));
    }
}

#[tokio::test]
async fn delegate_reply_never_started_deleted_settles_every_reply_once() {
    let f = Fixture::new();
    f.request("first");
    f.request("second");
    f.runtime.destroy_agent(&f.child, &f.owner).await.unwrap();
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    f.assert_gone(2);
    let before = f.replies();
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    assert_eq!(
        serde_json::to_value(f.replies()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}

#[tokio::test]
async fn delegate_reply_failed_start_settles_without_deletion() {
    let f = Fixture::new();
    f.request("unverified-profile");
    f.request("also-waiting");
    let error = f
        .runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap_err();
    assert!(
        matches!(error, DaemonError::ProviderProfileRefused { .. }),
        "{error}"
    );
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    f.assert_gone(2);
    assert!(f
        .replies()
        .iter()
        .all(|event| event.payload["public_answer"]["reason"] == "failed_start"));
    assert!(
        f.runtime
            .owned
            .durable_state_store
            .agent_delivery_front(&f.room, &f.child)
            .unwrap()
            .is_none(),
        "terminal failures must retire every affected request"
    );
    // Another delivery pass must not even try the still-unverified profile.
    f.runtime
        .deliver_agent_inbox(&f.room, &f.child)
        .await
        .unwrap();
}

#[tokio::test]
async fn delegate_reply_unsubscribed_then_deleted_survives_restart_and_duplicate_sweep() {
    let f = Fixture::new();
    let request = f.request("lost-source");
    let store = &f.runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::SourceOutcome {
            room: f.room.clone(),
            source: f.child.clone(),
            occurrence: "helper-source-lost".into(),
            success: false,
            public_answer: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    f.assert_gone(1);
    assert_eq!(
        f.replies()[0].payload["public_answer"]["reason"],
        "source_lost"
    );
    store
        .agent_lifecycle(Operation::Unsubscribe {
            task: "parent-turn".into(),
            prompt: "parent-turn".into(),
            registration: format!("completion-reply-{}", request.sequence),
        })
        .unwrap();
    // Reopen the durable read view between source loss and deletion.
    let reopened =
        crate::durable_state::DurableKernelStateStore::open(store.path().to_path_buf()).unwrap();
    assert!(!reopened.agent_registrations("parent-turn").unwrap()[0].live);
    drop(reopened);
    f.runtime.destroy_agent(&f.child, &f.owner).await.unwrap();
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    f.assert_gone(1);
    let reopened =
        crate::durable_state::DurableKernelStateStore::open(store.path().to_path_buf()).unwrap();
    let events = reopened.agent_inbox(&f.room, &f.parent, 0).unwrap();
    assert_eq!(
        serde_json::to_value(&events).unwrap(),
        serde_json::to_value(f.replies()).unwrap()
    );
    assert_eq!(
        reopened
            .agent_tasks(Some(&f.room), Some(&f.parent))
            .unwrap()[0]
            .state,
        ExecutionState::Working
    );
    drop(reopened);
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    assert_eq!(
        serde_json::to_value(f.replies()).unwrap(),
        serde_json::to_value(events).unwrap()
    );
}

#[tokio::test]
async fn delegate_reply_pre_upgrade_accepted_orphan_is_recovered_by_sweep() {
    let f = Fixture::new();
    let event = f.request("accepted-before-upgrade");
    let store = &f.runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Attempt {
            room: f.room.clone(),
            agent: f.child.clone(),
            sequence: event.sequence,
            prompt: format!("agent-event-{}-{}", f.child, event.sequence),
            target: None,
            run: None,
            now: crate::session::unix_epoch_ms(),
            work: None,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Receipt {
            room: f.room.clone(),
            agent: f.child.clone(),
            sequence: event.sequence,
            state: "accepted".into(),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    // Reproduce the old deletion path without the new runtime retirement hook.
    f.runtime
        .owned
        .agent_store
        .destroy_agent(&f.child, &mut f.runtime.owned.session_store.write())
        .unwrap();
    assert!(f.replies().is_empty());
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    f.assert_gone(1);
    let before = f.replies();
    f.runtime.sweep_agent_lifecycle().await.unwrap();
    assert_eq!(before, f.replies());
}
