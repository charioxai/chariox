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
        let agents = self.owned.agent_store.list_agents();
        let active_turns = self.active_turn_snapshot();
        let (tree, _) = self
            .read_agent_tree(&session, &request.agent_id, &tasks, &agents, &active_turns)
            .ok_or_else(|| DaemonError::AgentNotInSession {
                session_id: request.session_id.clone(),
                agent_id: request.agent_id.clone(),
            })?;
        let member = self.room_member_user_id_in(session.owner_user_id(), caller_user_id);
        let tree = tree_for_viewer(tree, &session, &agents, &member, &|agent_owner| {
            self.owned
                .acts_for_owner_in(session.owner_user_id(), caller_user_id, agent_owner)
        });
        Ok(LocalDaemonResponse::AgentTree { tree })
    }

    /// The tree below `root` with its revision, and the fingerprint that revision stands for.
    fn read_agent_tree(
        &self,
        session: &crate::session::RuntimeSession,
        root: &str,
        tasks: &[crate::durable_state::agent_lifecycle::AgentTaskExecution],
        agents: &[crate::agent::AgentInstance],
        active_turns: &std::collections::BTreeMap<String, crate::app::ActiveTurnState>,
    ) -> Option<(AgentTree, u64)> {
        let store = &self.owned.durable_state_store;
        let held_by_agent = self.owned.decisions_held_by_agents(session.id());
        let room_activity = std::cell::RefCell::new(std::collections::HashMap::<
            String,
            std::collections::BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity>,
        >::new());
        let mut tree = project_agent_tree(
            session,
            agents,
            root,
            &|room, interaction_id| {
                if room.id() == session.id() {
                    held_by_agent.contains(interaction_id)
                } else {
                    self.owned
                        .decisions_held_by_agents(room.id())
                        .contains(interaction_id)
                }
            },
            &|agent| {
                store
                    .agent_deadline(agent.session_id(), agent.id())
                    .ok()
                    .flatten()
                    .map(|deadline| (deadline.deadline_ms, deadline.state))
            },
            &|agent| {
                let active = active_turns.values().any(|turn| {
                    turn.session_id == agent.session_id() && turn.agent_id == agent.id()
                });
                let other_tasks;
                let tasks = if agent.session_id() == session.id() {
                    tasks
                } else {
                    other_tasks = store
                        .agent_tasks(Some(agent.session_id()), Some(agent.id()))
                        .unwrap_or_default();
                    &other_tasks
                };
                let task = tasks.iter().rev().find(|task| task.agent_id == agent.id());
                Some(NodeTurn {
                    active,
                    parked: task.is_some_and(|task| task.state == ExecutionState::Waiting),
                    blocked: task.is_some_and(|task| task.state == ExecutionState::Blocked),
                    outcome: task
                        .filter(|task| {
                            matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
                        })
                        .map(|task| self.owned.finished_task_outcome(task, None).kind),
                })
            },
            &|agent| {
                // One read of a Room's agent activity serves every node in it.
                let mut rooms = room_activity.borrow_mut();
                let activity = rooms
                    .entry(agent.session_id().to_owned())
                    .or_insert_with(|| {
                        // The stored Room has no agents of its own: sub-agents live in the agent store.
                        self.session_with_agents(agent.session_id())
                            .ok()
                            .map(|room| self.agent_activity_for_tree(&room))
                            .unwrap_or_default()
                    });
                activity
                    .get(agent.id())
                    .map(|activity| activity.liveness.clone())
            },
            &|room| self.owned.session_store.get_session(room).ok(),
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
        let agents = self.owned.agent_store.list_agents();
        let parents = agents
            .iter()
            .filter_map(|agent| agent.parent_agent_id())
            .collect::<std::collections::HashSet<_>>();
        let active_turns = self.active_turn_snapshot();
        for session in self
            .owned
            .session_store
            .list_non_ended_sessions_including_hidden()
        {
            let session_id = session.id().to_owned();
            // A root that lost its last child still has a change to announce.
            let mut roots = tree_rev::seen_roots(&session_id);
            roots.extend(
                agents
                    .iter()
                    .filter(|agent| {
                        agent.session_id() == session.id() && parents.contains(agent.id())
                    })
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
                let Some((tree, fingerprint)) =
                    self.read_agent_tree(&session, &root, &tasks, &agents, &active_turns)
                else {
                    continue;
                };
                if !tree_rev::claim_publication(&session_id, &root, fingerprint, tree.rev) {
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
                // The room's own clients hear of the change through their projection.
                self.owned
                    .session_projection
                    .record_session_change(&session_id);
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
