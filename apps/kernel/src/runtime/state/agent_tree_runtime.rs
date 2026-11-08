//! `ListDescendants`: the sub-agents below one agent, read from the room, and
//! the tree change event followers get when what that read returns differs.
use super::*;
use crate::agent::tree_rev;
use crate::durable_state::agent_lifecycle::ExecutionState;
use crate::local::{
    AgentTree, EventRefs, ListDescendantsRequest, LocalDaemonResponse, OutsideEventValue,
};
use crate::runtime::agent_tree_projection::{project_agent_tree, tree_for_viewer, NodeTurn};
use std::hash::{Hash, Hasher};

impl KernelRuntimeState {
    pub(crate) fn list_descendants(
        &self,
        caller_user_id: &str,
        request: ListDescendantsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = self.owned.session_store.get_session(&request.session_id)?;
        if !self.is_room_member(&session, caller_user_id) {
            return Err(DaemonError::SessionAccessDenied {
                session_id: request.session_id,
                user_id: caller_user_id.to_string(),
            });
        }
        let tasks = self
            .owned
            .durable_state_store
            .agent_tasks(Some(&request.session_id), None)?;
        let (tree, _) = self
            .read_agent_tree(&session, &request.agent_id, &tasks)
            .ok_or_else(|| DaemonError::AgentNotInSession {
                session_id: request.session_id.clone(),
                agent_id: request.agent_id.clone(),
            })?;
        let agents = self.owned.agent_store.get_session_agents(session.id());
        let tree = tree_for_viewer(tree, &session, &agents, caller_user_id);
        Ok(LocalDaemonResponse::AgentTree { tree })
    }

    /// The tree below `root` with its revision, and the fingerprint that revision stands for.
    fn read_agent_tree(
        &self,
        session: &crate::session::RuntimeSession,
        root: &str,
        tasks: &[crate::durable_state::agent_lifecycle::AgentTaskExecution],
    ) -> Option<(AgentTree, u64)> {
        let store = &self.owned.durable_state_store;
        let held_by_agent = self.owned.decisions_held_by_agents(session.id());
        let mut tree = project_agent_tree(
            session,
            &self.owned.agent_store.get_session_agents(session.id()),
            root,
            &|interaction_id| held_by_agent.contains(interaction_id),
            &|agent_id| {
                store
                    .agent_deadline(session.id(), agent_id)
                    .ok()
                    .flatten()
                    .map(|deadline| deadline.deadline_ms)
            },
            &|agent_id| {
                let task = tasks.iter().rev().find(|task| task.agent_id == agent_id)?;
                let finished =
                    matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled);
                Some(NodeTurn {
                    live: !finished && task.state != ExecutionState::Blocked,
                    blocked: task.state == ExecutionState::Blocked,
                    outcome: finished.then(|| self.owned.finished_task_outcome(task, None).kind),
                })
            },
        )?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&(&tree.totals, &tree.nodes))
            .unwrap_or_default()
            .hash(&mut hasher);
        let fingerprint = hasher.finish();
        tree.rev = tree_rev::observe(session.id(), root, fingerprint);
        Some((tree, fingerprint))
    }

    /// Tells followers of every tree that changed since the last pass to read it again.
    /// Run by the wake scheduler: a change is noticed whatever caused it, and several
    /// changes within one pass make one event.
    pub(crate) fn publish_changed_agent_trees(&self) {
        for session in self
            .owned
            .session_store
            .list_non_ended_sessions_including_hidden()
        {
            let session_id = session.id().to_owned();
            let agents = self.owned.agent_store.get_session_agents(&session_id);
            let parents = agents
                .iter()
                .filter_map(|agent| agent.parent_agent_id())
                .collect::<std::collections::HashSet<_>>();
            // A root that lost its last child still has a change to announce.
            let mut roots = tree_rev::seen_roots(&session_id);
            roots.extend(
                agents
                    .iter()
                    .filter(|agent| parents.contains(agent.id()))
                    .map(|agent| agent.id().to_owned()),
            );
            roots.sort();
            roots.dedup();
            if roots.is_empty() {
                continue;
            }
            let Ok(tasks) = self
                .owned
                .durable_state_store
                .agent_tasks(Some(&session_id), None)
            else {
                continue;
            };
            for root in roots {
                let Some((tree, fingerprint)) = self.read_agent_tree(&session, &root, &tasks)
                else {
                    continue;
                };
                if !tree_rev::claim_publication(&session_id, &root, fingerprint) {
                    continue;
                }
                if let Err(error) = self.owned.durable_state_store.outside_publish(
                    &session_id,
                    Some(&root),
                    OutsideEventValue::AgentTree {
                        root: root.clone(),
                        rev: tree.rev,
                    },
                    "agent tree changed",
                    EventRefs::default(),
                ) {
                    tracing::warn!(%error, %root, "A4: tree change event not published");
                }
            }
        }
        tree_rev::retain(|session, root| {
            self.owned
                .agent_store
                .get_session_agents(session)
                .iter()
                .any(|agent| agent.id() == root)
        });
    }
}
