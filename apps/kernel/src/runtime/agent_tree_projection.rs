//! A4: the descendants of one agent as a client reads them (`ListDescendants`).
//!
//! A pending interaction of a sub-agent, or a turn blocked on its owner, leaves
//! that node `waiting`, and an interaction of the root itself is its panel's own
//! card and no node's. A node needs the user (`needs_user`) only while the user
//! is the current holder of one of its pending decisions (slice A3): a decision
//! an ancestor agent holds keeps the node waiting and shows no card. A turn
//! blocked without an interaction is `waiting` only.
//! A node is `running` while its turn lives (processing, or a task working or
//! parked on a wait or on its children); its `outcome` is how its latest turn
//! ended and is null while it runs or before it ever ran.
//! `totals.live` counts the nodes that are starting, running, waiting or unreachable: an idle node (finished,
//! or not started) is done to a client, which closes its Sub-agents view when none is live.
use crate::agent::AgentInstance;
use crate::local::{AgentTree, AgentTreeNode, AgentTreeState, AgentTreeTotals};
use crate::provider::TurnOutcomeKind;
use crate::session::{
    effective_agent_execution_config_in_tree, CollaborationLevel, RuntimeSession,
};

/// What the latest task of an agent says about its turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeTurn {
    pub(crate) live: bool,
    pub(crate) blocked: bool,
    pub(crate) outcome: Option<TurnOutcomeKind>,
}

fn node_state(agent: &AgentInstance, waiting: bool, turn: Option<NodeTurn>) -> AgentTreeState {
    if agent.archived() {
        AgentTreeState::Ended
    } else if agent.state() == crate::agent::AgentState::Error {
        AgentTreeState::Error
    } else if waiting || turn.is_some_and(|turn| turn.blocked) {
        AgentTreeState::Waiting
    } else if agent.is_processing() || turn.is_some_and(|turn| turn.live) {
        AgentTreeState::Running
    } else if agent.state() == crate::agent::AgentState::Working {
        AgentTreeState::Starting
    } else {
        AgentTreeState::Idle
    }
}

fn totals_of(nodes: &[AgentTreeNode]) -> AgentTreeTotals {
    AgentTreeTotals {
        live: nodes
            .iter()
            .filter(|node| {
                !matches!(
                    node.state,
                    AgentTreeState::Idle | AgentTreeState::Ended | AgentTreeState::Error
                )
            })
            .count() as u32,
        running: nodes
            .iter()
            .filter(|node| node.state == AgentTreeState::Running)
            .count() as u32,
        needs_user: nodes.iter().filter(|node| node.needs_user).count() as u32,
    }
}

fn redact_parameters(node: &mut AgentTreeNode) {
    node.provider = "redacted".to_owned();
    node.model = None;
    node.execution_mode = None;
    node.permission_level = None;
    node.effort = None;
    node.account_profile = None;
    node.workspace_id = None;
    node.worktree_id = None;
    node.worker_kernel_id = None;
}

/// What the viewer reads of `tree`, by the rule the session projection applies to agents: an agent
/// of the viewer is whole; another user's agent shows only to a collaborator who may view traces
/// and then without provider and model unless the collaborator may view parameters.
/// `member` is the identity the room stores for the viewer and `is_viewers` says whether an agent
/// owner is the viewer, so the enrolled owner of a pre-enrollment room reads as its `local` owner.
pub(crate) fn tree_for_viewer(
    mut tree: AgentTree,
    session: &RuntimeSession,
    agents: &[AgentInstance],
    member: &str,
    is_viewers: &dyn Fn(&str) -> bool,
) -> AgentTree {
    let level = session
        .collaboration_level_for_user(member)
        .unwrap_or(CollaborationLevel::Private);
    let viewers_agent = |agent_id: &str| {
        agents
            .iter()
            .find(|agent| agent.id() == agent_id)
            .is_some_and(|agent| is_viewers(agent.owner_user_id()))
    };
    tree.nodes.retain(|node| {
        agents
            .iter()
            .find(|agent| agent.id() == node.agent_id)
            .is_some_and(|agent| {
                is_viewers(agent.owner_user_id())
                    || session.can_view_tree_agent_owned_by(member, agent.owner_user_id())
            })
    });
    if !level.can_view_agent_parameters() {
        for node in &mut tree.nodes {
            if !viewers_agent(&node.agent_id) {
                redact_parameters(node);
            }
        }
    }
    tree.totals = totals_of(&tree.nodes);
    tree
}

/// `None` when `root` is not an agent of the session. Nodes are the descendants
/// of `root` (not `root` itself) in creation order. The revision is the caller's.
/// `held_by_agent` says whether an ancestor agent, not the user, holds an interaction.
pub(crate) fn project_agent_tree(
    session: &RuntimeSession,
    agents: &[AgentInstance],
    root: &str,
    held_by_agent: &dyn Fn(&str) -> bool,
    deadline_ms: &dyn Fn(&str) -> Option<u64>,
    turn_of: &dyn Fn(&str) -> Option<NodeTurn>,
) -> Option<AgentTree> {
    agents.iter().find(|agent| agent.id() == root)?;
    let pending = |agent_id: &str| {
        session
            .active_interactions()
            .iter()
            .any(|interaction| interaction.agent_id() == Some(agent_id))
    };
    let user_holds = |agent_id: &str| {
        session.active_interactions().iter().any(|interaction| {
            interaction.agent_id() == Some(agent_id) && !held_by_agent(interaction.id())
        })
    };
    // Membership and depth come from the parent edges, whatever the creation order: the user can
    // place an older agent under a newer descendant. Only the display order is by creation.
    let mut children = std::collections::HashMap::<&str, Vec<&AgentInstance>>::new();
    for agent in agents {
        if let Some(parent) = agent.parent_agent_id() {
            children.entry(parent).or_default().push(agent);
        }
    }
    let mut members = Vec::new();
    let mut seen = std::collections::HashSet::from([root]);
    let mut level = vec![(root, 0u32)];
    while !level.is_empty() {
        let mut next = Vec::new();
        for (parent, depth) in level {
            for child in children.get(parent).into_iter().flatten() {
                if seen.insert(child.id()) {
                    members.push((*child, depth + 1));
                    next.push((child.id(), depth + 1));
                }
            }
        }
        level = next;
    }
    members.sort_by(|(left, _), (right, _)| {
        (left.created_at_ms(), left.id()).cmp(&(right.created_at_ms(), right.id()))
    });
    let mut nodes = Vec::new();
    for (agent, depth) in members {
        let turn = turn_of(agent.id());
        let state = node_state(
            agent,
            pending(agent.id()) || (!agent.is_processing() && session.agent_is_waiting(agent.id())),
            turn,
        );
        // The value approval gating reads, after the limit of the parent chain.
        let effective = effective_agent_execution_config_in_tree(session, agent, agents);
        nodes.push(AgentTreeNode {
            agent_id: agent.id().to_owned(),
            parent_agent_id: agent.parent_agent_id().map(str::to_owned),
            kernel_id: None,
            alias: agent.alias().map(str::to_owned),
            provider: agent.provider().to_owned(),
            model: agent.model().map(str::to_owned),
            execution_mode: Some(effective.mode),
            permission_level: Some(effective.permission_level),
            effort: agent.effort().map(str::to_owned),
            account_profile: agent.account_profile().map(str::to_owned),
            // An agent with no place of its own works where its Room does.
            workspace_id: Some(
                agent
                    .workspace_id()
                    .unwrap_or_else(|| session.workspace_id())
                    .to_owned(),
            ),
            worktree_id: Some(
                agent
                    .worktree_id()
                    .unwrap_or_else(|| session.worktree_id())
                    .to_owned(),
            ),
            worker_kernel_id: agent
                .remote_execution()
                .map(|remote| remote.worker_kernel_id.clone()),
            depth,
            state,
            outcome: turn.and_then(|turn| turn.outcome),
            needs_user: user_holds(agent.id())
                && !matches!(state, AgentTreeState::Ended | AgentTreeState::Error),
            deadline_ms: deadline_ms(agent.id()),
            created_at_ms: agent.created_at_ms(),
        });
    }
    let totals = totals_of(&nodes);
    Some(AgentTree {
        root: root.to_owned(),
        rev: 0,
        totals,
        nodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentRef, GridPosition};
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};

    fn agent(id: &str, parent: Option<&str>, at: u64) -> AgentInstance {
        let mut agent = AgentInstance::new(
            id,
            id,
            "room",
            Some(id.to_owned()),
            "codex",
            None,
            None,
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        agent.set_parent_agent(parent.map(AgentRef::local));
        agent.set_created_at_ms_for_test(at);
        agent
    }

    fn session(agents: Vec<AgentInstance>) -> RuntimeSession {
        let mut session =
            RuntimeSession::new("room", None, "workspace", "worktree", "machine", "daemon");
        session.set_agents(agents);
        session
    }

    #[test]
    fn an_idle_sub_agent_with_an_armed_timer_is_waiting_in_the_tree() {
        let mut session = session(vec![
            agent("root", None, 1),
            agent("child", Some("root"), 2),
        ]);
        session.set_agent_wakes(vec![serde_json::from_value(serde_json::json!({
            "id": "wake", "task_id": "task", "room_id": "room", "agent_id": "child",
            "registration_id": "reg", "kind": "timer", "label": "tick", "state": "scheduled",
            "created_at_ms": 1, "verified_at_ms": 1, "next_due_ms": 9, "interval_ms": null,
            "command": [], "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
            "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null, "last_sequence": null,
            "last_delivery": null, "last_delivered_at_ms": null, "last_acknowledged_at_ms": null,
            "alerted_sequence": null,
        })).unwrap()]);
        let tree = project_agent_tree(
            &session,
            session.agents(),
            "root",
            &|_| false,
            &|_| None,
            &|_| None,
        )
        .unwrap();
        assert_eq!(tree.nodes[0].state, AgentTreeState::Waiting);
        assert_eq!(tree.totals.live, 1);
        assert_eq!(tree.totals.running, 0);
    }

    // MP-08/MP-09/MP-10/MP-11 A4: creation order, depth from the edge, no holder or card.
    #[test]
    fn nodes_follow_creation_order_with_their_depth() {
        let session = session(vec![
            agent("g", Some("c1"), 4),
            agent("root", None, 1),
            agent("c2", Some("root"), 3),
            agent("c1", Some("root"), 2),
            agent("other", None, 5),
        ]);
        let tree = project_agent_tree(
            &session,
            session.agents(),
            "root",
            &|_| false,
            &|_| None,
            &|_| None,
        )
        .unwrap();
        let shape = tree
            .nodes
            .iter()
            .map(|node| (node.agent_id.as_str(), node.depth))
            .collect::<Vec<_>>();
        assert_eq!(shape, vec![("c1", 1), ("c2", 1), ("g", 2)]);
        assert_eq!(tree.totals.live, 0, "idle nodes are not live");
        assert!(project_agent_tree(
            &session,
            session.agents(),
            "missing",
            &|_| false,
            &|_| None,
            &|_| None
        )
        .is_none());
    }

    fn shape(tree: &AgentTree) -> Vec<(&str, u32)> {
        tree.nodes
            .iter()
            .map(|node| (node.agent_id.as_str(), node.depth))
            .collect()
    }

    // Reviewer scenario: R and X are room agents, Y is spawned under R, then X is placed under Y.
    #[test]
    fn an_older_agent_moved_under_a_newer_descendant_stays_in_the_tree() {
        let session = session(vec![
            agent("r", None, 1),
            agent("x", Some("y"), 2),
            agent("y", Some("r"), 3),
            agent("z", Some("x"), 4),
        ]);
        let tree = project_agent_tree(
            &session,
            session.agents(),
            "r",
            &|_| false,
            &|_| None,
            &|_| None,
        )
        .unwrap();
        assert_eq!(shape(&tree), vec![("x", 2), ("y", 1), ("z", 3)]);
        let x_nodes = tree
            .nodes
            .iter()
            .filter(|node| node.agent_id == "x")
            .count();
        assert_eq!(x_nodes, 1, "listed once, as a descendant");
        assert!(session
            .agents()
            .iter()
            .find(|a| a.id() == "x")
            .unwrap()
            .is_sub_agent());
    }

    #[test]
    fn a_deep_chain_in_reverse_creation_order_keeps_every_depth() {
        let session = session(vec![
            agent("r", None, 1),
            agent("d4", Some("d3"), 2),
            agent("d3", Some("d2"), 3),
            agent("d2", Some("d1"), 4),
            agent("d1", Some("r"), 5),
        ]);
        let tree = project_agent_tree(
            &session,
            session.agents(),
            "r",
            &|_| false,
            &|_| None,
            &|_| None,
        )
        .unwrap();
        assert_eq!(
            shape(&tree),
            vec![("d4", 4), ("d3", 3), ("d2", 2), ("d1", 1)]
        );
    }

    #[test]
    fn a_parent_cycle_outside_the_root_never_loops_or_enters_the_tree() {
        let session = session(vec![
            agent("r", None, 1),
            agent("a", Some("b"), 2),
            agent("b", Some("a"), 3),
            agent("c", Some("r"), 4),
        ]);
        let tree = project_agent_tree(
            &session,
            session.agents(),
            "r",
            &|_| false,
            &|_| None,
            &|_| None,
        )
        .unwrap();
        assert_eq!(shape(&tree), vec![("c", 1)]);
    }

    fn ask(agent_id: &str) -> crate::session::RuntimeInteraction {
        use crate::session::{
            RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionKind,
            RuntimeInteractionLevel,
        };
        RuntimeInteraction::new(
            format!("ask-{agent_id}"),
            "room",
            RuntimeInteractionKind::Permission,
            RuntimeInteractionLevel::Warning,
            None,
            "Allow?",
            vec![RuntimeInteractionChoice::new("ok", "OK", "ok", None)],
            None,
            None,
            None,
        )
        .with_agent_id(agent_id)
    }

    // A node the user holds a decision of says so; the root's own decision is no node's.
    #[test]
    fn needs_user_marks_the_nodes_whose_decision_waits_for_the_user() {
        let mut session = session(vec![
            agent("root", None, 1),
            agent("c1", Some("root"), 2),
            agent("c2", Some("root"), 3),
        ]);
        session.add_active_interaction(ask("c1"));
        session.add_active_interaction(ask("root"));
        let tree = project_agent_tree(
            &session,
            session.agents(),
            "root",
            &|_| false,
            &|_| None,
            &|_| None,
        )
        .unwrap();
        let held = tree
            .nodes
            .iter()
            .map(|node| (node.agent_id.as_str(), node.needs_user, node.state))
            .collect::<Vec<_>>();
        assert_eq!(
            held,
            vec![
                ("c1", true, AgentTreeState::Waiting),
                ("c2", false, AgentTreeState::Idle)
            ]
        );
        assert_eq!(
            tree.totals.needs_user, 1,
            "the count of nodes with needs_user"
        );
    }

    // A decision an ancestor agent holds keeps the node waiting without asking the user.
    #[test]
    fn a_decision_held_by_an_agent_keeps_the_node_waiting_without_the_user() {
        let mut session = session(vec![
            agent("root", None, 1),
            agent("c1", Some("root"), 2),
            agent("c2", Some("root"), 3),
        ]);
        session.add_active_interaction(ask("c1"));
        session.add_active_interaction(ask("c2"));
        let tree = project_agent_tree(
            &session,
            session.agents(),
            "root",
            &|id| id == "ask-c1",
            &|_| None,
            &|_| None,
        )
        .unwrap();
        let held = tree
            .nodes
            .iter()
            .map(|node| (node.agent_id.as_str(), node.needs_user, node.state))
            .collect::<Vec<_>>();
        assert_eq!(
            held,
            vec![
                ("c1", false, AgentTreeState::Waiting),
                ("c2", true, AgentTreeState::Waiting)
            ]
        );
        assert_eq!(tree.totals.needs_user, 1);
    }

    #[test]
    fn archived_is_ended_and_leaves_the_live_total() {
        let mut ended = agent("c1", Some("root"), 2);
        ended.set_archived(true);
        let session = session(vec![
            agent("root", None, 1),
            ended,
            agent("c2", Some("root"), 3),
        ]);
        let tree = project_agent_tree(
            &session,
            session.agents(),
            "root",
            &|_| false,
            &|id| (id == "c2").then_some(99),
            &|_| None,
        )
        .unwrap();
        assert_eq!(tree.nodes[0].state, AgentTreeState::Ended);
        assert_eq!(tree.nodes[1].state, AgentTreeState::Idle);
        assert_eq!(tree.nodes[1].deadline_ms, Some(99));
        assert_eq!(
            tree.totals.live, 0,
            "an ended and an idle node are not live"
        );
    }

    // The client's live states are starting, running, waiting and unreachable: a child whose latest
    // turn finished is idle with its outcome and no longer counts, or the view stays open on done rows.
    #[test]
    fn a_child_whose_turn_finished_is_idle_and_not_live() {
        let session = session(vec![
            agent("root", None, 1),
            agent("done", Some("root"), 2),
            agent("cancelled", Some("root"), 3),
            agent("busy", Some("root"), 4),
        ]);
        let finished = |outcome| NodeTurn {
            live: false,
            blocked: false,
            outcome: Some(outcome),
        };
        let tree = project_agent_tree(
            &session,
            session.agents(),
            "root",
            &|_| false,
            &|_| None,
            &|id| match id {
                "done" => Some(finished(TurnOutcomeKind::Completed)),
                "cancelled" => Some(finished(TurnOutcomeKind::Cancelled)),
                _ => Some(NodeTurn {
                    live: true,
                    blocked: false,
                    outcome: None,
                }),
            },
        )
        .unwrap();
        let states = tree.nodes.iter().map(|node| node.state).collect::<Vec<_>>();
        assert_eq!(
            states,
            vec![
                AgentTreeState::Idle,
                AgentTreeState::Idle,
                AgentTreeState::Running
            ]
        );
        assert_eq!(tree.totals.live, 1);
    }

    fn settings_child(
        id: &str,
        parent: &str,
        at: u64,
        mode: Option<AgentExecutionMode>,
        level: Option<AgentPermissionLevel>,
        effort: Option<&str>,
    ) -> AgentInstance {
        let mut child = AgentInstance::new(
            id,
            id,
            "room",
            Some(id.to_owned()),
            "codex",
            None,
            effort.map(str::to_owned),
            None,
            GridPosition::new(0, 0, 1, 1),
        );
        child.set_parent_agent(Some(AgentRef::local(parent)));
        child.set_created_at_ms_for_test(at);
        child.set_execution_mode_override(mode);
        child.set_permission_level_override(level);
        child
    }

    fn settings_of(
        tree: &AgentTree,
    ) -> Vec<(&str, AgentExecutionMode, AgentPermissionLevel, Option<&str>)> {
        tree.nodes
            .iter()
            .map(|node| {
                (
                    node.agent_id.as_str(),
                    node.execution_mode.expect("the kernel reports the mode"),
                    node.permission_level
                        .expect("the kernel reports the permission level"),
                    node.effort.as_deref(),
                )
            })
            .collect()
    }

    fn read_tree(session: &RuntimeSession) -> AgentTree {
        project_agent_tree(
            session,
            session.agents(),
            "root",
            &|_| false,
            &|_| None,
            &|_| None,
        )
        .unwrap()
    }

    // A child shows the mode, permission level and variant it really runs with, not a default.
    #[test]
    fn a_node_reports_its_mode_permission_level_and_effort() {
        let session = session(vec![
            agent("root", None, 1),
            settings_child(
                "plan",
                "root",
                2,
                Some(AgentExecutionMode::Plan),
                Some(AgentPermissionLevel::Required),
                Some("high"),
            ),
            settings_child("plain", "root", 3, None, None, None),
        ]);
        let tree = read_tree(&session);
        assert_eq!(
            settings_of(&tree),
            vec![
                (
                    "plan",
                    AgentExecutionMode::Plan,
                    AgentPermissionLevel::Required,
                    Some("high")
                ),
                (
                    "plain",
                    AgentExecutionMode::Build,
                    AgentPermissionLevel::Yolo,
                    None
                ),
            ]
        );
        let wire = serde_json::to_value(&tree.nodes[1]).unwrap();
        assert!(
            wire.get("effort").is_none(),
            "no effort is no field: {wire}"
        );
        assert_eq!(wire["execution_mode"], "build");
        assert_eq!(wire["permission_level"], "yolo");
        let wire = serde_json::to_value(&tree.nodes[0]).unwrap();
        assert_eq!(
            (
                &wire["execution_mode"],
                &wire["permission_level"],
                &wire["effort"]
            ),
            (&"plan".into(), &"required".into(), &"high".into())
        );
    }

    // The value is the one approval gating uses: a child above its parent chain shows the cap.
    #[test]
    fn a_node_shows_the_level_its_parent_chain_caps_it_to() {
        let mut capped_root = agent("root", None, 1);
        capped_root.set_execution_mode_override(Some(AgentExecutionMode::Plan));
        capped_root.set_permission_level_override(Some(AgentPermissionLevel::Required));
        let session = session(vec![
            capped_root,
            settings_child(
                "raised",
                "root",
                2,
                Some(AgentExecutionMode::Build),
                Some(AgentPermissionLevel::Yolo),
                None,
            ),
            settings_child("unset", "root", 3, None, None, None),
            settings_child(
                "grand",
                "raised",
                4,
                Some(AgentExecutionMode::Build),
                Some(AgentPermissionLevel::Yolo),
                None,
            ),
        ]);
        let tree = read_tree(&session);
        for node in &tree.nodes {
            let agent = session
                .agents()
                .iter()
                .find(|agent| agent.id() == node.agent_id)
                .unwrap();
            let gated = crate::session::effective_agent_execution_config_in_tree(
                &session,
                agent,
                session.agents(),
            );
            assert_eq!(
                (node.execution_mode, node.permission_level),
                (Some(gated.mode), Some(gated.permission_level)),
                "{}",
                node.agent_id
            );
            assert_eq!(node.execution_mode, Some(AgentExecutionMode::Plan));
            assert_eq!(node.permission_level, Some(AgentPermissionLevel::Required));
        }
    }

    // A collaborator who may not view parameters reads no settings either.
    #[test]
    fn settings_follow_the_parameters_redaction() {
        let mut node_tree = read_tree(&session(vec![
            agent("root", None, 1),
            settings_child("c", "root", 2, None, None, None),
        ]));
        let node = &mut node_tree.nodes[0];
        node.execution_mode = Some(AgentExecutionMode::Plan);
        node.permission_level = Some(AgentPermissionLevel::Required);
        node.effort = Some("high".to_owned());
        redact_parameters(node);
        assert_eq!(
            (
                node.execution_mode,
                node.permission_level,
                node.effort.as_deref()
            ),
            (None, None, None)
        );
    }

    // The client controller tests read this file as the kernel's answer once every child finished.
    #[test]
    fn the_tree_of_finished_children_is_the_shared_client_fixture() {
        let session = session(vec![
            agent("room", None, 1),
            agent("done", Some("room"), 2),
            agent("cancelled", Some("room"), 3),
        ]);
        let tree = project_agent_tree(
            &session,
            session.agents(),
            "room",
            &|_| false,
            &|_| None,
            &|id| {
                Some(NodeTurn {
                    live: false,
                    blocked: false,
                    outcome: Some(if id == "done" {
                        TurnOutcomeKind::Completed
                    } else {
                        TurnOutcomeKind::Cancelled
                    }),
                })
            },
        )
        .unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../fixtures/agent-tree-finished.json"
        ))
        .unwrap();
        assert_eq!(serde_json::to_value(&tree).unwrap(), fixture);
    }
}
