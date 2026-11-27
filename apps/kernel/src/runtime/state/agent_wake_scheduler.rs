//! MP-08 / MP-09 / MP-10 / MP-11 A03: kernel-owned wake scheduler.
//! The 1 s scheduler fires durable wakes; the 30 s lifecycle sweep is its
//! dead-man switch and each watches the other's heartbeat. Neither can stall
//! silently: every miss raises a notice on all clients and is retried.
use super::wake_driver::{Liveness, Recovery};
use super::*;
use crate::durable_state::agent_lifecycle::{AgentWake, Operation, Outcome, WakeEnd, SWEEP_MS};
use std::sync::atomic::AtomicBool;

const WAKE_TICK_MS: u64 = 1_000;
/// A due timer fires within this bound or the sweep alerts and fires it.
pub(crate) const WAKE_FIRE_TOLERANCE_MS: u64 = 15_000;
/// A fired wake reaches its turn within this bound or the sweep alerts.
pub(crate) const WAKE_DELIVERY_TOLERANCE_MS: u64 = 60_000;
const VERIFY_WAIT_MS: u64 = 5_000;
/// Drill-only stall fault: while this file exists the scheduler skips ticks.
const STALL_FAULT_ENV: &str = "CHARIOX_WAKE_SCHEDULER_STALL_FILE";
/// Drill-only task fault: while this file lists an agent id (one per line), the lifecycle sweep's
/// settlement of that agent's tasks does not end.
const TASK_STALL_FAULT_ENV: &str = "CHARIOX_SWEEP_TASK_STALL_FILE";

/// What a report of a lost watch tells its agent to do: the kernel never starts the work again, the agent does.
const LOST_NEXT_STEP: &str = "what it watched is gone: start it again and watch the new one";

pub(crate) struct AgentWakeMonitor {
    pub(super) started_at_ms: u64,
    scheduler_tick_ms: AtomicU64,
    sweep_ms: AtomicU64,
    scheduler_stall_alerted: AtomicBool,
    sweep_stall_alerted: AtomicBool,
    tick_failure_alerted: AtomicBool,
    stall_fault: Option<PathBuf>,
    task_stall_fault: std::sync::Mutex<Option<PathBuf>>,
    /// What the scheduler loop and the lifecycle sweep wait on, named in their stall reports.
    pub(super) scheduler_step: super::supervision_step::SupervisionStep,
    pub(super) sweep_step: super::supervision_step::SupervisionStep,
    pub(super) leased_polls: std::sync::Mutex<BTreeSet<String>>,
    pub(super) processes: super::agent_process_watch::WatchedProcesses,
    /// Sources of every other kind; watched processes stay with `processes`.
    pub(super) drivers: super::wake_driver::WakeDrivers,
}

impl AgentWakeMonitor {
    pub(super) fn new() -> Self {
        let monitor = Self {
            started_at_ms: crate::session::unix_epoch_ms(),
            scheduler_tick_ms: AtomicU64::new(0),
            sweep_ms: AtomicU64::new(0),
            scheduler_stall_alerted: AtomicBool::new(false),
            sweep_stall_alerted: AtomicBool::new(false),
            tick_failure_alerted: AtomicBool::new(false),
            stall_fault: std::env::var_os(STALL_FAULT_ENV).map(PathBuf::from),
            task_stall_fault: std::sync::Mutex::new(
                std::env::var_os(TASK_STALL_FAULT_ENV).map(PathBuf::from),
            ),
            processes: Default::default(),
            scheduler_step: Default::default(),
            sweep_step: Default::default(),
            leased_polls: Default::default(),
            drivers: Default::default(),
        };
        monitor
            .drivers
            .register(Arc::new(super::wake_driver::TimerDriver));
        monitor
            .drivers
            .register(Arc::new(super::owner_wait::OwnerWaitDriver));
        monitor
    }

    /// How long the scheduler has been silent: since its last tick, or since the kernel started
    /// when it has not ticked yet.
    fn scheduler_silent_ms(&self, now: u64) -> u64 {
        let tick = self.scheduler_tick_ms.load(Ordering::Acquire);
        now.saturating_sub(tick.max(self.started_at_ms))
    }

    #[cfg(test)]
    pub(super) fn set_task_stall_fault(&self, file: PathBuf) {
        *self
            .task_stall_fault
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(file);
    }

    /// Drill fault: a settlement of `agent`'s task that does not end while the fault file names it.
    pub(super) async fn hold_while_task_stall_fault_names(&self, agent: &str) {
        loop {
            let file = self
                .task_stall_fault
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let named = file
                .and_then(|file| std::fs::read_to_string(file).ok())
                .is_some_and(|names| names.lines().any(|name| name.trim() == agent));
            if !named {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Why the lifecycle sweep has not passed its dead-man check since `since`.
    fn sweep_stall_report(&self, since: u64) -> String {
        let held = match self.sweep_step.current() {
            Some((step, entered)) => format!("its pass waits on {step} since {}", iso(entered)),
            None => "no pass has started since".into(),
        };
        format!("Kernel supervision sweep stalled since {}: {held}; wake delivery checks are degraded; timers still fire", iso(since))
    }
}

fn armed(w: &AgentWake) -> bool {
    matches!(
        w.state.as_str(),
        "scheduled" | "starting" | "running" | "cancelling"
    )
}

pub(super) fn iso(ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64)
        .map(|t| t.format("%H:%M:%SZ").to_string())
        .unwrap_or_else(|| ms.to_string())
}

impl KernelRuntimeState {
    pub(crate) async fn run_agent_wake_scheduler(&self) {
        // JoinSet aborts the separate maintenance task when this scheduler is
        // cancelled at kernel shutdown, including a pending Vault interaction.
        let mut maintenance = tokio::task::JoinSet::new();
        // An orphan's cleanup can wait on a worker or on the process registry;
        // due wakes must not wait with it. One pass at a time.
        let mut orphan_sweep = tokio::task::JoinSet::new();
        let mut recovered = false;
        let mut worker_recovered = false;
        loop {
            let step = &self.owned.agent_wakes.scheduler_step;
            if maintenance.is_empty() {
                let state = self.clone();
                maintenance.spawn(async move { state.run_disk_maintenance().await });
            }
            step.enter("deadline cleanup");
            if let Err(error) = self
                .tick_agent_deadlines(crate::session::unix_epoch_ms())
                .await
            {
                tracing::warn!(%error, "MP-08/MP-09/MP-10/MP-11: deadline cleanup retained for retry");
            }
            step.enter("usage limit resumes");
            if let Err(error) = self
                .tick_usage_limit_stops(crate::session::unix_epoch_ms())
                .await
            {
                tracing::warn!(%error, "usage limit resume retained for retry");
            }
            while orphan_sweep.try_join_next().is_some() {}
            if orphan_sweep.is_empty() {
                let state = self.clone();
                orphan_sweep.spawn(async move { state.end_orphaned_sub_agents().await });
            }
            step.enter("agent tree publication");
            self.publish_changed_agent_trees();
            step.enter("watched process recovery");
            if !worker_recovered {
                worker_recovered = self
                    .recover_worker_processes(
                        crate::hot_reload::take_watched(),
                        crate::session::unix_epoch_ms(),
                    )
                    .is_ok();
            }
            step.enter("watched process waits");
            self.deliver_process_waits();
            step.enter("the wake tick");
            let monitor = &self.owned.agent_wakes;
            let paused = monitor.stall_fault.as_ref().is_some_and(|p| p.exists());
            if !paused
                && self.owned.config_projection.snapshot().room_agent_tools
                && self.owned.publication_activation.is_active()
            {
                let now = crate::session::unix_epoch_ms();
                monitor.scheduler_tick_ms.store(now, Ordering::Release);
                if monitor
                    .scheduler_stall_alerted
                    .swap(false, Ordering::AcqRel)
                {
                    self.notify_wake_owners(None, |_| {
                        "Wake scheduler recovered; due wakes fire on time again".into()
                    });
                }
                if !recovered {
                    recovered = self
                        .recover_lost_agent_processes(now)
                        .and_then(|()| self.recover_driver_wakes(now))
                        .is_ok();
                }
                match self.tick_agent_wakes(now, true).await {
                    Err(error) => {
                        tracing::warn!(%error, "MP-08/MP-09/MP-10/MP-11 A03: wake tick retained for the dead-man sweep");
                        if !monitor.tick_failure_alerted.swap(true, Ordering::AcqRel) {
                            self.notify_wake_owners(None, |_| "Wake alert: durable wake supervision failed; timers and receipts are not assumed successful. Retrying; restore durable state if this persists".into());
                        }
                    }
                    Ok(()) => {
                        if monitor.tick_failure_alerted.swap(false, Ordering::AcqRel) {
                            self.notify_wake_owners(None, |_| {
                                "Durable wake supervision recovered".into()
                            });
                        }
                    }
                }
                let sweep = monitor.sweep_ms.load(Ordering::Acquire);
                if sweep != 0
                    && now.saturating_sub(sweep) > 3 * SWEEP_MS
                    && !monitor.sweep_stall_alerted.swap(true, Ordering::AcqRel)
                {
                    let report = monitor.sweep_stall_report(sweep);
                    tracing::warn!(%report, "MP-08/MP-09/MP-10/MP-11 A03: lifecycle sweep stalled");
                    self.notify_wake_owners(None, |_| report.clone());
                }
            }
            step.leave();
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(WAKE_TICK_MS)) => {},
                result = maintenance.join_next() => {
                    tracing::warn!(?result, "disk maintenance stopped; restarting on the next tick");
                    tokio::time::sleep(Duration::from_millis(WAKE_TICK_MS)).await;
                }
            }
        }
    }

    /// Fires due wakes and (from the scheduler only) arms new ones.
    async fn tick_agent_wakes(&self, now: u64, verify: bool) -> Result<(), DaemonError> {
        // Timers read only their scheduled rows. A failed reconciliation or an unreadable
        // historical wake must not hold back the kernel clock.
        self.tick_timer_wakes(now, verify)?;
        self.tick_interaction_wakes(now)?;
        self.resume_browser_region_cards();
        let store = &self.owned.durable_state_store;
        for wake in store.agent_wakes(None, None)? {
            if matches!(wake.state.as_str(), "starting" | "running" | "cancelling")
                && store.leased_wake_binding(&wake.id)?.is_some()
            {
                self.ensure_leased_process_poll(&wake);
            }
        }
        Ok(())
    }

    fn tick_timer_wakes(&self, now: u64, verify: bool) -> Result<(), DaemonError> {
        let store = &self.owned.durable_state_store;
        let timers = store.agent_armed_timers()?;
        if verify && timers.iter().any(|w| w.verified_at_ms.is_none()) {
            store.agent_lifecycle(Operation::VerifyWakes { now })?;
        }
        let due: Vec<_> = timers
            .into_iter()
            .filter(|w| w.next_due_ms.is_some_and(|d| d <= now))
            .collect();
        if due.is_empty() {
            return Ok(());
        }
        let Outcome::Wakes(fired) = store
            .agent_lifecycle(Operation::FireWakes { now })
            .inspect_err(|_| {
                for wake in &due {
                    self.deliver_committed_wake(wake);
                }
            })?
        else {
            return Err(crate::durable_state::agent_lifecycle::error(
                "wake fire outcome mismatch",
            ));
        };
        let mut recipients = BTreeSet::new();
        for wake in fired {
            if wake.last_delivery.as_deref() == Some("backpressured") {
                self.wake_notice(&wake, format!("Wake alert: '{}' is backpressured by its recipient; its due occurrence is retained and other timers continue", wake.label));
                recipients.insert((wake.room_id, wake.agent_id));
                continue;
            }
            if wake.state == "cancelled" {
                self.wake_notice(&wake, format!("Wake alert: '{}' was due and ended without firing: its task is blocked until its owner resumes or cancels it", wake.label));
                recipients.insert((wake.room_id, wake.agent_id));
                continue;
            }
            let due = due
                .iter()
                .find(|d| d.id == wake.id)
                .and_then(|d| d.next_due_ms)
                .unwrap_or(now);
            let late = now.saturating_sub(due);
            let text = if late > WAKE_FIRE_TOLERANCE_MS {
                format!("Wake alert: '{}' was due at {} and fired {} s late (kernel down, host asleep or scheduler stalled); delivering now", wake.label, iso(due), late / 1000)
            } else {
                format!("Wake '{}' fired (due {})", wake.label, iso(due))
            };
            self.wake_notice(&wake, text);
            self.close_cards_following(&wake);
            recipients.insert((wake.room_id, wake.agent_id));
        }
        for (room, agent) in recipients {
            let state = self.clone();
            tokio::spawn(async move {
                if let Err(error) = Box::pin(state.deliver_agent_inbox(&room, &agent)).await {
                    tracing::warn!(%error, "MP-08/MP-09/MP-10/MP-11 A03: wake delivery retained for the sweep");
                }
                state.refresh_wake_projection(&room);
            });
        }
        Ok(())
    }

    /// Dead-man switch run by the 30 s lifecycle sweep.
    pub(super) async fn sweep_agent_wakes(&self, now: u64) -> Result<(), DaemonError> {
        // A deadline whose sources are still stopping retries on its own; it
        // must not hold back supervision of every other wake.
        if let Err(error) = self.tick_agent_deadlines(now).await {
            tracing::warn!(%error, "MP-08/MP-09/MP-10/MP-11: deadline cleanup retained for retry");
        }
        // Every worker is asked at once with its own 5 s bound; this bound only keeps a stuck
        // relay from holding the sweep. An unanswered worker's entry stays for the next one.
        match tokio::time::timeout(Duration::from_secs(10), self.rearm_leased_agent_deadlines()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "a worker still holds a leased agent's deadline from before the pause; retrying at the next sweep"),
            Err(_) => tracing::warn!("telling the workers the shifted deadlines took too long; retrying at the next sweep"),
        }
        let monitor = &self.owned.agent_wakes;
        monitor.sweep_ms.store(now, Ordering::Release);
        monitor.sweep_stall_alerted.store(false, Ordering::Release);
        let store = &self.owned.durable_state_store;
        let tick = monitor.scheduler_tick_ms.load(Ordering::Acquire);
        let timers = store.agent_armed_timers()?;
        if !timers.is_empty()
            && monitor.scheduler_silent_ms(now) > WAKE_FIRE_TOLERANCE_MS
            && !monitor.scheduler_stall_alerted.swap(true, Ordering::AcqRel)
        {
            let since = if tick == 0 {
                "kernel start".into()
            } else {
                iso(tick)
            };
            let held = match monitor.scheduler_step.current() {
                Some((step, entered)) => format!("; it waits on {step} since {}", iso(entered)),
                None => String::new(),
            };
            let report = format!("Wake alert: scheduler stalled (no tick since {since}{held}); the kernel sweep fires due wakes until it recovers");
            tracing::warn!(%report, "MP-08/MP-09/MP-10/MP-11 A03: wake scheduler stalled");
            self.notify_wake_owners(Some(&timers), |_| report.clone());
        }
        if timers.iter().any(|w| {
            w.next_due_ms
                .is_some_and(|d| d + WAKE_FIRE_TOLERANCE_MS < now)
        }) {
            self.tick_timer_wakes(now, false)?;
        }
        self.sweep_driver_liveness(now)?;
        let wakes: BTreeMap<_, _> = store
            .agent_wakes(None, None)?
            .into_iter()
            .map(|w| (w.id.clone(), w))
            .collect();
        for receipt in store.agent_wake_receipts(None, None)? {
            let Some(wake) = wakes.get(&receipt.wake_id) else {
                continue;
            };
            if receipt.delivered_at_ms.is_none()
                && receipt
                    .fired_at_ms
                    .saturating_add(WAKE_DELIVERY_TOLERANCE_MS)
                    < now
                && !receipt.alerted
                && receipt.delivery != "expired"
            {
                store.agent_lifecycle(Operation::WakeAlerted {
                    id: wake.id.clone(),
                    sequence: receipt.sequence,
                })?;
                let outcome = if receipt.delivery == "failed" {
                    "the agent did not start and the wake will not be retried"
                } else {
                    "retrying"
                };
                self.wake_notice(wake, format!("Wake alert: '{}' fire {} at {} is not delivered after {} s (delivery {}); {outcome}", wake.label, receipt.sequence, iso(receipt.fired_at_ms), (now - receipt.fired_at_ms) / 1000, receipt.delivery));
            }
        }
        Ok(())
    }

    /// Proof of life: wait for the scheduler's arm receipt of a new timer.
    pub(super) async fn await_wake_verification(&self, id: &str) -> Option<u64> {
        let deadline = Instant::now() + Duration::from_millis(VERIFY_WAIT_MS);
        while Instant::now() < deadline {
            let verified = self
                .owned
                .durable_state_store
                .agent_wake_verification(id)
                .ok()?;
            if verified.is_some() {
                return verified;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    #[cfg(test)]
    pub(crate) fn agent_wakes_last_swept_for_test(&self, at_ms: u64) {
        self.owned
            .agent_wakes
            .sweep_ms
            .store(at_ms, Ordering::Release);
    }

    pub(super) fn wake_scheduler_health(&self, now: u64) -> serde_json::Value {
        let monitor = &self.owned.agent_wakes;
        let tick = monitor.scheduler_tick_ms.load(Ordering::Acquire);
        serde_json::json!({"last_tick_at_ms":tick,"stalled":monitor.scheduler_silent_ms(now) > WAKE_FIRE_TOLERANCE_MS,"fire_tolerance_ms":WAKE_FIRE_TOLERANCE_MS,"delivery_tolerance_ms":WAKE_DELIVERY_TOLERANCE_MS})
    }

    /// After a restart no process from an earlier kernel is supervised; it is
    /// reported lost and never relaunched. Descendants it left running are
    /// identified by their wake marker and stopped.
    pub(super) fn recover_lost_agent_processes(&self, now: u64) -> Result<(), DaemonError> {
        self.recover_lost_agent_processes_with(
            now,
            super::agent_process_group::reap_orphans,
            crate::provider::managed_provider_isolation_required(),
        )
    }

    /// `reap` stops the orphans of the given wakes (wake id -> created at) and
    /// returns how many each had; it is a parameter so a failed cleanup can be tested.
    /// `isolated`: the managed PID namespace makes the scan complete; without it
    /// only processes carrying the wake marker are found.
    pub(super) fn recover_lost_agent_processes_with(
        &self,
        now: u64,
        reap: impl FnOnce(
            &std::collections::BTreeMap<String, u64>,
        ) -> std::io::Result<std::collections::BTreeMap<String, usize>>,
        isolated: bool,
    ) -> Result<(), DaemonError> {
        let monitor = &self.owned.agent_wakes;
        let lost: Vec<_> = self
            .owned
            .durable_state_store
            .agent_wakes(None, None)?
            .into_iter()
            .filter(|wake| {
                wake.kind == "process"
                    && matches!(wake.state.as_str(), "starting" | "running" | "cancelling")
                    && wake.created_at_ms < monitor.started_at_ms
                    && !monitor.processes.contains(&wake.id)
                    && self
                        .owned
                        .durable_state_store
                        .leased_wake_binding(&wake.id)
                        .is_ok_and(|binding| binding.is_none())
            })
            .collect();
        if lost.is_empty() {
            return Ok(());
        }
        let stopped = reap(
            &lost
                .iter()
                .map(|wake| (wake.id.clone(), wake.created_at_ms))
                .collect(),
        );
        for wake in lost {
            let mut orphans = match &stopped {
                Ok(stopped) => stopped.get(&wake.id).map(|count| format!("; {count} process(es) it left running were stopped and their exits confirmed")).unwrap_or_default(),
                Err(_) => "; escaped subtree settlement is unconfirmed after bounded cleanup; survivors may still be running and require operator cleanup".into(),
            };
            // Without the managed PID namespace, survivors are found only by
            // their inherited wake marker.
            if !isolated {
                orphans.push_str("; descendants that cleared their environment or have a live parent outside the watched session cannot be attributed and may still be running");
            }
            // Only the managed PID namespace makes an Ok scan complete.
            let outcome = match (&stopped, isolated) {
                (Ok(stopped), true) if stopped.contains_key(&wake.id) => LostWatch::Stopped,
                (Ok(_), true) => LostWatch::NotFound,
                (Ok(stopped), false) if stopped.contains_key(&wake.id) => LostWatch::StoppedSome,
                (Ok(_), false) => LostWatch::NoneAttributed,
                (Err(_), _) => LostWatch::Unconfirmed,
            };
            let report = lost_watch_report(&wake, now, outcome);
            self.owned.durable_state_store.agent_lifecycle(Operation::ProcessExited {
                id: wake.id.clone(),
                exit_code: None,
                tail: format!("process_lost: {report}{orphans}"),
                now,
            })?;
            self.wake_notice(&wake, format!("Wake alert: {report}{orphans}"));
            self.schedule_wake_delivery(&wake);
        }
        Ok(())
    }

    /// After a restart a driver-held source from an earlier kernel is either
    /// reattached by its driver or reported lost; it is never relaunched.
    pub(super) fn recover_driver_wakes(&self, now: u64) -> Result<(), DaemonError> {
        let monitor = &self.owned.agent_wakes;
        for wake in self.owned.durable_state_store.agent_wakes(None, None)? {
            if wake.kind == "process"
                || !matches!(wake.state.as_str(), "starting" | "running" | "cancelling")
                || wake.created_at_ms >= monitor.started_at_ms
            {
                continue;
            }
            let recovery = match monitor.drivers.get(&wake.kind) {
                Ok(driver) => driver.recover(&wake),
                Err(error) => Recovery::Lost(error.to_string()),
            };
            if let Recovery::Lost(reason) = recovery {
                self.withdraw_owner_wait_card(&wake);
                self.report_lost_wake(&wake, now, &format!("the kernel restarted: {reason}"))?;
            }
        }
        monitor.drivers.reconcile_ended();
        Ok(())
    }

    /// Dead-man check for driver-held sources, run by the lifecycle sweep.
    fn sweep_driver_liveness(&self, now: u64) -> Result<(), DaemonError> {
        for wake in self.owned.durable_state_store.agent_wakes(None, None)? {
            if wake.kind == "process" || wake.state != "running" {
                continue;
            }
            let liveness = match self.owned.agent_wakes.drivers.get(&wake.kind) {
                Ok(driver) => driver.liveness(&wake),
                Err(error) => Liveness::Lost(error.to_string()),
            };
            if let Liveness::Lost(reason) = liveness {
                self.report_lost_wake(&wake, now, &reason)?;
            }
        }
        Ok(())
    }

    pub(super) fn report_lost_wake(
        &self,
        wake: &AgentWake,
        now: u64,
        reason: &str,
    ) -> Result<(), DaemonError> {
        let reason = crate::secret_redaction::redact_secrets(reason).into_owned();
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::WakeOccurred {
                id: wake.id.clone(),
                occurrence: format!("{}-lost", wake.id),
                kind: "source_lost_reported".into(),
                payload: serde_json::json!({"reason": reason}),
                summary: Some(format!("lost: {reason}")),
                end: Some(WakeEnd::Lost),
                now,
            })?;
        self.wake_notice(
            wake,
            format!(
                "Wake alert: '{}' was lost ({reason}); it is not relaunched; {LOST_NEXT_STEP}",
                wake.label
            ),
        );
        self.schedule_wake_delivery(wake);
        Ok(())
    }

    /// A failed writer reply can follow a successful commit. Only a new
    /// durable occurrence warrants delivery when the writer returned an error.
    pub(super) fn deliver_committed_wake(&self, before: &AgentWake) {
        match self.owned.durable_state_store.agent_wake(&before.id) {
            Ok(Some(wake)) if wake.last_sequence > before.last_sequence => {
                self.schedule_wake_delivery(&wake);
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, wake_id=%before.id,
                "could not reconcile an unconfirmed wake commit; the sweep retains delivery"),
        }
    }

    pub(super) fn schedule_wake_delivery(&self, wake: &AgentWake) {
        let state = self.clone();
        let (room, agent) = (wake.room_id.clone(), wake.agent_id.clone());
        let wake = wake.clone();
        tokio::spawn(async move {
            state.close_cards_following(&wake);
            let _ = Box::pin(state.deliver_agent_inbox(&room, &agent)).await;
            state.refresh_wake_projection(&room);
        });
    }

    pub(super) fn wake_notice(&self, wake: &AgentWake, text: String) {
        self.owned.record_notice_for_agent(
            &wake.room_id,
            None,
            Some(&wake.agent_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(&wake.room_id),
            crate::secret_redaction::redact_secrets(&text).into_owned(),
        );
        self.refresh_wake_projection(&wake.room_id);
    }

    fn notify_wake_owners(&self, wakes: Option<&[AgentWake]>, text: impl Fn(&AgentWake) -> String) {
        let all;
        let wakes = match wakes {
            Some(w) => w,
            None => {
                let stored = match self.owned.durable_state_store.agent_wakes(None, None) {
                    Ok(wakes) => wakes,
                    Err(_) => {
                        for session in self
                            .owned
                            .session_store
                            .list_non_ended_sessions_including_hidden()
                        {
                            self.owned.record_notice_for_agent(session.id(),None,None,
                                self.owned.attachment_store.list_session_attachment_ids(session.id()),
                                "Wake alert: durable wake state is unavailable; supervision is degraded and retrying");
                        }
                        return;
                    }
                };
                all = stored.into_iter().filter(armed).collect::<Vec<_>>();
                &all
            }
        };
        let mut seen = BTreeSet::new();
        for wake in wakes {
            if seen.insert((wake.room_id.clone(), wake.agent_id.clone())) {
                self.wake_notice(wake, text(wake));
            }
        }
    }

    pub(super) fn refresh_wake_projection(&self, room: &str) {
        if self.owned.session_snapshot(room).is_ok() {
            self.owned
                .terminal_stream
                .notify_terminal_projection_change(room);
        }
    }
}

/// What a watch lost across a cold restart tells its owner: when the process
/// was last seen, whether recovery stopped it or it ended while the kernel
/// was down, and (as #1757's `LOST_NEXT_STEP`) what to do.
fn lost_watch_report(wake: &AgentWake, now: u64, outcome: LostWatch) -> String {
    let seen = wake.verified_at_ms.unwrap_or(wake.created_at_ms);
    let (secs, clock) = (
        now.saturating_sub(seen) / 1000,
        (seen / 1000) % 86_400,
    );
    let ago = match secs {
        0..=89 => format!("{secs} s"),
        90..=5_399 => format!("{} min", secs / 60),
        _ => format!("{} h", (secs + 1800) / 3600),
    };
    let (state, next) = match outcome {
        LostWatch::Stopped => ("the kernel restarted and stopped it and its exit is unknown", LOST_NEXT_STEP),
        LostWatch::NotFound => (
            "no process of it was found after the restart, so it ended while the kernel was down and its exit is unknown",
            LOST_NEXT_STEP,
        ),
        // Survivors cannot be ruled out: never say the work is gone.
        LostWatch::StoppedSome => (
            "the kernel restarted and stopped the processes it could attribute to it, but cannot rule out others it started, so it may still be running and its exit is unknown",
            CHECK_FIRST,
        ),
        LostWatch::NoneAttributed => (
            "no process of it was found after the restart, so it most likely ended while the kernel was down, but the kernel cannot rule out others it started, so it may still be running and its exit is unknown",
            CHECK_FIRST,
        ),
        LostWatch::Unconfirmed => (
            "the kernel restarted and could not confirm that it, or anything it started, has stopped, so it may still be running and its exit is unknown",
            "check for it and stop what remains before starting it again",
        ),
    };
    const CHECK_FIRST: &str = "check for what remains first; only then start it again and watch the new one";
    format!(
        "'{}' was last seen running at {:02}:{:02}:{:02} UTC ({ago} ago); {state}; {next}",
        wake.label,
        clock / 3600,
        clock % 3600 / 60,
        clock % 60
    )
}

/// What the restart's cleanup established about a lost watched process.
#[derive(Clone, Copy)]
enum LostWatch {
    /// Processes of the watch were found and stopped, exits confirmed.
    Stopped,
    /// The scan completed and found none of the watch's processes.
    NotFound,
    /// Without the managed PID namespace: some were stopped, but a descendant that
    /// cleared its marker or has a live outside parent cannot be attributed.
    StoppedSome,
    /// Without the managed PID namespace: none was attributed, none can be ruled out.
    NoneAttributed,
    /// The cleanup failed or timed out: survivors cannot be ruled out.
    Unconfirmed,
}
