//! A4: a sub-agent ends with its parent, and nothing is left hidden for ever.
use super::*;

struct Tree {
    runtime: KernelRuntimeState,
    room: String,
    ids: std::collections::BTreeMap<&'static str, String>,
}

/// r -> {c1 -> {g}, c2}, all in one room.
async fn tree(label: &str) -> Tree {
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
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
    Tree { runtime, room, ids }
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
