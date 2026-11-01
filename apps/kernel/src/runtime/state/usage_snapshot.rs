use super::*;
use crate::local::{
    AgentContextUsage, AgentExecutionRoute, AgentUsageSnapshot, GetAgentUsageSnapshotRequest,
};

impl KernelRuntimeState {
    pub(crate) fn agent_usage_snapshot_response(
        &self,
        request: GetAgentUsageSnapshotRequest,
        remote_kernels: &[chariox_relay::protocol::RelayKernelPresence],
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let (session, agent) =
            self.session_agent_snapshot(&request.session_id, &request.agent_id)?;
        if agent.session_id() != session.id() {
            return Err(DaemonError::AgentNotInSession {
                session_id: request.session_id,
                agent_id: request.agent_id,
            });
        }
        // A usage read must not wait for an unrelated provider operation that holds the
        // service mutex; it then answers from the last published run projection.
        let run = self
            .owned
            .provider_store
            .try_get_agent_run_for_projection(session.id(), agent.id())
            .or_else(|| {
                self.owned
                    .provider_run_projection
                    .get_for_agent(session.id(), agent.id())
            });
        let context = run.as_ref().and_then(|run| {
            run.usage()
                .context_tokens
                .map(|used_tokens| AgentContextUsage {
                    used_tokens,
                    limit_tokens: run.usage().context_window.filter(|limit| *limit > 0),
                })
        });
        let config = self.owned.config_projection.snapshot();
        let home_name = config
            .daemon_alias
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| config.daemon_id.clone());
        let route = match agent.remote_execution() {
            Some(remote) => {
                let slice = self
                    .owned
                    .slice_store
                    .list_by_session(session.id())
                    .into_iter()
                    .find(|slice| slice.agent_ids.iter().any(|id| id == agent.id()))
                    .or_else(|| {
                        self.owned
                            .slice_store
                            .resolve_by_worker_kernel_ref(&remote.worker_kernel_id)
                    })
                    .or_else(|| {
                        self.owned
                            .slice_store
                            .resolve_by_worker_kernel_ref(&remote.worker_machine_id)
                    });
                let target = slice
                    .map(|slice| slice.name)
                    .or_else(|| {
                        remote_kernels
                            .iter()
                            .find(|kernel| {
                                kernel.kernel_id == remote.worker_kernel_id
                                    && kernel.machine_id == remote.worker_machine_id
                            })
                            .and_then(|kernel| kernel.kernel_alias.clone())
                    })
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| remote.worker_kernel_id.clone());
                AgentExecutionRoute {
                    target,
                    target_kernel_id: remote.worker_kernel_id.clone(),
                    via: Some(home_name),
                    via_kernel_id: Some(config.daemon_id.clone()),
                }
            }
            None => AgentExecutionRoute {
                target: home_name,
                target_kernel_id: config.daemon_id.clone(),
                via: None,
                via_kernel_id: None,
            },
        };
        Ok(LocalDaemonResponse::AgentUsageSnapshot {
            snapshot: AgentUsageSnapshot {
                session_id: request.session_id,
                agent_id: request.agent_id,
                context,
                route,
            },
        })
    }
}
