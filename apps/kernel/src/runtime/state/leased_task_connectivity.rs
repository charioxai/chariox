//! MP-08 / MP-10 / MP-11 G03: advisory execution-kernel connectivity for
//! active home tasks, including yielded turns with no projection pump.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE: &str = "leased-task-connectivity";

impl KernelRuntimeState {
    pub(super) async fn check_leased_task_connections(
        &self,
        tasks: &[AgentTaskExecution],
    ) -> Result<(), DaemonError> {
        let workers: BTreeMap<_, _> = tasks
            .iter()
            .filter_map(|task| {
                active_binding(self, task).map(|binding| {
                    (
                        (binding.worker_kernel_id.clone(), binding.relay_url.clone()),
                        binding,
                    )
                })
            })
            .collect();
        let results = futures_util::future::join_all(workers.into_iter().map(|(key, binding)| async move {
            let reply = tokio::time::timeout(PROBE_TIMEOUT, async {
                let config = self.with_app_side_effect(move |app| app.relay_config_for_remote_execution(&binding)).await;
                let target = ClientTarget { daemon_id: Some(key.0.clone()), daemon_alias: None };
                let request = RelayPeerRequest::Ping { value: PROBE.into() };
                match self.connected_relay_state_for_config(&config).await {
                    Some(state) => crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                        &config, &state, target, request, PROBE_TIMEOUT,
                    ).await,
                    None => crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                        &config, target, request, PROBE_TIMEOUT,
                    ).await,
                }
            }).await;
            (key, matches!(reply, Ok(Ok(RelayPeerResponse::Pong { value, .. })) if value == PROBE))
        })).await.into_iter().collect::<BTreeMap<_, _>>();

        // Re-read after relay I/O: a task may finish or move while a probe is in flight.
        let current = self.owned.durable_state_store.agent_tasks(None, None)?;
        let active: Vec<_> = current
            .iter()
            .filter_map(|task| active_binding(self, task).map(|binding| (task, binding)))
            .collect();
        let keys: BTreeSet<_> = active
            .iter()
            .map(|(task, binding)| {
                (
                    task.task_id.clone(),
                    binding.worker_kernel_id.clone(),
                    binding.execution_lease_id.clone(),
                )
            })
            .collect();
        let mut uncertain = self
            .owned
            .uncertain_leased_tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        uncertain.retain(|key| keys.contains(key));
        for (task, binding) in active {
            let Some(connected) =
                results.get(&(binding.worker_kernel_id.clone(), binding.relay_url))
            else {
                continue;
            };
            let key = (
                task.task_id.clone(),
                binding.worker_kernel_id,
                binding.execution_lease_id,
            );
            let message = if *connected {
                uncertain
                    .remove(&key)
                    .then_some("Worker connection restored.")
            } else {
                uncertain.insert(key).then_some(
                    "Worker connection uncertain. The kernel is checking for reconnection.",
                )
            };
            if let Some(message) = message {
                // A connection probe cannot settle a task or authorize a replacement lease.
                self.owned.record_notice_for_agent(
                    &task.room_id,
                    task.provider_run_id.as_deref(),
                    Some(&task.agent_id),
                    self.owned
                        .attachment_store
                        .list_session_attachment_ids(&task.room_id),
                    format!("{}: {message}", self.agent_notice_name(&task.agent_id)),
                );
            }
        }
        Ok(())
    }
}

fn active_binding(
    state: &KernelRuntimeState,
    task: &AgentTaskExecution,
) -> Option<crate::agent::RemoteAgentBinding> {
    if !matches!(
        task.state,
        ExecutionState::Working | ExecutionState::Waiting
    ) {
        return None;
    }
    state
        .owned
        .agent_store
        .get_agent(&task.agent_id)
        .ok()?
        .remote_execution()
        .cloned()
}
