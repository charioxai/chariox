//! A4: a sub-agent ends with its parent. The parent is destroyed first, so
//! the edge left on its children (a parent id that no agent holds any more)
//! is the durable record that they still have to end. A destroy walks them
//! depth-first at once; the scheduler repeats the walk for whatever a crash
//! or a failed cleanup left behind, so no orphan stays hidden for ever.
use super::*;

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

    /// Ends every sub-agent whose parent no longer exists, with its subtree.
    /// Run at startup and by the scheduler. Returns how many roots it ended.
    pub(crate) async fn end_orphaned_sub_agents(&self) -> usize {
        let agents = self.owned.agent_store.list_agents();
        let known = agents
            .iter()
            .map(|agent| agent.id().to_owned())
            .collect::<std::collections::HashSet<_>>();
        let mut ended = 0;
        for orphan in agents.iter().filter(|agent| {
            agent
                .parent_agent_id()
                .is_some_and(|parent| !known.contains(parent))
        }) {
            let owner = orphan.owner_user_id().to_owned();
            match self.destroy_agent(orphan.id(), &owner).await {
                Ok(_) => ended += 1,
                Err(error) => tracing::warn!(%error, agent_id = orphan.id(),
                    "A4: orphaned sub-agent retained for the next pass"),
            }
        }
        ended
    }
}
