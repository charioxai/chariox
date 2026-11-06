//! A4: the descendants of one agent as a client reads them (`ListDescendants`).
//!
//! The decision holder is slice A3's; until it exists the rule is: a pending
//! interaction of a sub-agent, or a turn blocked on its owner, leaves that
//! node `waiting` (an ancestor or the user will answer it), and an interaction
//! of the root itself is its panel's own card and no node's. A node whose
//! interaction is pending is the user's to answer (`needs_user`): until the
//! holder exists nothing else could answer it. A turn blocked without an
//! interaction is `waiting` only.
//! A node is `running` while its turn lives (processing, or a task working or
//! parked on a wait or on its children); its `outcome` is how its latest turn
//! ended and is null while it runs or before it ever ran.
use crate::agent::AgentInstance;
use crate::local::{AgentTree, AgentTreeNode, AgentTreeState, AgentTreeTotals};
use crate::provider::TurnOutcomeKind;
use crate::session::RuntimeSession;

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

/// `None` when `root` is not an agent of the session. Nodes are the descendants
/// of `root` (not `root` itself) in creation order. The revision is the caller's.
pub(crate) fn project_agent_tree(
    session: &RuntimeSession,
    agents: &[AgentInstance],
    root: &str,
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
    let mut depth = std::collections::HashMap::from([(root.to_owned(), 0u32)]);
    let mut ordered = agents.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        (left.created_at_ms(), left.id()).cmp(&(right.created_at_ms(), right.id()))
    });
    // Parents are created before their children: one creation-order pass places every node.
    let mut nodes = Vec::new();
    for agent in ordered {
        let Some(parent) = agent.parent_agent_id() else {
            continue;
        };
        let Some(parent_depth) = depth.get(parent).copied() else {
            continue;
        };
        depth.insert(agent.id().to_owned(), parent_depth + 1);
        let turn = turn_of(agent.id());
        let state = node_state(agent, pending(agent.id()), turn);
        nodes.push(AgentTreeNode {
            agent_id: agent.id().to_owned(),
            parent_agent_id: Some(parent.to_owned()),
            kernel_id: None,
            alias: agent.alias().map(str::to_owned),
            provider: agent.provider().to_owned(),
            model: agent.model().map(str::to_owned),
            depth: parent_depth + 1,
            state,
            outcome: turn.and_then(|turn| turn.outcome),
            needs_user: pending(agent.id())
                && !matches!(state, AgentTreeState::Ended | AgentTreeState::Error),
            deadline_ms: deadline_ms(agent.id()),
            created_at_ms: agent.created_at_ms(),
        });
    }
    let totals = AgentTreeTotals {
        live: nodes
            .iter()
            .filter(|node| !matches!(node.state, AgentTreeState::Ended | AgentTreeState::Error))
            .count() as u32,
        running: nodes
            .iter()
            .filter(|node| node.state == AgentTreeState::Running)
            .count() as u32,
        needs_user: nodes.iter().filter(|node| node.needs_user).count() as u32,
    };
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
        let tree =
            project_agent_tree(&session, session.agents(), "root", &|_| None, &|_| None).unwrap();
        let shape = tree
            .nodes
            .iter()
            .map(|node| (node.agent_id.as_str(), node.depth))
            .collect::<Vec<_>>();
        assert_eq!(shape, vec![("c1", 1), ("c2", 1), ("g", 2)]);
        assert_eq!(tree.totals.live, 3);
        assert!(
            project_agent_tree(&session, session.agents(), "missing", &|_| None, &|_| None)
                .is_none()
        );
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
        let tree =
            project_agent_tree(&session, session.agents(), "root", &|_| None, &|_| None).unwrap();
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
            &|id| (id == "c2").then_some(99),
            &|_| None,
        )
        .unwrap();
        assert_eq!(tree.nodes[0].state, AgentTreeState::Ended);
        assert_eq!(tree.nodes[1].state, AgentTreeState::Idle);
        assert_eq!(tree.nodes[1].deadline_ms, Some(99));
        assert_eq!(tree.totals.live, 1);
    }
}
