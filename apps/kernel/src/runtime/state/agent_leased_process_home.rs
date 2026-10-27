//! MP-08/MP-09/MP-10/MP-11 A10: home-owned watcher admission and source freshness.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentWake, LeasedProcessBinding, Operation, Outcome,
};
use crate::transport::relay_peer::{
    LeasedWatchedProcessAction as Action, RelayPeerRequest, RelayPeerResponse,
};

// MP-08/MP-09/MP-10/MP-11: reserve supervision before publishing the wake.
// A scheduler Query must not race the worker's first durable Start receipt.
struct LeasedProcessPoll {
    monitor: Arc<super::agent_wake_scheduler::AgentWakeMonitor>,
    id: String,
}

impl LeasedProcessPoll {
    fn reserve(
        monitor: &Arc<super::agent_wake_scheduler::AgentWakeMonitor>,
        id: &str,
    ) -> Option<Self> {
        monitor
            .leased_polls
            .lock()
            .expect("leased process polls poisoned")
            .insert(id.to_owned())
            .then(|| Self {
                monitor: monitor.clone(),
                id: id.to_owned(),
            })
    }
}

impl Drop for LeasedProcessPoll {
    fn drop(&mut self) {
        self.monitor
            .leased_polls
            .lock()
            .expect("leased process polls poisoned")
            .remove(&self.id);
    }
}

/// How the agent's current lease relates to the lease a wake was launched under.
/// The same signed worker identity on a new lease is the same worker that came back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LeaseRelation {
    Current,
    ReplacedSameWorker,
    OtherWorker,
}

impl LeaseRelation {
    fn of(remote: &crate::agent::RemoteAgentBinding, binding: &LeasedProcessBinding) -> Self {
        if remote.worker_kernel_id != binding.worker_kernel_id
            || remote.worker_machine_id != binding.worker_machine_id
        {
            Self::OtherWorker
        } else if remote.execution_lease_id == binding.execution_lease_id
            && remote.leased_agent_id == binding.leased_agent_id
        {
            Self::Current
        } else {
            Self::ReplacedSameWorker
        }
    }
}

impl KernelRuntimeState {
    async fn leased_process_request(
        &self,
        wake: &AgentWake,
        binding: &LeasedProcessBinding,
        action: Action,
    ) -> Result<Option<ledger::LeasedProcessReceipt>, DaemonError> {
        let agent = self.owned.agent_store.get_agent(&wake.agent_id)?;
        let remote = agent
            .remote_execution()
            .ok_or_else(|| ledger::error("watched process execution lease was lost"))?;
        let relation = LeaseRelation::of(remote, binding);
        if agent.session_id() != wake.room_id || relation == LeaseRelation::OtherWorker {
            return Err(ledger::error(
                "watched process execution lease was replaced",
            ));
        }
        // The same authenticated worker retains the exact old launch receipt.
        // Queries and cancellation may reconcile it; Start still needs a current lease.
        if relation == LeaseRelation::ReplacedSameWorker && matches!(action, Action::Start { .. }) {
            return Err(ledger::error(
                "watched process execution lease was replaced",
            ));
        }
        let mut config = self.config_snapshot().await;
        if let (Some(url), Some(token)) = (remote.relay_url.clone(), remote.relay_token.clone()) {
            config.apply_remote_relay_override(url, token);
        }
        let timeout = if matches!(action, Action::Start { .. }) {
            Duration::from_secs(310)
        } else {
            Duration::from_secs(5)
        };
        let response = tokio::time::timeout(
            timeout,
            crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                &config,
                chariox_relay::protocol::ClientTarget {
                    daemon_id: Some(binding.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                RelayPeerRequest::LeasedWatchedProcess {
                    leased_agent_id: binding.leased_agent_id.clone(),
                    action,
                },
                timeout,
            ),
        )
        .await
        .map_err(|_| ledger::error("leased process receipt query timed out"))??;
        let RelayPeerResponse::LeasedWatchedProcess { receipt } = response else {
            return Err(ledger::error("unexpected leased process receipt response"));
        };
        if let Some(receipt) = &receipt {
            if receipt.binding != *binding
                || receipt.wake.id != wake.id
                || receipt.wake.task_id != wake.task_id
                || receipt.wake.room_id != wake.room_id
                || receipt.wake.agent_id != wake.agent_id
                || receipt.wake.command != wake.command
                || receipt.wake.match_text != wake.match_text
                || receipt.wake.registration_id != wake.registration_id
            {
                return Err(ledger::error(
                    "leased process receipt does not match its admitted source",
                ));
            }
        }
        Ok(receipt)
    }

    pub(super) async fn start_home_leased_process(
        &self,
        wake: AgentWake,
        cwd: Option<String>,
        task: &str,
        prompt: &str,
    ) -> Result<AgentWake, DaemonError> {
        let target = self.owned.agent_store.get_agent(&wake.agent_id)?;
        let remote = target
            .remote_execution()
            .ok_or_else(|| ledger::error("leased watcher target missing"))?;
        if remote.relay_peer_protocol_version
            != Some(crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION)
        {
            return Err(ledger::error(format!(
                "leased watched processes require relay peer protocol {}; upgrade the worker",
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION
            )));
        }
        let binding = LeasedProcessBinding {
            worker_kernel_id: remote.worker_kernel_id.clone(),
            worker_machine_id: remote.worker_machine_id.clone(),
            execution_lease_id: remote.execution_lease_id.clone(),
            leased_agent_id: remote.leased_agent_id.clone(),
            home_prompt_id: prompt.into(),
            worker_provider_run_id: remote
                .active_worker_provider_run_id
                .clone()
                .ok_or_else(|| ledger::error("leased process needs an active worker run"))?,
        };
        self.authorize_current_external_command()?;
        let admission = LeasedProcessPoll::reserve(&self.owned.agent_wakes, &wake.id)
            .ok_or_else(|| ledger::error("leased process admission is already supervised"))?;
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::CreateLeasedWake {
                task: task.into(),
                prompt: prompt.into(),
                wake: wake.clone(),
                binding: binding.clone(),
            })?;
        let result = self
            .leased_process_request(
                &wake,
                &binding,
                Action::Start {
                    wake: wake.clone(),
                    binding: binding.clone(),
                    cwd,
                },
            )
            .await;
        // The worker answered with a refusal. Without a receipt, nothing ran.
        let result = match result {
            Err(refusal @ DaemonError::RelayTransport { .. }) => {
                let receipt = self
                    .leased_process_request(
                        &wake,
                        &binding,
                        Action::Query {
                            wake_id: wake.id.clone(),
                            binding: binding.clone(),
                        },
                    )
                    .await;
                if matches!(receipt, Ok(None)) {
                    drop(admission);
                    self.settle_failed_process_launch(
                        &wake,
                        format!(
                            "launch refused: {}",
                            crate::secret_redaction::redact_secrets(&refusal.to_string())
                        ),
                    );
                    return Err(refusal);
                }
                Err(refusal)
            }
            other => other,
        };
        // Lost transport never retries Start. Poll the original identity only.
        if let Ok(Some(receipt)) = &result {
            self.project_leased_process_receipt(&wake, receipt)?;
        }
        drop(admission);
        self.ensure_leased_process_poll(&wake);
        self.refresh_wake_projection(&wake.room_id);
        self.owned
            .durable_state_store
            .agent_wakes(Some(&wake.room_id), Some(&wake.agent_id))?
            .into_iter()
            .find(|w| w.id == wake.id)
            .ok_or_else(|| ledger::error("leased process wake missing"))
    }

    fn project_leased_process_receipt(
        &self,
        wake: &AgentWake,
        receipt: &ledger::LeasedProcessReceipt,
    ) -> Result<(), DaemonError> {
        let store = &self.owned.durable_state_store;
        let current = store
            .agent_wakes(Some(&wake.room_id), Some(&wake.agent_id))?
            .into_iter()
            .find(|current| current.id == wake.id)
            .ok_or_else(|| ledger::error("leased process source disappeared"))?;
        let wake = &current;
        if matches!(wake.state.as_str(), "exited" | "lost" | "cancelled") {
            return Ok(());
        }
        let now = crate::session::unix_epoch_ms();
        if wake.state == "starting" && receipt.wake.pid.is_some() {
            if receipt.process_start_ticks.is_none() || receipt.worker_boot_id.is_empty() {
                return Err(ledger::error(
                    "worker process start identity is unconfirmed",
                ));
            }
            store.agent_lifecycle(Operation::ProcessStarted {
                id: wake.id.clone(),
                pid: receipt.wake.pid.unwrap(),
                now,
            })?;
        }
        if wake.matched_at_ms.is_none() {
            if let Some(line) = &receipt.matched_line {
                let protect = super::agent_process_output::room_protector(
                    self.owned.room_secret_observations.clone(),
                    wake.room_id.clone(),
                );
                if let Some(line) = protect(super::agent_process_output::sanitize(line.as_bytes()))
                {
                    let outcome = store.agent_lifecycle(Operation::ProcessMatched {
                        id: wake.id.clone(),
                        line,
                        now,
                    })?;
                    // Same as a local watcher: the agent is woken by the match
                    // while the process keeps running.
                    if let Outcome::Wakes(matched) = outcome {
                        for matched in &matched {
                            self.wake_notice(
                                matched,
                                format!("Watched process '{}' printed its match", matched.label),
                            );
                            self.schedule_wake_delivery(matched);
                        }
                    }
                }
            }
        }
        if matches!(receipt.wake.state.as_str(), "exited" | "lost" | "cancelled") {
            let protect = super::agent_process_output::room_protector(
                self.owned.room_secret_observations.clone(),
                wake.room_id.clone(),
            );
            let tail = protect(super::agent_process_output::sanitize(
                receipt.output_tail.as_bytes(),
            ))
            .unwrap_or_else(|| "[output protected]".into());
            store.agent_lifecycle(Operation::ProcessExited {
                id: wake.id.clone(),
                exit_code: receipt.wake.exit_code,
                tail,
                now,
            })?;
            self.schedule_wake_delivery(wake);
        }
        self.refresh_wake_projection(&wake.room_id);
        Ok(())
    }

    pub(super) fn ensure_leased_process_poll(&self, wake: &AgentWake) {
        let Some(poll) = LeasedProcessPoll::reserve(&self.owned.agent_wakes, &wake.id) else {
            return;
        };
        let state = self.clone();
        let wake = wake.clone();
        tokio::spawn(async move {
            if let Err(error) = state.poll_leased_process(&wake).await {
                tracing::warn!(%error,"MP-08/MP-09/MP-10/MP-11: leased watcher supervision retained for next tick");
            }
            drop(poll);
        });
    }

    async fn poll_leased_process(&self, initial: &AgentWake) -> Result<(), DaemonError> {
        let store = &self.owned.durable_state_store;
        let binding = store
            .leased_wake_binding(&initial.id)?
            .ok_or_else(|| ledger::error("leased watcher binding missing"))?;
        let mut last_fresh = Instant::now();
        loop {
            let Some(wake) = store
                .agent_wakes(Some(&initial.room_id), Some(&initial.agent_id))?
                .into_iter()
                .find(|w| w.id == initial.id)
            else {
                return Ok(());
            };
            if !matches!(wake.state.as_str(), "starting" | "running" | "cancelling") {
                return Ok(());
            }
            let action = if wake.state == "cancelling" {
                Action::Cancel {
                    wake_id: wake.id.clone(),
                    binding: binding.clone(),
                }
            } else {
                Action::Query {
                    wake_id: wake.id.clone(),
                    binding: binding.clone(),
                }
            };
            match self.leased_process_request(&wake, &binding, action).await {
                Ok(Some(receipt)) => {
                    self.project_leased_process_receipt(&wake, &receipt)?;
                    last_fresh = Instant::now();
                }
                Ok(None) => {
                    self.lost_leased_process(
                        &wake,
                        "worker has no durable receipt for the original launch",
                    )?;
                    return Ok(());
                }
                Err(error) => {
                    let agent = self.owned.agent_store.get_agent(&wake.agent_id)?;
                    let current = agent.remote_execution().is_some_and(|r| {
                        LeaseRelation::of(r, &binding) != LeaseRelation::OtherWorker
                    });
                    if !current || last_fresh.elapsed() >= Duration::from_secs(120) {
                        self.lost_leased_process(
                            &wake,
                            &crate::secret_redaction::redact_secrets(&error.to_string()),
                        )?;
                        return Ok(());
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    fn lost_leased_process(&self, wake: &AgentWake, reason: &str) -> Result<(), DaemonError> {
        self.owned.durable_state_store.agent_lifecycle(Operation::ProcessExited{id:wake.id.clone(),exit_code:None,tail:format!("process_lost: {reason}; original launch is not replayed; physical cleanup may require the worker"),now:crate::session::unix_epoch_ms()})?;
        self.wake_notice(
            wake,
            format!(
                "Wake alert: leased process '{}' lost its source; re-evaluate without relaunching",
                wake.label
            ),
        );
        self.schedule_wake_delivery(wake);
        Ok(())
    }

    pub(super) async fn cancel_agent_watched_process(&self, id: &str) -> Result<bool, DaemonError> {
        if self
            .owned
            .durable_state_store
            .leased_wake_binding(id)?
            .is_some()
        {
            if let Some(wake) = self
                .owned
                .durable_state_store
                .agent_wakes(None, None)?
                .into_iter()
                .find(|w| w.id == id)
            {
                self.ensure_leased_process_poll(&wake);
            }
            Ok(false)
        } else {
            Ok(self.owned.agent_wakes.processes.terminate(id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_returned_worker_is_the_same_worker_only_under_its_signed_identity() {
        let binding = LeasedProcessBinding {
            worker_kernel_id: "worker".into(),
            worker_machine_id: "machine".into(),
            execution_lease_id: "lease-1".into(),
            leased_agent_id: "leased-1".into(),
            home_prompt_id: "prompt".into(),
            worker_provider_run_id: "run".into(),
        };
        let mut remote = crate::agent::RemoteAgentBinding {
            worker_kernel_id: "worker".into(),
            worker_machine_id: "machine".into(),
            execution_lease_id: "lease-1".into(),
            leased_agent_id: "leased-1".into(),
            active_worker_provider_run_id: None,
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: None,
        };
        assert_eq!(LeaseRelation::of(&remote, &binding), LeaseRelation::Current);
        remote.execution_lease_id = "lease-2".into();
        remote.leased_agent_id = "leased-2".into();
        assert_eq!(
            LeaseRelation::of(&remote, &binding),
            LeaseRelation::ReplacedSameWorker
        );
        remote.worker_kernel_id = "another-worker".into();
        assert_eq!(
            LeaseRelation::of(&remote, &binding),
            LeaseRelation::OtherWorker
        );
        remote.worker_kernel_id = "worker".into();
        remote.worker_machine_id = "another-machine".into();
        assert_eq!(
            LeaseRelation::of(&remote, &binding),
            LeaseRelation::OtherWorker
        );
    }

    #[tokio::test]
    async fn leased_process_admission_excludes_poll_until_commit_and_releases_on_abort() {
        let monitor = Arc::new(super::super::agent_wake_scheduler::AgentWakeMonitor::new());
        let (ready, admitted) = tokio::sync::oneshot::channel();
        let task_monitor = monitor.clone();
        let admission = tokio::spawn(async move {
            let _reservation = LeasedProcessPoll::reserve(&task_monitor, "pending-start").unwrap();
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        admitted.await.unwrap();
        assert!(LeasedProcessPoll::reserve(&monitor, "pending-start").is_none());
        // Independent sources still admit while this Start is in flight.
        assert!(LeasedProcessPoll::reserve(&monitor, "another-source").is_some());
        admission.abort();
        assert!(admission.await.unwrap_err().is_cancelled());
        let poll = LeasedProcessPoll::reserve(&monitor, "pending-start").unwrap();
        assert!(LeasedProcessPoll::reserve(&monitor, "pending-start").is_none());
        drop(poll);
        assert!(LeasedProcessPoll::reserve(&monitor, "pending-start").is_some());
    }
}
