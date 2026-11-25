//! The task a parent gives its sub-agent is the sub-agent's prompt, not an inbox row to read.
use super::*;
use crate::durable_state::agent_lifecycle::InboxEvent;

struct Team {
    runtime: KernelRuntimeState,
    parent: String,
    child: String,
    peer: String,
    _worktree: crate::test_support::TestWorktree,
}

impl Team {
    fn new() -> Self {
        let worktree = crate::test_support::TestWorktree::new("inbox-brief");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.room_agent_tools = true;
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let (room, parent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        // Spawning a child needs the parent's live turn.
        crate::test_support::admit_room_test_turn(&mut app, room.id(), parent.id());
        let mut sessions = crate::app::KernelSessionService::new(&mut app);
        let child = sessions
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(room.id(), "claude")
                    .with_parent_agent_id(parent.id()),
            )
            .unwrap();
        let peer = sessions
            .spawn_agent(crate::agent::CreateAgentRequest::new(room.id(), "claude"))
            .unwrap();
        let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        )
        .runtime_state();
        Self {
            runtime,
            parent: parent.id().into(),
            child: child.id().into(),
            peer: peer.id().into(),
            _worktree: worktree,
        }
    }

    fn message(&self, to: &str, from: &str, reply_requested: bool) -> InboxEvent {
        InboxEvent {
            sequence: 7,
            room_id: "room".into(),
            agent_id: to.into(),
            source_id: from.into(),
            occurrence_id: "m".into(),
            kind: "message".into(),
            payload: serde_json::json!({"message": "Fix the parser."}),
            urgent: false,
            reply_requested,
            state: "pending".into(),
            prompt_id: None,
            target_prompt_id: None,
            provider_run_id: None,
            arrived_at_ms: None,
            attempted_at_ms: None,
            submit_epoch: None,
            resumed_at_ms: None,
        }
    }
}

#[tokio::test]
async fn a_parents_message_alone_is_its_sub_agents_prompt() {
    let team = Team::new();
    let event = team.message(&team.child, &team.parent, false);
    let prompt = team
        .runtime
        .parent_brief(&team.child, &event, &[])
        .expect("the parent's message is the task");
    assert!(
        prompt.contains("Fix the parser.")
            && prompt.contains("the agent that started you")
            && prompt.contains("inbox message 7"),
        "{prompt}"
    );
    assert!(!prompt.contains("chariox.events.inbox"), "{prompt}");
}

#[tokio::test]
async fn every_other_message_stays_in_the_inbox() {
    let team = Team::new();
    let from_parent = team.message(&team.child, &team.parent, false);
    assert!(
        team.runtime
            .parent_brief(
                &team.child,
                &team.message(&team.child, &team.peer, false),
                &[]
            )
            .is_none(),
        "a peer's text is information to read, not an order"
    );
    assert!(
        team.runtime
            .parent_brief(
                &team.child,
                &team.message(&team.child, &team.parent, true),
                &[]
            )
            .is_none(),
        "a question with a reply requested keeps its own reply wake"
    );
    assert!(
        team.runtime
            .parent_brief(
                &team.child,
                &from_parent,
                std::slice::from_ref(&from_parent)
            )
            .is_none(),
        "a wake that carries more events is read from the inbox"
    );
    assert!(
        team.runtime
            .parent_brief(
                &team.peer,
                &team.message(&team.peer, &team.parent, false),
                &[]
            )
            .is_none(),
        "an agent nobody started has no parent text"
    );
}
