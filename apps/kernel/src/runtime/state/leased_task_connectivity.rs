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
            let slice_hosted = self
                .owned
                .slice_store
                .resolve_by_worker_kernel_ref(&key.0)
                .is_some();
            let probe = classify_probe(&reply, slice_hosted);
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
                let message = match observe(&mut uncertain, &key, *probe, now) {
                    Verdict::Restored => Some("Worker connection restored."),
                    Verdict::BecameUncertain => Some(
                        "Worker connection uncertain. The kernel is checking for reconnection.",
                    ),
                    Verdict::Lost { offline_ms } => {
                        lost.push((task.clone(), binding, offline_ms));
                        None
                    }
                    Verdict::Unchanged => None,
                };
                if let Some(message) = message {
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
        let host = self.owned.leased_host_name(binding);
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

type TaskKey = (String, String, String);

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Unchanged,
    BecameUncertain,
    Restored,
    /// The relay has reported the worker offline for the whole grace period.
    Lost { offline_ms: u64 },
}

/// Folds one probe into the task's connection state. A probe alone never
/// settles a task; only the relay's own report that the worker is gone, held
/// for the grace period, does.
fn observe(
    uncertain: &mut BTreeMap<TaskKey, u64>,
    key: &TaskKey,
    probe: Probe,
    now: u64,
) -> Verdict {
    if probe == Probe::Connected {
        return if uncertain.remove(key).is_some() {
            Verdict::Restored
        } else {
            Verdict::Unchanged
        };
    }
    let newly_uncertain = !uncertain.contains_key(key);
    let first_failed = *uncertain.entry(key.clone()).or_insert(now);
    let offline_ms = now.saturating_sub(first_failed);
    if probe == Probe::WorkerOffline && offline_ms >= WORKER_LOSS_GRACE_MS {
        Verdict::Lost { offline_ms }
    } else if newly_uncertain {
        Verdict::BecameUncertain
    } else {
        Verdict::Unchanged
    }
}

/// What a ping tells about the worker. A home-managed slice carries its own
/// relay, so there an unreachable relay is the slice that is gone; on a shared
/// relay only the relay's own report that the worker is not connected counts.
fn classify_probe(
    reply: &Result<Result<RelayPeerResponse, DaemonError>, tokio::time::error::Elapsed>,
    slice_hosted: bool,
) -> Probe {
    match reply {
        Ok(Ok(RelayPeerResponse::Pong { value, .. })) if value == PROBE => Probe::Connected,
        Ok(Err(DaemonError::RelayTransport { code, .. }))
            if matches!(
                code.as_str(),
                "target_not_connected" | "target_disconnected"
            ) =>
        {
            Probe::WorkerOffline
        }
        Ok(Err(DaemonError::LocalTransport { operation, message }))
            if slice_hosted
                && (matches!(
                    *operation,
                    "connect relay metadata socket" | "connect temporary relay peer socket"
                ) || (*operation == "send relay peer request"
                    && message == "relay is not connected")) =>
        {
            Probe::WorkerOffline
        }
        _ => Probe::Unreachable,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn refused_at(
        operation: &'static str,
    ) -> Result<Result<RelayPeerResponse, DaemonError>, tokio::time::error::Elapsed> {
        Ok(Err(DaemonError::LocalTransport {
            operation,
            message: "IO error: Connection refused (os error 111)".into(),
        }))
    }

    fn refused() -> Result<Result<RelayPeerResponse, DaemonError>, tokio::time::error::Elapsed> {
        refused_at("connect temporary relay peer socket")
    }

    #[test]
    fn a_slice_whose_own_relay_refuses_the_connection_is_offline() {
        assert!(classify_probe(&refused(), true) == Probe::WorkerOffline);
        // The first thing a ping to a stopped slice does is the relay metadata
        // discovery, which is where a stopped container is refused.
        assert!(
            classify_probe(&refused_at("connect relay metadata socket"), true)
                == Probe::WorkerOffline
        );
    }

    #[test]
    fn a_shared_relay_that_refuses_the_connection_says_nothing_about_the_worker() {
        assert!(classify_probe(&refused(), false) == Probe::Unreachable);
    }

    #[test]
    fn the_relay_reporting_the_worker_not_connected_is_offline_on_any_relay() {
        let reply = || {
            Ok(Err(DaemonError::RelayTransport {
                operation: "peer request",
                code: "target_not_connected".into(),
                message: "target daemon is not connected to relay".into(),
                retryable: true,
            }))
        };
        assert!(classify_probe(&reply(), false) == Probe::WorkerOffline);
        assert!(classify_probe(&reply(), true) == Probe::WorkerOffline);
    }
}
