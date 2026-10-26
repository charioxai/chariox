//! MP-08/MP-09/MP-10/MP-11 G06: forgetting a machine revokes its trust, so
//! every worker lease on it ends. The home kernel is the only authority: it
//! ends the sudo windows of those leased agents, stops their running turns,
//! tells the owner why, and refuses every later call through the lease.
use super::*;

/// Why a lease on a forgotten machine is refused, shown to the owner and to
/// the agent that tried to use it.
pub(crate) fn forgotten_machine_reason(machine_id: &str) -> String {
    format!(
        "machine `{machine_id}` was forgotten by the owner, so the worker lease on it has ended; \
         approve the machine again and rebind the agent to use it"
    )
}

/// Admission shared by every home-side use of a worker lease.
pub(crate) fn ensure_worker_machine_trusted(
    binding: &crate::agent::RemoteAgentBinding,
) -> Result<(), DaemonError> {
    if !crate::config::DaemonConfig::machine_forgotten(&binding.worker_machine_id) {
        return Ok(());
    }
    Err(DaemonError::RelayTransport {
        operation: "authorize leased worker",
        code: "unauthorized".into(),
        message: forgotten_machine_reason(&binding.worker_machine_id),
        retryable: false,
    })
}

impl KernelRuntimeState {
    /// Private-slice workers are absent from primary-relay discovery. An
    /// existing lease still gives the owner an exact machine identity to forget.
    pub(crate) fn resolve_leased_worker_machine_ref(&self, machine_ref: &str) -> Option<String> {
        self.owned
            .agent_store
            .list_agents()
            .into_iter()
            .find_map(|agent| {
                agent.remote_execution().and_then(|binding| {
                    (binding.worker_machine_id == machine_ref.trim())
                        .then(|| binding.worker_machine_id.clone())
                })
            })
    }

    /// Owner forgot `machine_id`: end the leases of every agent leased to it.
    pub(crate) async fn end_worker_leases_of_forgotten_machine(&self, machine_id: &str) {
        let reason = forgotten_machine_reason(machine_id);
        let leased = self
            .owned
            .agent_store
            .list_agents()
            .into_iter()
            .filter(|agent| {
                agent
                    .remote_execution()
                    .is_some_and(|remote| remote.worker_machine_id == machine_id)
            })
            .collect::<Vec<_>>();
        for agent in &leased {
            // The machine is already forgotten and every later call is
            // refused; a step that fails here is told to the owner, and the
            // other leases still end.
            if let Err(error) = self.end_worker_lease(agent, &reason).await {
                self.owned.record_notice_for_agent(
                    agent.session_id(),
                    None,
                    Some(agent.id()),
                    self.owned
                        .attachment_store
                        .list_session_attachment_ids(agent.session_id()),
                    format!(
                        "Agent `{}` is on forgotten machine `{machine_id}` but its lease could not be ended cleanly: {error}. Later calls through it are refused.",
                        agent.alias().unwrap_or_else(|| agent.id()),
                    ),
                );
            }
        }
    }

    async fn end_worker_lease(
        &self,
        agent: &crate::agent::AgentInstance,
        reason: &str,
    ) -> Result<(), DaemonError> {
        let Some(binding) = agent.remote_execution().cloned() else {
            return Ok(());
        };
        let windows = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .iter()
            .filter(|(_, turn)| turn.agent_id == agent.id())
            .map(|(entry, _)| entry.clone())
            .collect::<Vec<_>>();
        for entry in windows {
            self.revoke_sudo(None, Some(entry.as_str()), "machine_forgotten")?;
        }
        if let Some(run) = binding.active_worker_provider_run_id.as_deref() {
            self.end_accepted_remote_prompt_run(
                agent.session_id(),
                agent.id(),
                &binding,
                run,
                &format!("Stopped: {reason}"),
                "worker machine forgotten",
            )
            .await?;
        }
        self.owned.record_notice_for_agent(
            agent.session_id(),
            None,
            Some(agent.id()),
            self.owned
                .attachment_store
                .list_session_attachment_ids(agent.session_id()),
            format!(
                "Agent `{}` lost its worker lease: {reason}. Its sudo window ended and its running turn stopped.",
                agent.alias().unwrap_or_else(|| agent.id()),
            ),
        );
        self.release_worker_leases_in_background(vec![agent.clone()]);
        Ok(())
    }
}
