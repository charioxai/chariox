// MP-08 / MP-09 / MP-10 / MP-11: ListDescendants follows the room's collaboration levels, like the
// session projection: a Private collaborator sees nothing of another user's sub-agents, a
// Transparent one sees them without provider and model, a Full one and the owner see everything.
use super::*;
use crate::local::{AgentTree, ListDescendantsRequest};

struct Room {
    router: CommandRouter,
    session_id: String,
    root: String,
    child: String,
    grandchild: String,
}

fn invite(
    app: &mut DaemonApp,
    session_id: &str,
    user: &str,
    level: crate::session::CollaborationLevel,
) {
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            session_id,
            format!("invite-{user}"),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            level,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(session_id, invite.invite_id(), user.to_string(), 1)
        .expect("user should join session");
}

fn room(label: &str) -> (Room, crate::test_support::TestWorktree) {
    use crate::session::CollaborationLevel::{Full, Private, Transparent};
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(worktree.session_request())
        .expect("session should be created");
    let session_id = session.id().to_string();
    invite(&mut app, &session_id, "private-user", Private);
    invite(&mut app, &session_id, "transparent-user", Transparent);
    invite(&mut app, &session_id, "full-user", Full);
    let mut spawn = |alias: &str, parent: Option<&str>| {
        let mut request = CreateAgentRequest::new(&session_id, "dev-stub")
            .with_alias(alias)
            .with_model("secret-model");
        if let Some(parent) = parent {
            request = request.with_parent_agent_id(parent);
        }
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(request)
            .expect("agent should spawn")
            .id()
            .to_string()
    };
    let root = spawn("tree-root", None);
    let child = spawn("tree-child", Some(&root));
    let grandchild = spawn("tree-grandchild", Some(&child));
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    (
        Room {
            router,
            session_id,
            root,
            child,
            grandchild,
        },
        worktree,
    )
}

/// The node ids, sorted: siblings created in the same millisecond have no fixed order.
fn sorted_ids(tree: &AgentTree) -> Vec<&str> {
    let mut ids = tree
        .nodes
        .iter()
        .map(|node| node.agent_id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

fn both(room: &Room) -> Vec<&str> {
    let mut ids = vec![room.child.as_str(), room.grandchild.as_str()];
    ids.sort_unstable();
    ids
}

async fn descendants(room: &Room, user: &str) -> AgentTree {
    let request = LocalDaemonRequest::ListDescendants(ListDescendantsRequest {
        session_id: room.session_id.clone(),
        agent_id: room.root.clone(),
    });
    match room
        .router
        .dispatch(remote_command_for_request(&request, Some(user)), request)
        .await
        .expect("a room member reads the tree")
    {
        LocalDaemonResponse::AgentTree { tree } => tree,
        other => panic!("agent tree expected, got {other:?}"),
    }
}

#[test]
fn a_private_collaborator_reads_nothing_of_another_users_subtree() {
    run_remote_authorization_large_stack_test("tree-private", private_collaborator);
}

async fn private_collaborator() {
    let (room, _worktree) = room("tree-private");
    let tree = descendants(&room, "private-user").await;
    assert_eq!(tree.root, room.root);
    assert!(
        tree.nodes.is_empty(),
        "another user's sub-agents stay hidden: {:?}",
        tree.nodes
    );
    assert_eq!(tree.totals.live, 0);
    assert_eq!(tree.totals.running, 0);
    assert_eq!(tree.totals.needs_user, 0);
}

#[test]
fn a_transparent_collaborator_reads_the_tree_without_provider_and_model() {
    run_remote_authorization_large_stack_test("tree-transparent", transparent_collaborator);
}

async fn transparent_collaborator() {
    let (room, _worktree) = room("tree-transparent");
    let tree = descendants(&room, "transparent-user").await;
    assert_eq!(sorted_ids(&tree), both(&room));
    for node in &tree.nodes {
        assert_eq!(node.provider, "redacted");
        assert_eq!(node.model, None);
    }
}

#[test]
fn the_owner_and_a_full_collaborator_read_the_whole_tree() {
    run_remote_authorization_large_stack_test("tree-full", owner_and_full_collaborator);
}

async fn owner_and_full_collaborator() {
    let (room, _worktree) = room("tree-full");
    for user in [DEFAULT_LOCAL_USER_ID, "full-user"] {
        let tree = descendants(&room, user).await;
        assert_eq!(sorted_ids(&tree), both(&room), "{user}");
        for node in &tree.nodes {
            assert_eq!(node.provider, "dev-stub", "{user}");
            assert_eq!(node.model.as_deref(), Some("secret-model"), "{user}");
        }
        let grandchild = tree
            .nodes
            .iter()
            .find(|node| node.agent_id == room.grandchild);
        assert_eq!(
            grandchild.and_then(|node| node.parent_agent_id.as_deref()),
            Some(room.child.as_str()),
            "{user}"
        );
    }
}
