use super::*;
use crate::agent::{AgentInstance, CreateAgentRequest};

impl KernelRuntimeState {
    pub(crate) async fn spawn_agents(
        &self,
        mut requests: Vec<CreateAgentRequest>,
        caller_user_id: &str,
        slice_refs: &[Option<String>],
    ) -> Result<Vec<AgentInstance>, DaemonError> {
        self.authorize_current_external_command()?;
        if requests.len() != slice_refs.len() {
            return Err(DaemonError::InternalInvariant {
                operation: "agents.spawn",
                message: "slice admission target count mismatch".to_string(),
            });
        }
        for request in &mut requests {
            request.kernel_ref = self.settle_kernel_ref(request.kernel_ref.take()).await?;
        }
        // Checkouts made for agents that are then refused are removed with the refusal.
        let mut checkouts = Vec::new();
        if requests.iter().all(|request| request.kernel_ref.is_none()) {
            let mut prepared = Vec::new();
            for request in requests {
                match self.prepare_local_agent_worktree_placement(request) {
                    Ok((request, checkout)) => {
                        prepared.push(request);
                        checkouts.extend(checkout);
                    }
                    Err(error) => {
                        return Err(self.discard_unused_checkouts(checkouts, error));
                    }
                }
            }
            return self
                .owned
                .spawn_agents(prepared)
                .map_err(|error| self.discard_unused_checkouts(checkouts, error));
        }

        let mut ordered_agents = vec![None; requests.len()];
        let result = async {
            let mut local_requests = Vec::new();
            let mut local_indices = Vec::new();
            for (index, request) in requests.into_iter().enumerate() {
                if request.kernel_ref.is_none() {
                    let (request, checkout) =
                        self.prepare_local_agent_worktree_placement(request)?;
                    local_requests.push(request);
                    checkouts.extend(checkout);
                    local_indices.push(index);
                } else {
                    ordered_agents[index] = Some(self.spawn_agent(request).await?);
                }
            }
            if !local_requests.is_empty() {
                self.authorize_current_external_command()?;
                let local_agents = self.owned.spawn_agents(local_requests)?;
                for (index, agent) in local_indices.into_iter().zip(local_agents) {
                    ordered_agents[index] = Some(agent);
                }
            }
            let agents = ordered_agents
                .iter()
                .cloned()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| DaemonError::InternalInvariant {
                    operation: "agents.spawn",
                    message: "batch spawn returned incomplete results".to_string(),
                })?;
            // Worker-backed batches follow the same rule as local ones: a
            // metaagent's new agents never take the focus.
            if let Some(target) = crate::agent::new_agent_focus_target(&agents) {
                self.authorize_current_external_command()?;
                self.owned
                    .focus_agent(target.session_id(), target.id(), caller_user_id)?;
            }
            Ok(agents)
        }
        .await;
        match result {
            Ok(agents) => Ok(agents),
            Err(error) => {
                let error = self
                    .with_external_command_authority(None)
                    .rollback_agent_batch(ordered_agents, slice_refs, caller_user_id, error)
                    .await;
                Err(self.discard_unused_checkouts(checkouts, error))
            }
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
        if !retained_attachments.is_empty() {
            if let Err(error) = self.attach_slice_agents(retained_attachments).await {
                failures.push(format!("retained slice attachment: {error}"));
            }
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
