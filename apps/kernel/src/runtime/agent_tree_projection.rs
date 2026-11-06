//! A4: the descendants of one agent as a client reads them (`ListDescendants`).
//!
//! The decision holder is slice A3's; until it exists the rule is: a pending
//! interaction of a sub-agent leaves that node `waiting` (an ancestor or the
//! user will answer it), and one of the root itself, which the user alone
//! holds, counts as `needs_user`.
use crate::agent::AgentInstance;
use crate::local::{AgentTree, AgentTreeNode, AgentTreeState, AgentTreeTotals};
use crate::session::RuntimeSession;

fn node_state(agent: &AgentInstance, waiting: bool) -> AgentTreeState {
    if agent.archived() {
        AgentTreeState::Ended
    } else if agent.state() == crate::agent::AgentState::Error {
        AgentTreeState::Error
    } else if waiting {
        AgentTreeState::Waiting
    } else if agent.is_processing() {
        AgentTreeState::Running
    } else if agent.state() == crate::agent::AgentState::Working {
        AgentTreeState::Starting
    } else {
        AgentTreeState::Idle
    }
}

/// `None` when `root` is not an agent of the session. Nodes are the descendants
/// of `root` (not `root` itself) in creation order.
pub(crate) fn project_agent_tree(
    session: &RuntimeSession,
    agents: &[AgentInstance],
    root: &str,
    rev: u64,
    deadline_ms: &dyn Fn(&str) -> Option<u64>,
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
        nodes.push(AgentTreeNode {
            agent_id: agent.id().to_owned(),
            parent_agent_id: Some(parent.to_owned()),
            kernel_id: None,
            alias: agent.alias().map(str::to_owned),
            provider: agent.provider().to_owned(),
            model: agent.model().map(str::to_owned),
            depth: parent_depth + 1,
            state: node_state(agent, pending(agent.id())),
            outcome: None,
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
        needs_user: u32::from(pending(root)),
    };
    Some(AgentTree {
        root: root.to_owned(),
        rev,
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
        let tree = project_agent_tree(&session, session.agents(), "root", 7, &|_| None).unwrap();
        let shape = tree
            .nodes
            .iter()
            .map(|node| (node.agent_id.as_str(), node.depth))
            .collect::<Vec<_>>();
        assert_eq!(shape, vec![("c1", 1), ("c2", 1), ("g", 2)]);
        assert_eq!(tree.rev, 7);
        assert_eq!(tree.totals.live, 3);
        assert!(project_agent_tree(&session, session.agents(), "missing", 7, &|_| None).is_none());
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
        let tree = project_agent_tree(&session, session.agents(), "root", 1, &|id| {
            (id == "c2").then_some(99)
        })
        .unwrap();
        assert_eq!(tree.nodes[0].state, AgentTreeState::Ended);
        assert_eq!(tree.nodes[1].state, AgentTreeState::Idle);
        assert_eq!(tree.nodes[1].deadline_ms, Some(99));
        assert_eq!(tree.totals.live, 1);
    }
}
