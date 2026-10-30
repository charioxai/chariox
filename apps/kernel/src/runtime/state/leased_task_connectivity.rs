//! MP-08 / MP-10 / MP-11 G03 / A10-S04: execution-kernel connectivity for
//! active home tasks, including yielded turns with no projection pump. A lost
//! connection is a notice; a worker the relay reports offline past the grace
//! period ends its turn as failed.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE: &str = "leased-task-connectivity";
/// A worker the relay reports offline for this long is lost: the sweep ends its
/// turns as failed instead of leaving the helper and whoever waits on it dormant.
const WORKER_LOSS_GRACE_MS: u64 = 20_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Probe {
    Connected,
    /// The relay itself says the worker's kernel is not connected.
    WorkerOffline,
    /// No answer, and no proof that the worker is the one that is gone.
    Unreachable,
}

impl KernelRuntimeState {
    pub(super) async fn check_leased_task_connections(
        &self,
        tasks: &[AgentTaskExecution],
        now: u64,
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
            let probe = match reply {
                Ok(Ok(RelayPeerResponse::Pong { value, .. })) if value == PROBE => Probe::Connected,
                Ok(Err(DaemonError::RelayTransport { code, .. }))
                    if matches!(code.as_str(), "target_not_connected" | "target_disconnected") =>
                {
                    Probe::WorkerOffline
                }
                _ => Probe::Unreachable,
            };
            (key, probe)
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
        let mut lost = Vec::new();
        {
            let mut uncertain = self
                .owned
                .uncertain_leased_tasks
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            uncertain.retain(|key, _| keys.contains(key));
            for (task, binding) in active {
                let Some(probe) =
                    results.get(&(binding.worker_kernel_id.clone(), binding.relay_url.clone()))
                else {
                    continue;
                };
                let key = (
                    task.task_id.clone(),
                    binding.worker_kernel_id.clone(),
                    binding.execution_lease_id.clone(),
                );
                let message = if *probe == Probe::Connected {
                    uncertain
                        .remove(&key)
                        .map(|_| "Worker connection restored.")
                } else {
                    let newly_uncertain = !uncertain.contains_key(&key);
                    let first_failed = *uncertain.entry(key.clone()).or_insert(now);
                    if *probe == Probe::WorkerOffline
                        && now.saturating_sub(first_failed) >= WORKER_LOSS_GRACE_MS
                    {
                        lost.push((task.clone(), binding, now.saturating_sub(first_failed)));
                    }
                    newly_uncertain.then_some(
                        "Worker connection uncertain. The kernel is checking for reconnection.",
                    )
                };
                if let Some(message) = message {
                    // A connection probe alone never settles a task; only the
                    // relay's own report that the worker is gone, held for the
                    // grace period, does.
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
        }
        for (task, binding, offline_ms) in lost {
            Box::pin(self.end_turn_on_lost_worker(&task, &binding, offline_ms)).await?;
        }
        Ok(())
    }

    /// The slice or worker kernel that hosts a leased agent, as users read it.
    pub(super) fn leased_host_name(&self, binding: &crate::agent::RemoteAgentBinding) -> String {
        match self
            .owned
            .slice_store
            .resolve_by_worker_kernel_ref(&binding.worker_kernel_id)
        {
            Some(slice) => format!("slice `{}`", slice.name),
            None => format!("worker kernel `{}`", binding.worker_kernel_id),
        }
    }

    /// The worker is gone and with it the provider run it hosted. The turn ends
    /// failed, never replayed, so the helper's task is resolved and its waiters wake.
    async fn end_turn_on_lost_worker(
        &self,
        task: &AgentTaskExecution,
        binding: &crate::agent::RemoteAgentBinding,
        offline_ms: u64,
    ) -> Result<(), DaemonError> {
        let Some(run) = binding.active_worker_provider_run_id.as_deref() else {
            return Ok(());
        };
        let session = self.owned.session_store.get_session(&task.room_id)?;
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &task.agent_id)
            .is_none_or(|prompt| prompt.id() != task.prompt_id)
        {
            return Ok(());
        }
        let host = self.leased_host_name(binding);
        let lost = format!(
            "The {host} hosting this turn has been offline for {} s",
            offline_ms / 1000
        );
        self.settle_lost_remote_prompt_run(
            &task.room_id,
            &task.agent_id,
            binding,
            run,
            &lost,
            "the relay reports the worker kernel is not connected",
        )
        .await
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
