//! What a spawn answer states about a new agent beyond its record: read here once, so the
//! local answer and the room-command answer cannot disagree.
use super::*;
use crate::local::{GetAgentDeadlineRequest, LocalDaemonResponse};

pub(crate) struct SpawnedAgentFacts {
    /// The folder the agent works in: a remote agent's is its worker's, which only its binding
    /// names (None until it does); a local agent's is its own, else its room's.
    pub(crate) worktree: Option<String>,
    /// None when HEAD is detached, the folder is no git worktree, or it lies on a worker.
    pub(crate) branch: Option<String>,
    pub(crate) deadline_ms: Option<u64>,
}

impl KernelRuntimeState {
    pub(crate) async fn spawned_agent_facts(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<SpawnedAgentFacts, DaemonError> {
        let deadline_ms = match self.get_agent_deadline(GetAgentDeadlineRequest {
            session_id: agent.session_id().to_string(),
            agent_id: agent.id().to_string(),
        })? {
            LocalDaemonResponse::AgentDeadline { deadline } => {
                deadline.map(|deadline| deadline.deadline_ms)
            }
            _ => None,
        };
        let (worktree, branch) = match agent.remote_execution() {
            Some(binding) => (binding.worker_worktree_id.clone(), None),
            None => {
                let room = self.session_snapshot(agent.session_id()).await?;
                let worktree = agent.worktree_id().unwrap_or_else(|| room.worktree_id());
                (
                    Some(worktree.to_string()),
                    crate::git_worktree_placement::checked_out_branch(worktree),
                )
            }
        };
        Ok(SpawnedAgentFacts {
            worktree,
            branch,
            deadline_ms,
        })
    }
}
