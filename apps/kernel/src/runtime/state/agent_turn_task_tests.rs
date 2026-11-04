//! MP-08 / MP-09 / MP-10 / MP-11: tasks are kept after they finish, so the reads made on
//! every turn take that turn's task by index instead of decoding the agent's history.
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, DECODED_TASKS};

/// Tasks decoded by `act` on a turn that already has its task, after `finished` earlier turns.
fn decoded_during(
    finished: usize,
    act: impl FnOnce(&KernelRuntimeState, &str, &str, &crate::session::PromptQueueItem),
) -> usize {
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
    act(&runtime, room.id(), agent.id(), &current);
    DECODED_TASKS.with(|n| n.get())
}

/// Tasks decoded while a turn that already has its task ends, after `finished` earlier turns.
fn decoded_at_turn_end(finished: usize) -> usize {
    decoded_during(finished, |runtime, room, agent, current| {
        let store = &runtime.owned.durable_state_store;
        runtime
            .owned
            .ensure_agent_task(room, agent, current, None)
            .unwrap();
        let task = store
            .agent_turn_task(room, agent, "current")
            .unwrap()
            .unwrap();
        assert!(!store
            .agent_has_other_unfinished_task(room, agent, &task.task_id)
            .unwrap());
    })
}

/// Tasks decoded when a prompt is admitted again, after `finished` earlier turns.
fn decoded_at_admission(finished: usize) -> usize {
    decoded_during(finished, |runtime, room, _agent, current| {
        let created = runtime
            .owned
            .admit_agent_task(&crate::app::KernelPreparedPromptSubmission {
                session_id: room.into(),
                prompt: current.clone(),
                force_queue: false,
                refresh_projection: false,
            })
            .unwrap();
        assert!(!created, "the turn already had its task");
    })
}

#[test]
fn a_turn_end_reads_only_its_own_task() {
    let (few, many) = (decoded_at_turn_end(2), decoded_at_turn_end(30));
    assert_eq!(
        few, many,
        "a turn end decoded {few} tasks after 2 earlier turns and {many} after 30"
    );
}

#[test]
fn an_admission_reads_only_its_own_task() {
    let (few, many) = (decoded_at_admission(2), decoded_at_admission(30));
    assert_eq!(
        few, many,
        "an admission decoded {few} tasks after 2 earlier turns and {many} after 30"
    );
}

/// Tasks decoded by the legacy goal offer that every lifecycle sweep makes, after `finished` earlier turns.
fn decoded_at_legacy_goal_offer(finished: usize) -> usize {
    decoded_during(finished, |runtime, _room, _agent, _current| {
        runtime.offer_legacy_goal_adoption().unwrap();
    })
}

#[test]
fn a_room_without_legacy_runs_reads_no_tasks_for_the_goal_offer() {
    let (few, many) = (
        decoded_at_legacy_goal_offer(2),
        decoded_at_legacy_goal_offer(30),
    );
    assert_eq!(
        (few, many),
        (0, 0),
        "the goal offer decoded {few} tasks after 2 earlier turns and {many} after 30"
    );
}
