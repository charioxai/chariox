//! A4: a sub-agent ends with its parent. Before any of a subtree is torn
//! down, one durable record marks all of it as deleting, so a crash at any
//! later step leaves work to finish, never a parent restored without its
//! children. A destroy walks the subtree depth-first at once; the scheduler's
//! pass, also the first thing a restarted kernel runs, finishes every marked
//! agent still there and every sub-agent whose parent's deletion is durable.
use super::*;
use crate::durable_state::agent_subtree_deletion::AGENT_SUBTREE_DELETING;

impl KernelRuntimeState {
    /// Direct children of `parent_id`, oldest first, so a walk is stable.
    pub(super) fn sub_agents_of(&self, parent_id: &str) -> Vec<crate::agent::AgentInstance> {
        let mut children = self
            .owned
            .agent_store
            .list_agents()
            .into_iter()
            .filter(|agent| agent.parent_agent_id() == Some(parent_id))
            .collect::<Vec<_>>();
        children.sort_by(|left, right| {
            (left.created_at_ms(), left.id()).cmp(&(right.created_at_ms(), right.id()))
        });
        children
    }

    /// Every descendant of `root_id` in depth-first order, `root_id` excluded.
    pub(super) fn descendants_depth_first(
        &self,
        root_id: &str,
    ) -> Vec<crate::agent::AgentInstance> {
        let mut found = Vec::new();
        let mut seen = std::collections::HashSet::from([root_id.to_owned()]);
        let mut stack = self.sub_agents_of(root_id);
        stack.reverse();
        while let Some(agent) = stack.pop() {
            if !seen.insert(agent.id().to_owned()) {
                continue;
            }
            let mut children = self.sub_agents_of(agent.id());
            children.reverse();
            stack.extend(children);
            found.push(agent);
        }
        found
    }

    /// Marks the agent and every descendant as deleting in one durable record.
    /// A destroy that fails after the mark is finished by the pass.
    pub(super) fn mark_subtree_deleting(
        &self,
        agent_id: &str,
        caller_user_id: &str,
    ) -> Result<(), DaemonError> {
        self.authorize_current_external_command()?;
        let root = self
            .owned
            .ensure_agent_owner(agent_id, caller_user_id, "destroy agent")?;
        let descendants = self.descendants_depth_first(root.id());
        if descendants.is_empty() {
            return Ok(());
        }
        let agent_ids = std::iter::once(root.id())
            .chain(descendants.iter().map(|agent| agent.id()))
            .collect::<Vec<_>>();
        self.owned.durable_state_store.append_event(
            AGENT_SUBTREE_DELETING,
            Some(root.id().to_owned()),
            serde_json::json!({ "session_id": root.session_id(), "agent_ids": agent_ids }),
        )?;
        Ok(())
    }

    /// Destroys the agent, then what is left below it depth-first.
    pub(super) async fn destroy_subtree(
        &self,
        agent_id: &str,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let destroyed = self
            .destroy_one_agent(agent_id, caller_user_id, false)
            .await?;
        self.destroy_sub_agents_of(destroyed.id()).await;
        Ok(destroyed)
    }

    /// Destroys what is left below an agent that is already gone. A failure
    /// leaves that child with a dangling edge for the next scheduler pass.
    pub(super) async fn destroy_sub_agents_of(&self, parent_id: &str) {
        let mut stack = self.sub_agents_of(parent_id);
        stack.reverse();
        while let Some(child) = stack.pop() {
            let owner = child.owner_user_id().to_owned();
            match self.destroy_one_agent(child.id(), &owner, true).await {
                Ok(_) => {
                    let mut grandchildren = self.sub_agents_of(child.id());
                    grandchildren.reverse();
                    stack.extend(grandchildren);
                }
                Err(error) => tracing::warn!(%error, agent_id = child.id(),
                    "A4: sub-agent cleanup retained for the next pass"),
            }
        }
    }

    /// Finishes what a crash or a failed cleanup cut short: every agent still
    /// marked as deleting, then every sub-agent whose parent's deletion is
    /// durable. A parent only gone from memory is still being torn down: its
    /// children wait for its record, or a crash would restore it without them.
    /// Run at startup and by the scheduler. Returns how many roots it ended.
    pub(crate) async fn end_orphaned_sub_agents(&self) -> usize {
        let marked = match self.owned.durable_state_store.agents_marked_deleting() {
            Ok(marked) => marked,
            Err(error) => {
                tracing::warn!(%error, "A4: deletion pass retained for the next run");
                return 0;
            }
        };
        let mut ended = 0;
        for agent_id in marked {
            // A marked root ends the rest of its subtree with it.
            if let Ok(agent) = self.owned.agent_store.get_agent(&agent_id) {
                ended += self.end_retained(&agent).await;
            }
        }
        let agents = self.owned.agent_store.list_agents();
        let known = agents
            .iter()
            .map(|agent| agent.id().to_owned())
            .collect::<std::collections::HashSet<_>>();
        for orphan in agents.iter().filter(|agent| {
            agent
                .parent_agent_id()
                .is_some_and(|parent| !known.contains(parent) && self.deletion_recorded(parent))
        }) {
            ended += self.end_retained(orphan).await;
        }
        ended
    }

    fn deletion_recorded(&self, agent_id: &str) -> bool {
        self.owned
            .durable_state_store
            .agent_deletion_recorded(agent_id)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, agent_id, "A4: orphan retained for the next pass");
                false
            })
    }

    async fn end_retained(&self, agent: &crate::agent::AgentInstance) -> usize {
        let owner = agent.owner_user_id().to_owned();
        match self.destroy_subtree(agent.id(), &owner).await {
            Ok(_) => 1,
            Err(error) => {
                tracing::warn!(%error, agent_id = agent.id(),
                    "A4: sub-agent cleanup retained for the next pass");
                0
            }
        }
    }
}
