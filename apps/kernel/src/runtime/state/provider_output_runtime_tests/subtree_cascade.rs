//! A4: a sub-agent ends with its parent, and nothing is left hidden for ever.
use super::*;

struct Tree {
    runtime: KernelRuntimeState,
    room: String,
    ids: std::collections::BTreeMap<&'static str, String>,
    config: crate::config::DaemonConfig,
    _worktree: crate::test_support::TestWorktree,
}

/// r -> {c1 -> {g}, c2}, all in one room.
async fn tree(label: &str) -> Tree {
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, root) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    crate::test_support::admit_room_test_turn(&mut app, session.id(), root.id());
    let mut ids = std::collections::BTreeMap::from([("r", root.id().to_owned())]);
    for (name, parent) in [("c1", "r"), ("c2", "r"), ("g", "c1")] {
        // Spawning under an agent needs that agent's live turn.
        crate::test_support::admit_room_test_turn(&mut app, session.id(), &ids[parent]);
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias(name)
                    .with_parent_agent_id(&ids[parent]),
            )
            .unwrap();
        ids.insert(name, agent.id().to_owned());
    }
    let room = session.id().to_owned();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    Tree {
        runtime,
        room,
        ids,
        config,
        _worktree: worktree,
    }
}

/// SIGKILL and a new kernel on the same durable state.
async fn restart(tree: Tree) -> Tree {
    let Tree {
        runtime,
        room,
        ids,
        config,
        _worktree,
    } = tree;
    drop(runtime);
    let app = crate::test_support::bootstrap_after_test_owner_exit(config.clone()).await;
    let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
    Tree {
        runtime,
        room,
        ids,
        config,
        _worktree,
    }
}

fn alive(tree: &Tree, name: &str) -> bool {
    tree.runtime
        .owned
        .agent_store
        .get_agent(&tree.ids[name])
        .is_ok()
}

const USER: &str = crate::session::DEFAULT_LOCAL_USER_ID;

#[tokio::test]
async fn destroying_a_parent_destroys_every_descendant_depth_first() {
    let tree = tree("cascade-destroy").await;
    tree.runtime
        .destroy_agent(&tree.ids["r"], USER)
        .await
        .unwrap();
    for name in ["r", "c1", "c2", "g"] {
        assert!(!alive(&tree, name), "{name} outlived its parent");
    }
}

#[tokio::test]
async fn a_cascade_cut_short_is_completed_by_the_recovery_pass() {
    let tree = tree("cascade-recovery").await;
    // The crash happens after the parent is gone and before any child is.
    tree.runtime
        .destroy_one_agent(&tree.ids["r"], USER, false)
        .await
        .unwrap();
    for name in ["c1", "c2", "g"] {
        assert!(
            alive(&tree, name),
            "{name} is an orphan waiting for the pass"
        );
    }
    assert_eq!(tree.runtime.end_orphaned_sub_agents().await, 2, "c1 and c2");
    for name in ["c1", "c2", "g"] {
        assert!(!alive(&tree, name), "{name} survived the recovery pass");
    }
    assert_eq!(tree.runtime.end_orphaned_sub_agents().await, 0);
}

/// A destroy removes c1 from memory, then waits on its processes before its
/// own record is durable, and the scheduler's pass runs meanwhile. A crash in
/// that window must not bring c1 back without its sub-agent.
#[tokio::test]
async fn a_crash_while_a_parent_is_torn_down_leaves_no_half_deleted_subtree() {
    let tree = tree("cascade-crash-teardown").await;
    tree.runtime
        .owned
        .destroy_agent(&tree.ids["c1"], USER)
        .unwrap();
    tree.runtime.end_orphaned_sub_agents().await;
    let tree = restart(tree).await;
    tree.runtime.end_orphaned_sub_agents().await;
    assert_eq!(
        alive(&tree, "c1"),
        alive(&tree, "g"),
        "c1 and its sub-agent g must end together"
    );
}

#[tokio::test]
async fn the_recovery_pass_leaves_a_live_tree_alone() {
    let tree = tree("cascade-live").await;
    assert_eq!(tree.runtime.end_orphaned_sub_agents().await, 0);
    // A middle agent gone: only its own subtree ends.
    tree.runtime
        .destroy_one_agent(&tree.ids["c1"], USER, false)
        .await
        .unwrap();
    assert_eq!(tree.runtime.end_orphaned_sub_agents().await, 1);
    assert!(!alive(&tree, "g"));
    for name in ["r", "c2"] {
        assert!(
            alive(&tree, name),
            "{name} is not below the destroyed agent"
        );
    }
}

#[tokio::test]
async fn ending_the_room_ends_its_sub_agents() {
    let tree = tree("cascade-room-end").await;
    tree.runtime.end_session(&tree.room).await.unwrap();
    assert!(tree
        .runtime
        .owned
        .agent_store
        .get_session_agents(&tree.room)
        .is_empty());
}

/// An orphan's cleanup ends its processes, which can wait on the process
/// registry. The wake scheduler must not wait with it: other Rooms' timers
/// still fire on time.
// Other lifecycle work takes the registry synchronously, so the stuck cleanup
// needs worker threads of its own to be observable.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stuck_orphan_cleanup_does_not_hold_back_due_wakes() {
    use crate::durable_state::agent_lifecycle::{AgentWake, Operation};
    let tree = tree("cascade-stuck-sweep").await;
    // g's parent c1 is gone, so g is an orphan; r and c2 are live.
    tree.runtime
        .destroy_one_agent(&tree.ids["c1"], USER, false)
        .await
        .unwrap();
    let registry = crate::runtime::process_resources::hold_registry_for_test();
    let scheduler = {
        let runtime = tree.runtime.clone();
        tokio::spawn(async move { runtime.run_agent_wake_scheduler().await })
    };
    let store = &tree.runtime.owned.durable_state_store;
    let (room, agent) = (tree.room.as_str(), tree.ids["r"].as_str());
    let now = crate::session::unix_epoch_ms();
    store
        .agent_lifecycle(Operation::Begin {
            owner: "test-owner".into(),
            room: room.into(),
            agent: agent.into(),
            prompt: "timer-task".into(),
            run: Some("timer-run".into()),
            now,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::CreateWake {
            task: "timer-task".into(),
            prompt: "timer-task".into(),
            wake: AgentWake {
                id: "due-timer".into(),
                task_id: "timer-task".into(),
                room_id: room.into(),
                agent_id: agent.into(),
                registration_id: "completion-due-timer".into(),
                kind: "timer".into(),
                label: "due timer".into(),
                state: String::new(),
                created_at_ms: now,
                verified_at_ms: None,
                next_due_ms: Some(now + 100),
                interval_ms: None,
                command: vec![],
                match_text: None,
                matched_at_ms: None,
                pid: None,
                exit_code: None,
                summary: String::new(),
                detail: serde_json::Value::Null,
                fire_count: 0,
                missed_fires: 0,
                last_fired_at_ms: None,
                last_sequence: None,
                last_delivery: None,
                last_delivered_at_ms: None,
                last_acknowledged_at_ms: None,
                alerted_sequence: None,
            },
        })
        .unwrap();
    let fired = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let wakes = store.agent_wakes(Some(room), Some(agent)).unwrap();
            if wakes.iter().any(|wake| wake.fire_count == 1) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    drop(registry);
    // The released cleanup still ends the orphan, off the scheduler's loop.
    let ended = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while alive(&tree, "g") {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    scheduler.abort();
    let _ = scheduler.await;
    assert!(
        fired.is_ok(),
        "a due timer must fire while an orphan's cleanup waits"
    );
    assert!(
        ended.is_ok(),
        "the orphan is ended once its cleanup can run"
    );
}
