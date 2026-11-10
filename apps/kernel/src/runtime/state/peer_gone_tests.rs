//! MP-08 / MP-09 / MP-10 / MP-11: a follower hears when the agent it follows is removed.
use super::*;
use crate::durable_state::agent_lifecycle::{InboxEvent, Operation, Registration};

struct Fixture {
    runtime: KernelRuntimeState,
    room: String,
    follower: String,
    peer_room: String,
    peer: String,
    owner: String,
    _worktrees: [crate::test_support::TestWorktree; 2],
}

impl Fixture {
    /// A follower with a running turn in one room, and `peer` in another room of the same owner
    /// (or in the follower's own room).
    fn new(follow_registration_is_peer: bool, same_room: bool) -> Self {
        let first = crate::test_support::TestWorktree::new("peer-gone-a");
        let second = crate::test_support::TestWorktree::new("peer-gone-b");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let (room, follower) = crate::app::KernelSessionService::new(&mut app)
            .create_session(first.session_request())
            .unwrap();
        let (peer_room, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(second.session_request())
            .unwrap();
        let peer = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(crate::agent::CreateAgentRequest::new(
                if same_room { room.id() } else { peer_room.id() },
                "codex",
            ))
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
                "follower-turn",
                attachment.id(),
                follower.id(),
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
        let store = &runtime.owned.durable_state_store;
        store
            .agent_lifecycle(Operation::Begin {
                owner: room.owner_user_id().into(),
                room: room.id().into(),
                agent: follower.id().into(),
                prompt: "follower-turn".into(),
                run: None,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::Subscribe {
                task: "follower-turn".into(),
                prompt: "follower-turn".into(),
                registration: Registration {
                    id: "follow-peer".into(),
                    task_id: "follower-turn".into(),
                    source_id: peer.id().into(),
                    obligation_id: None,
                    source_cursor: 0,
                    live: true,
                    peer: follow_registration_is_peer,
                },
            })
            .unwrap();
        Self {
            runtime,
            room: room.id().into(),
            follower: follower.id().into(),
            peer_room: peer_room.id().into(),
            peer: peer.id().into(),
            owner: room.owner_user_id().into(),
            _worktrees: [first, second],
        }
    }

    fn inbox(&self) -> Vec<InboxEvent> {
        self.runtime
            .owned
            .durable_state_store
            .agent_inbox(&self.room, &self.follower, 0)
            .unwrap()
    }

    fn assert_told_once(&self, kind: &str) {
        let events = self.inbox();
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].kind, kind);
        assert_eq!(events[0].source_id, self.peer);
        assert!(
            events[0]
                .payload
                .get("public_answer")
                .is_none_or(|a| a.is_null()),
            "{events:?}"
        );
    }
}

#[tokio::test]
async fn deleting_a_followed_peer_in_another_room_tells_its_follower() {
    let f = Fixture::new(true, false);
    f.runtime.destroy_agent(&f.peer, &f.owner).await.unwrap();
    f.assert_told_once("peer_event");
    assert_eq!(f.inbox()[0].payload["event"], "turn_failed");
}

#[tokio::test]
async fn ending_the_room_of_a_followed_peer_tells_its_follower() {
    let f = Fixture::new(true, false);
    f.runtime.delete_session_id(&f.peer_room).await.unwrap();
    f.assert_told_once("peer_event");
}

#[tokio::test]
async fn deleting_an_agent_followed_in_its_own_room_is_told_the_same_way() {
    let f = Fixture::new(false, true);
    f.runtime.destroy_agent(&f.peer, &f.owner).await.unwrap();
    f.assert_told_once("source_lost");
}
