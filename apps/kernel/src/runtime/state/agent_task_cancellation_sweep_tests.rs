//! MP-08 / MP-09 / MP-10 / MP-11: a sweep re-cancels nothing for a cancelled task whose
//! resources are settled, so its writes do not grow with the cancelled tasks a room keeps.
use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation, Outcome};

/// Writer requests made by a second sweep over `cancelled` settled, cancelled turns.
async fn second_sweep_writes(cancelled: usize) -> u64 {
    let worktree = crate::test_support::TestWorktree::new("cancelled-sweep");
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
    for i in 0..cancelled {
        let prompt = format!("turn-{i}");
        store
            .agent_lifecycle(Operation::Begin {
                owner: room.owner_user_id().into(),
                room: room.id().into(),
                agent: agent.id().into(),
                prompt: prompt.clone(),
                run: None,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        let Outcome::Task(task) = store
            .agent_lifecycle(Operation::EndFailedTurn {
                task: prompt.clone(),
                prompt,
                reason: "provider run failed".into(),
            })
            .unwrap()
        else {
            panic!("a failed turn ends its task")
        };
        assert_eq!(task.state, ExecutionState::Cancelled);
    }
    runtime.sweep_agent_lifecycle().await.unwrap();
    for task in store.agent_tasks(Some(room.id()), None).unwrap() {
        assert!(store.agent_task_terminal_recorded(&task).unwrap());
    }
    let before = store.writer_requests_received();
    runtime.sweep_agent_lifecycle().await.unwrap();
    store.writer_requests_received() - before
}

#[tokio::test]
async fn a_sweep_leaves_settled_cancelled_tasks_alone() {
    let few = second_sweep_writes(2).await;
    let many = second_sweep_writes(30).await;
    assert!(
        many <= few + 2,
        "a second sweep made {few} writes for 2 cancelled turns and {many} for 30"
    );
}
