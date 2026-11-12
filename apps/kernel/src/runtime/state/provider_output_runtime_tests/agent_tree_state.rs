//! A4: the tree a client reads follows the sub-agents' turns: state, outcome and a
//! revision that moves exactly when the read differs, announced once to followers.
use super::child_finished::{family, team, team_with, Team};
use crate::durable_state::agent_lifecycle::outside::Mutation;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};
use crate::local::{
    AgentTree, AgentTreeState, ListDescendantsRequest, LocalDaemonResponse, OutsideEventValue,
};
use crate::provider::TurnOutcomeKind;

const USER: &str = crate::session::DEFAULT_LOCAL_USER_ID;

fn read(team: &Team) -> AgentTree {
    let request = ListDescendantsRequest {
        session_id: team.room.clone(),
        agent_id: team.parent.clone(),
    };
    match team.runtime.list_descendants(USER, request).unwrap() {
        LocalDaemonResponse::AgentTree { tree } => tree,
        other => panic!("agent tree expected, got {other:?}"),
    }
}

fn follow(team: &Team) {
    let store = &team.runtime.owned.durable_state_store;
    store
        .outside_mutate(Mutation::Subscribe {
            id: "tree-follow".into(),
            home: true,
            targets: vec![crate::local::EventTarget {
                kernel: "tree-test".into(),
                session_id: team.room.clone(),
                agent_id: None,
            }],
            baseline: vec![],
            open_decisions: vec![],
        })
        .unwrap();
    store
        .outside_mutate(Mutation::Identity {
            kernel: "tree-test".into(),
            machine: "tree-test".into(),
        })
        .unwrap();
}

fn tree_events(team: &Team) -> Vec<(String, u64)> {
    team.runtime
        .owned
        .durable_state_store
        .outside_list("tree-follow", None, 100)
        .unwrap()
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.value {
            OutsideEventValue::AgentTree { root, rev } => Some((root, rev)),
            _ => None,
        })
        .collect()
}

// MP-08/MP-09/MP-10/MP-11 A4: a working turn is running with no outcome; a finished one is idle with how it ended.
#[tokio::test]
async fn a_node_runs_without_an_outcome_and_finishes_with_one() {
    let working = team_with("tree-working", None, true).await;
    let node = &read(&working).nodes[0];
    assert_eq!(node.state, AgentTreeState::Running);
    assert!(node.outcome.is_none(), "{:?}", node.outcome);

    let done = team("tree-done", false, true).await;
    let node = &read(&done).nodes[0];
    assert_eq!(node.state, AgentTreeState::Idle);
    assert_eq!(node.outcome, Some(TurnOutcomeKind::Completed));

    let cancelled = team("tree-cancelled", true, true).await;
    let node = &read(&cancelled).nodes[0];
    assert_eq!(node.state, AgentTreeState::Idle);
    assert_eq!(node.outcome, Some(TurnOutcomeKind::Cancelled));
}

// A turn blocked on its owner is waiting; the user holds nothing of it on the node.
#[tokio::test]
async fn a_blocked_turn_is_waiting() {
    let team = team_with("tree-blocked", None, true).await;
    let store = &team.runtime.owned.durable_state_store;
    let Outcome::Task(blocked) = store
        .agent_lifecycle(Operation::Block {
            task: team.task.task_id.clone(),
            prompt: team.child_prompt.id().into(),
            reason: "needs the owner".into(),
        })
        .unwrap()
    else {
        panic!("blocked")
    };
    assert_eq!(
        blocked.state,
        crate::durable_state::agent_lifecycle::ExecutionState::Blocked
    );
    assert_eq!(read(&team).nodes[0].state, AgentTreeState::Waiting);
}

// The revision stays while the read is the same and grows when a turn finishes.
#[tokio::test]
async fn the_revision_moves_only_when_the_tree_differs() {
    let team = team_with("tree-rev", None, true).await;
    let first = read(&team);
    assert_eq!(read(&team).rev, first.rev, "nothing changed between reads");
    let Outcome::Settled { .. } = team
        .runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: team.child.clone(),
            prompt: team.child_prompt.id().into(),
            run: team.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 4,
        })
        .unwrap()
    else {
        panic!("settled")
    };
    let after = read(&team);
    assert!(after.rev > first.rev, "{} !> {}", after.rev, first.rev);
    assert_eq!(after.nodes[0].outcome, Some(TurnOutcomeKind::Completed));
    assert_eq!(read(&team).rev, after.rev);
}

// Followers hear once per change, with the revision the next read returns.
#[tokio::test]
async fn followers_hear_one_event_per_change() {
    let team = team_with("tree-event", None, true).await;
    follow(&team);
    team.runtime.publish_changed_agent_trees();
    let first = tree_events(&team);
    assert_eq!(first.len(), 1, "the new tree: {first:?}");
    team.runtime.publish_changed_agent_trees();
    assert_eq!(tree_events(&team).len(), 1, "no change, no event");
    team.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: team.child.clone(),
            prompt: team.child_prompt.id().into(),
            run: team.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 4,
        })
        .unwrap();
    team.runtime.publish_changed_agent_trees();
    let events = tree_events(&team);
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[1].0, team.parent);
    assert_eq!(events[1].1, read(&team).rev);
    assert!(events[1].1 > events[0].1);
}

// An older sub-agent the user places under a newer one stays in the tree, one level deeper, and
// never joins the room's own agents.
#[tokio::test]
async fn an_older_agent_placed_under_a_newer_sibling_is_listed_once_below_it() {
    let family = family("tree-reparent", 2, None, true).await;
    let (older, newer) = (&family.kids[0].child, &family.kids[1].child);
    let agents = family
        .runtime
        .owned
        .agent_store
        .get_session_agents(&family.room);
    let created = |id: &str| {
        agents
            .iter()
            .find(|a| a.id() == id)
            .unwrap()
            .created_at_ms()
    };
    // The reviewer scenario needs the moved agent to be the older one.
    let (older, newer) = if (created(older), older) <= (created(newer), newer) {
        (older, newer)
    } else {
        (newer, older)
    };
    family
        .runtime
        .set_agent_parent(
            USER,
            crate::local::SetAgentParentRequest {
                session_id: family.room.clone(),
                agent_id: older.clone(),
                parent_agent_id: Some(newer.clone()),
            },
        )
        .await
        .unwrap();
    let LocalDaemonResponse::AgentTree { tree } = family
        .runtime
        .list_descendants(
            USER,
            ListDescendantsRequest {
                session_id: family.room.clone(),
                agent_id: family.parent.clone(),
            },
        )
        .unwrap()
    else {
        panic!("agent tree expected")
    };
    let shape = tree
        .nodes
        .iter()
        .map(|node| (node.agent_id.clone(), node.depth))
        .collect::<Vec<_>>();
    assert!(shape.contains(&(newer.clone(), 1)), "{shape:?}");
    assert!(shape.contains(&(older.clone(), 2)), "{shape:?}");
    assert_eq!(shape.len(), 2, "each listed once: {shape:?}");
    let room_agents = family
        .runtime
        .owned
        .agent_store
        .get_session_agents(&family.room)
        .iter()
        .filter(|agent| !agent.is_sub_agent())
        .count();
    assert_eq!(room_agents, 1, "only the parent is a room agent");
}

// A client that reads the tree after a change must not use up the change for the followers:
// they hear of every change, whoever read it first.
#[tokio::test]
async fn a_client_read_does_not_swallow_the_event_followers_are_owed() {
    let team = team_with("tree-event-read", None, true).await;
    follow(&team);
    team.runtime.publish_changed_agent_trees();
    assert_eq!(tree_events(&team).len(), 1);
    team.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: team.child.clone(),
            prompt: team.child_prompt.id().into(),
            run: team.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 4,
        })
        .unwrap();
    // Another client refreshes its view before the scheduler's pass.
    let read_rev = read(&team).rev;
    team.runtime.publish_changed_agent_trees();
    let events = tree_events(&team);
    assert_eq!(events.len(), 2, "the follower was never told: {events:?}");
    assert_eq!(events[1].1, read_rev);
}

fn tree_changed_events(
    snapshot: &crate::runtime::projection::SessionSnapshotProjection,
    previous: &crate::runtime::projection::SessionSnapshotProjection,
) -> Vec<crate::transport::kernel_protocol::KernelEvent> {
    crate::transport::kernel_protocol::session_projection_changed_events(snapshot, Some(previous))
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                crate::transport::kernel_protocol::KernelEvent::AgentTreeChanged { .. }
            )
        })
        .collect()
}

// The room's own clients (not only external followers) hear of a tree change, once.
#[tokio::test]
async fn the_rooms_clients_hear_a_tree_change_through_their_projection() {
    use crate::transport::kernel_protocol::KernelEvent;
    let team = team_with("tree-room-event", None, true).await;
    let quiet = team
        .runtime
        .session_snapshot_projection(&team.room, 0)
        .unwrap();
    assert!(
        quiet.agent_tree_revs.is_empty(),
        "{:?}",
        quiet.agent_tree_revs
    );
    let sequence = team
        .runtime
        .session_projection_session_change_sequence(&team.room);
    team.runtime.publish_changed_agent_trees();
    assert_ne!(
        team.runtime
            .session_projection_session_change_sequence(&team.room),
        sequence,
        "the room's subscriptions are woken"
    );
    let told = team
        .runtime
        .session_snapshot_projection(&team.room, 0)
        .unwrap();
    let rev = read(&team).rev;
    assert_eq!(told.agent_tree_revs.get(&team.parent), Some(&rev));
    let events = tree_changed_events(&told, &quiet);
    assert!(
        matches!(events.as_slice(), [KernelEvent::AgentTreeChanged { session_id, root, rev: sent }]
            if *session_id == team.room && *root == team.parent && *sent == rev),
        "{events:?}"
    );
    // The same tree again says nothing, to followers or to the room.
    let sequence = team
        .runtime
        .session_projection_session_change_sequence(&team.room);
    team.runtime.publish_changed_agent_trees();
    assert_eq!(
        team.runtime
            .session_projection_session_change_sequence(&team.room),
        sequence
    );
    // A change moves the rev the event carries to the one external followers get.
    team.runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: team.child.clone(),
            prompt: team.child_prompt.id().into(),
            run: team.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 4,
        })
        .unwrap();
    team.runtime.publish_changed_agent_trees();
    let after = team
        .runtime
        .session_snapshot_projection(&team.room, 0)
        .unwrap();
    let events = tree_changed_events(&after, &told);
    assert!(
        matches!(events.as_slice(), [KernelEvent::AgentTreeChanged { rev: sent, .. }]
            if *sent == read(&team).rev && *sent != rev),
        "{events:?}"
    );
}

// `agent_tree_changed` and `ListDescendants` follow one rule: a member hears of a change to a tree
// exactly when the listing shows them its agents. A full collaborator reads every node of the owner's
// tree and hears of its change; a private member reads none of them and hears nothing.
#[tokio::test]
async fn a_tree_change_reaches_exactly_the_members_whose_listing_shows_the_tree() {
    use crate::session::CollaborationLevel;
    let team = team_with("tree-private", None, true).await;
    {
        let mut sessions = team.runtime.owned.session_store.write();
        for (invite_id, user_id, level) in [
            ("invite-full", "user-full", CollaborationLevel::Full),
            (
                "invite-private",
                "user-private",
                CollaborationLevel::Private,
            ),
        ] {
            let (_, invite) = sessions
                .create_session_invite(
                    &team.room,
                    invite_id.into(),
                    USER.into(),
                    None,
                    Some(1),
                    level,
                )
                .unwrap();
            sessions
                .join_session_invite(&team.room, invite.invite_id(), user_id.into(), 1)
                .unwrap();
        }
    }
    let history = &team.runtime.owned.durable_state_store;
    let seen_by = |user: &str| {
        team.runtime
            .session_snapshot_projection(&team.room, 0)
            .unwrap()
            .redacted_for_user(user, history)
            .unwrap()
    };
    let before = [
        ("user-full", seen_by("user-full")),
        ("user-private", seen_by("user-private")),
    ];
    team.runtime.publish_changed_agent_trees();
    let whole = read(&team);
    assert!(!whole.nodes.is_empty());
    for (user, quiet) in before {
        let listed = match team
            .runtime
            .list_descendants(
                user,
                ListDescendantsRequest {
                    session_id: team.room.clone(),
                    agent_id: team.parent.clone(),
                },
            )
            .unwrap()
        {
            LocalDaemonResponse::AgentTree { tree } => tree,
            other => panic!("agent tree expected, got {other:?}"),
        };
        let heard: Vec<String> = tree_changed_events(&seen_by(user), &quiet)
            .into_iter()
            .filter_map(|event| match event {
                crate::transport::kernel_protocol::KernelEvent::AgentTreeChanged {
                    root, ..
                } => Some(root),
                _ => None,
            })
            .collect();
        let shown = !listed.nodes.is_empty();
        assert_eq!(
            heard == vec![team.parent.clone()],
            shown,
            "{user}: heard {heard:?}, listed {:?}",
            listed.nodes
        );
        assert_eq!(heard.is_empty(), !shown, "{user}: heard {heard:?}");
        match user {
            "user-full" => assert_eq!(listed.nodes.len(), whole.nodes.len()),
            _ => assert!(heard.is_empty() && listed.nodes.is_empty(), "{user}"),
        }
    }
}

// A client reads the settings a child really runs with through `ListDescendants`, and a change
// of the child's settings moves the revision followers are told about.
#[tokio::test]
async fn a_node_reports_the_settings_it_runs_with_and_a_change_moves_the_revision() {
    let team = team_with("tree-settings", None, true).await;
    let before = read(&team);
    assert_eq!(
        (
            before.nodes[0].execution_mode,
            before.nodes[0].permission_level
        ),
        (
            Some(crate::provider::AgentExecutionMode::Build),
            Some(crate::provider::AgentPermissionLevel::Yolo)
        )
    );
    team.runtime
        .owned
        .agent_store
        .update_agent_config(
            &team.child,
            Some(Some(crate::provider::AgentExecutionMode::Plan)),
            Some(Some(crate::provider::AgentPermissionLevel::Required)),
            None,
            None,
        )
        .unwrap();
    let after = read(&team);
    assert_eq!(
        (
            after.nodes[0].execution_mode,
            after.nodes[0].permission_level
        ),
        (
            Some(crate::provider::AgentExecutionMode::Plan),
            Some(crate::provider::AgentPermissionLevel::Required)
        )
    );
    assert!(after.rev > before.rev, "{} > {}", after.rev, before.rev);
}

// Landing 9 sub-panes: a list row shows what the prompt footer shows, so a node also reports the
// account, workspace, worktree and the machine its agent runs on; a change moves the revision.
#[tokio::test]
async fn a_node_reports_its_account_workspace_worktree_and_machine() {
    let team = team_with("tree-footer", None, true).await;
    let before = read(&team);
    let node = &before.nodes[0];
    // A child with no place of its own works where its Room does.
    let room = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    assert_eq!(
        (
            &node.account_profile,
            node.workspace_id.as_deref(),
            node.worktree_id.as_deref(),
            &node.worker_kernel_id
        ),
        (
            &None,
            Some(room.workspace_id()),
            Some(room.worktree_id()),
            &None
        )
    );
    let agents = &team.runtime.owned.agent_store;
    let child = agents.get_agent(&team.child).unwrap();
    agents
        .set_agent_runtime_profile_with_account_profile(
            &team.child,
            child.provider(),
            child.model().map(str::to_owned),
            child.effort().map(str::to_owned),
            Some("work".into()),
            child.provider_resume_state().clone(),
        )
        .unwrap();
    agents
        .update_agent_config(
            &team.child,
            None,
            None,
            Some(Some("workspace-a".into())),
            Some(Some("tree-a".into())),
        )
        .unwrap();
    agents
        .bind_remote_execution(
            &team.child,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".into(),
                worker_machine_id: "worker-machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased".into(),
                active_worker_provider_run_id: None,
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: None,
                worker_worktree_id: None,
            },
        )
        .unwrap();
    let after = read(&team);
    let node = &after.nodes[0];
    assert_eq!(node.account_profile.as_deref(), Some("work"));
    assert_eq!(node.workspace_id.as_deref(), Some("workspace-a"));
    assert_eq!(node.worktree_id.as_deref(), Some("tree-a"));
    assert_eq!(node.worker_kernel_id.as_deref(), Some("worker-kernel"));
    assert!(after.rev > before.rev);
}
