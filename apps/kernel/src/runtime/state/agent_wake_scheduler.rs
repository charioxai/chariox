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
    stall_fault: Option<PathBuf>,
    pub(super) processes: super::agent_process_watch::WatchedProcesses,
}

impl AgentWakeMonitor {
    pub(super) fn new() -> Self {
        Self {
            started_at_ms: crate::session::unix_epoch_ms(),
            scheduler_tick_ms: AtomicU64::new(0),
            sweep_ms: AtomicU64::new(0),
            scheduler_stall_alerted: AtomicBool::new(false),
            sweep_stall_alerted: AtomicBool::new(false),
            stall_fault: std::env::var_os(STALL_FAULT_ENV).map(PathBuf::from),
            processes: Default::default(),
        }
    }
}

fn armed(w: &AgentWake) -> bool {
    matches!(w.state.as_str(), "scheduled" | "starting" | "running")
}

/// Armed wakes plus the 64 most recent finished ones with their receipts.
pub(super) fn visible_wakes(mut wakes: Vec<AgentWake>, now: u64) -> Vec<AgentWake> {
    wakes.retain(|w| armed(w) || w.last_fired_at_ms.unwrap_or(w.created_at_ms) + 86_400_000 > now);
    let skip = wakes.len().saturating_sub(64);
    wakes.split_off(skip)
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
                if let Err(error) = self.tick_agent_wakes(now, true).await {
                    tracing::warn!(%error, "MP-08/MP-09/MP-10/MP-11 A03: wake tick retained for the dead-man sweep");
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
        for wake in store.agent_wakes(None, None)? {
            let (Some(seq), Some(fired)) = (wake.last_sequence, wake.last_fired_at_ms) else {
                continue;
            };
            if wake.last_delivered_at_ms.is_none()
                && fired + WAKE_DELIVERY_TOLERANCE_MS < now
                && wake.alerted_sequence != Some(seq)
                && !matches!(wake.last_delivery.as_deref(), Some("expired" | "failed"))
            {
                store.agent_lifecycle(Operation::WakeAlerted {
                    id: wake.id.clone(),
                    sequence: seq,
                })?;
                self.wake_notice(&wake, format!("Wake alert: '{}' fired at {} but is not delivered after {} s (delivery {}); retrying", wake.label, iso(fired), (now - fired) / 1000, wake.last_delivery.as_deref().unwrap_or("pending")));
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
                .agent_armed_timers()
                .ok()?
                .into_iter()
                .find(|w| w.id == id)
                .and_then(|w| w.verified_at_ms);
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
    /// reported lost and never relaunched.
    fn recover_lost_agent_processes(&self, now: u64) -> Result<(), DaemonError> {
        let monitor = &self.owned.agent_wakes;
        for wake in self.owned.durable_state_store.agent_wakes(None, None)? {
            if wake.kind == "process"
                && matches!(wake.state.as_str(), "starting" | "running")
                && wake.created_at_ms < monitor.started_at_ms
                && !monitor.processes.contains(&wake.id)
            {
                self.owned.durable_state_store.agent_lifecycle(Operation::ProcessExited {
                    id: wake.id.clone(),
                    exit_code: None,
                    tail: "process_lost: the kernel restarted; it is not supervised and is not relaunched".into(),
                    now,
                })?;
                self.wake_notice(&wake, format!("Wake alert: watched process '{}' was lost when the kernel restarted; it is not relaunched", wake.label));
                self.schedule_wake_delivery(&wake);
            }
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
                all = self
                    .owned
                    .durable_state_store
                    .agent_wakes(None, None)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(armed)
                    .collect::<Vec<_>>();
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
