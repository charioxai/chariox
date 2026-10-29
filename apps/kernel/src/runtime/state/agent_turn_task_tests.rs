//! MP-08 / MP-09 / MP-10 / MP-11: tasks are kept after they finish, so the reads made on
//! every turn take that turn's task by index instead of decoding the agent's history.
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, DECODED_TASKS};

/// Tasks decoded while a turn that already has its task ends, after `finished` earlier turns.
fn decoded_at_turn_end(finished: usize) -> usize {
    let worktree = crate::test_support::TestWorktree::new("turn-task");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (room, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let runtime = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    )
    .runtime_state();
    let store = &runtime.owned.durable_state_store;
    let begin = |prompt: &str| {
        store
            .agent_lifecycle(Operation::Begin {
                owner: room.owner_user_id().into(),
                room: room.id().into(),
                agent: agent.id().into(),
                prompt: prompt.into(),
                run: None,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
    };
    for i in 0..finished {
        let prompt = format!("turn-{i}");
        begin(&prompt);
        store
            .agent_lifecycle(Operation::EndFailedTurn {
                task: prompt.clone(),
                prompt,
                reason: "provider run failed".into(),
            })
            .unwrap();
    }
    begin("current");
    let current = crate::session::PromptQueueItem::new(
        "current",
        "attachment",
        agent.id(),
        "work",
        crate::session::PromptStatus::Running,
    );
    DECODED_TASKS.with(|n| n.set(0));
    runtime
        .owned
        .ensure_agent_task(room.id(), agent.id(), &current, None)
        .unwrap();
    let task = store
        .agent_turn_task(room.id(), agent.id(), "current")
        .unwrap()
        .unwrap();
    assert!(!store
        .agent_has_other_unfinished_task(room.id(), agent.id(), &task.task_id)
        .unwrap());
    DECODED_TASKS.with(|n| n.get())
}

#[test]
fn a_turn_end_reads_only_its_own_task() {
    let (few, many) = (decoded_at_turn_end(2), decoded_at_turn_end(30));
    assert_eq!(
        few, many,
        "a turn end decoded {few} tasks after 2 earlier turns and {many} after 30"
    );
}
