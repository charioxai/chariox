//! MP-08 / MP-09 / MP-10 / MP-11 A03: kernel-owned wake scheduler.
//! The 1 s scheduler fires durable wakes; the 30 s lifecycle sweep is its
//! dead-man switch and each watches the other's heartbeat. Neither can stall
//! silently: every miss raises a notice on all clients and is retried.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentWake, Operation, Outcome, SWEEP_MS};
use std::sync::atomic::AtomicBool;

const WAKE_TICK_MS: u64 = 1_000;
/// A due timer fires within this bound or the sweep alerts and fires it.
pub(crate) const WAKE_FIRE_TOLERANCE_MS: u64 = 15_000;
/// A fired wake reaches its turn within this bound or the sweep alerts.
pub(crate) const WAKE_DELIVERY_TOLERANCE_MS: u64 = 60_000;
const VERIFY_WAIT_MS: u64 = 5_000;
/// Drill-only stall fault: while this file exists the scheduler skips ticks.
const STALL_FAULT_ENV: &str = "CHARIOX_WAKE_SCHEDULER_STALL_FILE";

pub(crate) struct AgentWakeMonitor {
    started_at_ms: u64,
    scheduler_tick_ms: AtomicU64,
    sweep_ms: AtomicU64,
    scheduler_stall_alerted: AtomicBool,
    sweep_stall_alerted: AtomicBool,
    tick_failure_alerted: AtomicBool,
    stall_fault: Option<PathBuf>,
    pub(super) processes: super::agent_process_watch::WatchedProcesses,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a03_projection_keeps_every_armed_wake_when_finished_history_is_bounded() {
        let template: AgentWake = serde_json::from_value(serde_json::json!({
            "id":"wake", "task_id":"task", "room_id":"room", "agent_id":"agent",
            "registration_id":"registration", "kind":"timer", "label":"check-in",
            "state":"scheduled", "created_at_ms":1, "verified_at_ms":1,
            "next_due_ms":100_000, "interval_ms":null, "command":[], "match_text":null,
            "matched_at_ms":null, "pid":null, "exit_code":null, "fire_count":0,
            "missed_fires":0, "last_fired_at_ms":null, "last_sequence":null,
            "last_delivery":null, "last_delivered_at_ms":null,
            "last_acknowledged_at_ms":null, "alerted_sequence":null
        }))
        .unwrap();
        // Several agents may each own up to 32 wakes in one Room.
        let mut wakes = vec![template.clone(); 65];
        let mut finished = template;
        finished.state = "fired".into();
        finished.last_fired_at_ms = Some(2);
        wakes.extend(vec![finished; 100]);
        let projected = visible_wakes(wakes, 3);
        assert_eq!(
            projected.iter().filter(|w| armed(w)).count(),
            65,
            "finished history must not evict pending wakes from any client"
        );
        assert_eq!(projected.iter().filter(|w| !armed(w)).count(), 64);
    }
}

impl AgentWakeMonitor {
    pub(super) fn new() -> Self {
        Self {
            started_at_ms: crate::session::unix_epoch_ms(),
            scheduler_tick_ms: AtomicU64::new(0),
            sweep_ms: AtomicU64::new(0),
            scheduler_stall_alerted: AtomicBool::new(false),
            sweep_stall_alerted: AtomicBool::new(false),
            tick_failure_alerted: AtomicBool::new(false),
            stall_fault: std::env::var_os(STALL_FAULT_ENV).map(PathBuf::from),
            processes: Default::default(),
        }
    }
}

fn armed(w: &AgentWake) -> bool {
    matches!(
        w.state.as_str(),
        "scheduled" | "starting" | "running" | "cancelling"
    )
}

/// Armed wakes plus the 64 most recent finished ones with their receipts.
pub(super) fn visible_wakes(wakes: Vec<AgentWake>, now: u64) -> Vec<AgentWake> {
    let mut finished = 0;
    let mut visible: Vec<_> = wakes
        .into_iter()
        .rev()
        .filter(|w| {
            if armed(w) {
                return true;
            }
            if finished < 64
                && w.last_fired_at_ms
                    .unwrap_or(w.created_at_ms)
                    .saturating_add(86_400_000)
                    > now
            {
                finished += 1;
                return true;
            }
            false
        })
        .collect();
    visible.reverse();
    visible
}

fn iso(ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64)
        .map(|t| t.format("%H:%M:%SZ").to_string())
        .unwrap_or_else(|| ms.to_string())
}

impl KernelRuntimeState {
    pub(crate) async fn run_agent_wake_scheduler(&self) {
        let mut recovered = false;
        loop {
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
                    recovered = self.recover_lost_agent_processes(now).is_ok();
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
                    self.notify_wake_owners(None, |_| format!("Kernel supervision sweep stalled since {}: wake delivery checks are degraded; timers still fire", iso(sweep)));
                }
            }
            tokio::time::sleep(Duration::from_millis(WAKE_TICK_MS)).await;
        }
    }

    /// Fires due wakes and (from the scheduler only) arms new ones.
    async fn tick_agent_wakes(&self, now: u64, verify: bool) -> Result<(), DaemonError> {
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
        let Outcome::Wakes(fired) = store.agent_lifecycle(Operation::FireWakes { now })? else {
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
        let monitor = &self.owned.agent_wakes;
        monitor.sweep_ms.store(now, Ordering::Release);
        monitor.sweep_stall_alerted.store(false, Ordering::Release);
        let store = &self.owned.durable_state_store;
        let tick = monitor.scheduler_tick_ms.load(Ordering::Acquire);
        let timers = store.agent_armed_timers()?;
        if !timers.is_empty()
            && now.saturating_sub(tick) > WAKE_FIRE_TOLERANCE_MS
            && !monitor.scheduler_stall_alerted.swap(true, Ordering::AcqRel)
        {
            let since = if tick == 0 {
                "kernel start".into()
            } else {
                iso(tick)
            };
            self.notify_wake_owners(Some(&timers), |_| format!("Wake alert: scheduler stalled (no tick since {since}); the kernel sweep fires due wakes until it recovers"));
        }
        if timers.iter().any(|w| {
            w.next_due_ms
                .is_some_and(|d| d + WAKE_FIRE_TOLERANCE_MS < now)
        }) {
            self.tick_agent_wakes(now, false).await?;
        }
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
                && !matches!(receipt.delivery.as_str(), "expired" | "failed")
            {
                store.agent_lifecycle(Operation::WakeAlerted {
                    id: wake.id.clone(),
                    sequence: receipt.sequence,
                })?;
                self.wake_notice(wake, format!("Wake alert: '{}' fire {} at {} is not delivered after {} s (delivery {}); retrying", wake.label, receipt.sequence, iso(receipt.fired_at_ms), (now - receipt.fired_at_ms) / 1000, receipt.delivery));
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

    pub(super) fn wake_scheduler_health(&self, now: u64) -> serde_json::Value {
        let tick = self
            .owned
            .agent_wakes
            .scheduler_tick_ms
            .load(Ordering::Acquire);
        serde_json::json!({"last_tick_at_ms":tick,"stalled":now.saturating_sub(tick) > WAKE_FIRE_TOLERANCE_MS,"fire_tolerance_ms":WAKE_FIRE_TOLERANCE_MS,"delivery_tolerance_ms":WAKE_DELIVERY_TOLERANCE_MS})
    }

    /// After a restart no process from an earlier kernel is supervised; it is
    /// reported lost and never relaunched. Descendants it left running are
    /// identified by their wake marker and stopped.
    fn recover_lost_agent_processes(&self, now: u64) -> Result<(), DaemonError> {
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
            })
            .collect();
        if lost.is_empty() {
            return Ok(());
        }
        let stopped = super::agent_process_group::reap_orphans(
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
            if !crate::provider::managed_provider_isolation_required() {
                orphans.push_str("; descendants that cleared their environment or have a live parent outside the watched session cannot be attributed and may still be running");
            }
            self.owned.durable_state_store.agent_lifecycle(Operation::ProcessExited {
                id: wake.id.clone(),
                exit_code: None,
                tail: format!("process_lost: the kernel restarted; it is not supervised and is not relaunched{orphans}"),
                now,
            })?;
            self.wake_notice(&wake, format!("Wake alert: watched process '{}' was lost when the kernel restarted; it is not relaunched{orphans}", wake.label));
            self.schedule_wake_delivery(&wake);
        }
        Ok(())
    }

    pub(super) fn schedule_wake_delivery(&self, wake: &AgentWake) {
        let state = self.clone();
        let (room, agent) = (wake.room_id.clone(), wake.agent_id.clone());
        tokio::spawn(async move {
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
