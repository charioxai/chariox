use super::*;
use crate::agent::{AgentInstance, CreateAgentRequest};

impl KernelRuntimeState {
    pub(crate) async fn spawn_agents(
        &self,
        mut requests: Vec<CreateAgentRequest>,
        caller_user_id: &str,
        slice_refs: &[Option<String>],
    ) -> Result<Vec<AgentInstance>, DaemonError> {
        if requests.len() != slice_refs.len() {
            return Err(DaemonError::LocalTransport {
                operation: "agents.spawn",
                message: "slice admission target count mismatch".to_string(),
            });
        }
        for request in &mut requests {
            self.normalize_local_kernel_ref(request);
        }
        if requests.iter().all(|request| request.kernel_ref.is_none()) {
            let prepared = requests
                .into_iter()
                .map(|request| self.prepare_local_agent_worktree_placement(request))
                .collect::<Result<_, _>>()?;
            return self.owned.spawn_agents(prepared);
        }

        let mut ordered_agents = vec![None; requests.len()];
        let result = async {
            let mut local_requests = Vec::new();
            let mut local_indices = Vec::new();
            for (index, request) in requests.into_iter().enumerate() {
                if request.kernel_ref.is_none() {
                    local_requests.push(self.prepare_local_agent_worktree_placement(request)?);
                    local_indices.push(index);
                } else {
                    ordered_agents[index] = Some(self.spawn_agent(request).await?);
                }
            }
            if !local_requests.is_empty() {
                let local_agents = self.owned.spawn_agents(local_requests)?;
                for (index, agent) in local_indices.into_iter().zip(local_agents) {
                    ordered_agents[index] = Some(agent);
                }
            }
            let agents = ordered_agents
                .iter()
                .cloned()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "agents.spawn",
                    message: "batch spawn returned incomplete results".to_string(),
                })?;
            if let Some(last) = agents.last() {
                self.owned
                    .focus_agent(last.session_id(), last.id(), caller_user_id)?;
            }
            Ok(agents)
        }
        .await;
        match result {
            Ok(agents) => Ok(agents),
            Err(error) => Err(self
                .rollback_agent_batch(ordered_agents, slice_refs, caller_user_id, error)
                .await),
        }
    }

    async fn rollback_agent_batch(
        &self,
        agents: Vec<Option<AgentInstance>>,
        slice_refs: &[Option<String>],
        caller_user_id: &str,
        cause: DaemonError,
    ) -> DaemonError {
        let mut failures = Vec::new();
        let mut retained_attachments = Vec::new();
        // Reverse successful creation without forgetting unreachable workers.
        // The caller holds every admission guard through this recovery.
        for (agent, slice_ref) in agents.into_iter().zip(slice_refs).rev() {
            let Some(agent) = agent else { continue };
            if let Err(error) = self.destroy_agent(agent.id(), caller_user_id).await {
                failures.push(format!("{}: {error}", agent.id()));
                if let Some(slice_ref) = slice_ref {
                    retained_attachments.push(crate::slice::SliceAgentAttachment {
                        slice_ref: slice_ref.clone(),
                        session_id: agent.session_id().to_string(),
                        agent_id: agent.id().to_string(),
                    });
                }
            }
        }
        if let Err(error) = self.attach_slice_agents(retained_attachments).await {
            failures.push(format!("retained slice attachment: {error}"));
        }
        if failures.is_empty() {
            cause
        } else {
            DaemonError::LocalTransport {
                operation: "agents.spawn",
                message: format!("batch failed: {cause}; cleanup incomplete, retry cleanup for retained agents: {}", failures.join("; ")),
            }
        }
    }
}
