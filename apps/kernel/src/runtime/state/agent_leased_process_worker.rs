//! MP-08/MP-09/MP-10/MP-11 A10: lease-admitted worker process execution.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, LeasedProcessMutation, LeasedProcessReceipt, Operation, Outcome,
};
use crate::transport::relay_peer::LeasedWatchedProcessAction;
use sha2::{Digest, Sha256};

pub(super) fn process_digest(
    wake: &ledger::AgentWake,
    cwd: &Option<String>,
) -> Result<String, DaemonError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&wake.command, &wake.match_text, cwd))
                .map_err(|_| ledger::error("process command encoding failed"))?
        )
    ))
}

impl KernelRuntimeState {
    // MP-08/MP-10/MP-11: restart cleanup does not depend on a surviving lease.
    pub(super) fn recover_worker_leased_processes(&self, now: u64) -> Result<(), DaemonError> {
        let receipts: Vec<_> = self
            .owned
            .durable_state_store
            .leased_process_receipts()?
            .into_iter()
            .filter(|r| {
                matches!(r.wake.state.as_str(), "starting" | "running" | "cancelling")
                    && r.wake.created_at_ms < self.owned.agent_wakes.started_at_ms
                    && !self.owned.agent_wakes.processes.contains(&r.wake.id)
            })
            .collect();
        if receipts.is_empty() {
            return Ok(());
        }
        let launches = receipts
            .iter()
            .map(|r| {
                (
                    r.wake.id.clone(),
                    r.process_start_ticks
                        .and_then(super::agent_process_group::start_time_ms)
                        .unwrap_or(r.wake.created_at_ms),
                )
            })
            .collect();
        let stopped = super::agent_process_group::reap_orphans(&launches);
        for receipt in receipts {
            let cleanup = if stopped.is_ok() {
                "attributable orphan exits confirmed"
            } else {
                "physical cleanup unconfirmed; operator reconciliation required"
            };
            self.apply_agent_process_receipt(Operation::ProcessExited {
                id: receipt.wake.id,
                exit_code: None,
                tail: format!(
                    "process_lost: worker restarted; original launch is never replayed; {cleanup}"
                ),
                now,
            })?;
        }
        Ok(())
    }

    pub(crate) async fn relay_leased_watched_process(
        &self,
        leased: &str,
        action: LeasedWatchedProcessAction,
    ) -> Result<Option<LeasedProcessReceipt>, DaemonError> {
        let _operation = self.leased_agent_operations.lock(leased).await;
        let (id, binding) = match &action {
            LeasedWatchedProcessAction::Start { wake, binding, .. } => (wake.id.as_str(), binding),
            LeasedWatchedProcessAction::Query { wake_id, binding }
            | LeasedWatchedProcessAction::Cancel { wake_id, binding } => {
                (wake_id.as_str(), binding)
            }
        };
        if binding.leased_agent_id != leased {
            return Err(ledger::error("leased process request identity mismatch"));
        }
        let leased = leased.to_string();
        self.with_app_side_effect(move |app| {
            crate::app::RemoteLeaseRuntime::new(app).consume_leased_agent_authorization(&leased)
        })
        .await?;
        let store = &self.owned.durable_state_store;
        if let Some(mut stored) = store.leased_process_receipt(id)? {
            if &stored.binding != binding {
                return Err(ledger::error(
                    "leased process receipt belongs to another lease or run",
                ));
            }
            if let LeasedWatchedProcessAction::Start { wake, cwd, .. } = &action {
                if process_digest(wake, cwd)? != stored.command_digest {
                    return Err(ledger::error("leased process command identity changed"));
                }
            }
            let (b, w) = (binding.clone(), stored.wake.clone());
            let target = self
                .with_app_side_effect(move |app| {
                    crate::app::RemoteLeaseRuntime::new(app).leased_process_target(&b, &w, false)
                })
                .await;
            match target {
                Ok(_) => {}
                // The authenticated home owns this exact stored binding. Lease loss
                // does not revoke its ability to stop that launch, but cannot authorize Start.
                Err(
                    DaemonError::LeasedAgentNotFound { .. }
                    | DaemonError::ExecutionLeaseNotFound { .. },
                ) if matches!(
                    action,
                    LeasedWatchedProcessAction::Query { .. }
                        | LeasedWatchedProcessAction::Cancel { .. }
                ) => {}
                Err(error) => return Err(error),
            }
            if matches!(action, LeasedWatchedProcessAction::Cancel { .. })
                && matches!(stored.wake.state.as_str(), "starting" | "running")
            {
                let revision = stored.revision;
                stored.wake.state = "cancelling".into();
                store.agent_lifecycle(Operation::LeasedProcess(LeasedProcessMutation::Update {
                    id: id.into(),
                    expected_revision: revision,
                    receipt: stored,
                }))?;
                self.owned.agent_wakes.processes.terminate(id);
            }
            // A persisted PID is never a supervised handle after worker restart.
            let stored = store.leased_process_receipt(id)?.unwrap();
            if matches!(
                stored.wake.state.as_str(),
                "starting" | "running" | "cancelling"
            ) && !self.owned.agent_wakes.processes.contains(id)
            {
                let launch = stored
                    .process_start_ticks
                    .and_then(super::agent_process_group::start_time_ms)
                    .unwrap_or(stored.wake.created_at_ms);
                // A receipt never grants arbitrary PID authority. The common reaper
                // pins only attributable orphan descendants of this owned launch.
                let _ = super::agent_process_group::reap_orphans(
                    &[(stored.wake.id.clone(), launch)].into(),
                );
                self.apply_agent_process_receipt(Operation::ProcessExited{id:id.into(),exit_code:None,tail:"process_lost: worker no longer supervises the launch; it is never relaunched".into(),now:crate::session::unix_epoch_ms()})?;
            }
            return store.leased_process_receipt(id);
        }
        let LeasedWatchedProcessAction::Start {
            mut wake,
            binding,
            cwd,
        } = action
        else {
            return Ok(None);
        };
        if !cfg!(target_os = "linux") {
            return Err(ledger::error("leased watched processes require Linux ownership verification on the execution worker"));
        }
        if wake.kind != "process"
            || wake.command.is_empty()
            || wake.command.len() > 64
            || wake.command[0].is_empty()
            || wake
                .command
                .iter()
                .any(|s| s.len() > 4096 || s.chars().any(char::is_control))
            || wake
                .match_text
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 200 || s.chars().any(char::is_control))
        {
            return Err(ledger::error("invalid leased process argv or match"));
        }
        let (b, w) = (binding.clone(), wake.clone());
        let (room, agent, prompt) = self
            .with_app_side_effect(move |app| {
                crate::app::RemoteLeaseRuntime::new(app).leased_process_target(&b, &w, true)
            })
            .await?;
        let run = self
            .owned
            .provider_store
            .get_run(&binding.worker_provider_run_id)?;
        if run.state() != crate::provider::ProviderRunState::Running
            || run.agent_instance_id() != Some(agent.as_str())
            || run.session_id() != room
            || ((run.execution_mode() != crate::provider::AgentExecutionMode::Build
                || run.permission_level() != crate::provider::AgentPermissionLevel::Yolo
                || run.write_access_mode()
                    != crate::provider::ProviderWriteAccessMode::Unrestricted)
                && !self.sudo_task_authorizes_run(&run).await)
        {
            return Err(ledger::error("leased process requires an unrestricted build run with unattended command permission"));
        }
        let root = run
            .working_directory()
            .and_then(|p| p.canonicalize().ok())
            .ok_or_else(|| ledger::error("worker workspace unavailable"))?;
        let directory = root
            .join(cwd.as_deref().unwrap_or("."))
            .canonicalize()
            .ok()
            .filter(|p| p.starts_with(&root) && p.is_dir())
            .ok_or_else(|| ledger::error("cwd must be inside the leased workspace"))?;
        let mut approval_wake = wake.clone();
        approval_wake.room_id = room.clone();
        approval_wake.agent_id = agent.clone();
        self.approve_agent_process(
            &run,
            &approval_wake,
            &directory,
            prompt
                .as_deref()
                .ok_or_else(|| ledger::error("worker prompt missing"))?,
        )
        .await?;
        let (b, w) = (binding.clone(), wake.clone());
        self.with_app_side_effect(move |app| {
            crate::app::RemoteLeaseRuntime::new(app).leased_process_target(&b, &w, true)
        })
        .await?;
        if self.owned.provider_store.get_run(run.id())?.state()
            != crate::provider::ProviderRunState::Running
        {
            return Err(ledger::error("leased process run ended before launch"));
        }
        if (run.permission_level() != crate::provider::AgentPermissionLevel::Yolo
            || run.write_access_mode() != crate::provider::ProviderWriteAccessMode::Unrestricted
            || run.execution_mode() != crate::provider::AgentExecutionMode::Build)
            && !self.sudo_task_authorizes_run(&run).await
        {
            return Err(ledger::error(
                "home sudo task ended before watched process launch",
            ));
        }
        wake.state = "starting".into();
        // Worker clock pins orphan recovery even when home clocks differ.
        wake.created_at_ms = crate::session::unix_epoch_ms();
        wake.pid = None;
        wake.verified_at_ms = None;
        let receipt = LeasedProcessReceipt {
            command_digest: process_digest(&wake, &cwd)?,
            worker_boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .map_err(|_| ledger::error("worker boot identity unavailable"))?
                .trim()
                .into(),
            process_start_ticks: None,
            matched_line: None,
            output_tail: String::new(),
            revision: 1,
            backing_room: room,
            backing_agent: agent,
            binding,
            wake: wake.clone(),
        };
        let Outcome::LeasedProcess(Some(admitted)) = store.agent_lifecycle(
            Operation::LeasedProcess(LeasedProcessMutation::Intent(receipt)),
        )?
        else {
            return Err(ledger::error("process intent receipt missing"));
        };
        // The intent is never replayed as a new launch, including a lost start response.
        if admitted.wake.state != "starting" {
            return Ok(Some(admitted));
        }
        self.start_agent_process(
            wake.clone(),
            wake.command.clone(),
            directory,
            &wake.task_id,
            &wake.task_id,
            &run,
        )
        .await?;
        store.leased_process_receipt(&wake.id)
    }
}
