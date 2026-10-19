//! MP-08 / MP-09 / MP-10 / MP-11 A03: the source interface of the wake
//! service. The ledger owns tasks, obligations, occurrences, receipts and
//! proof of life; a driver owns one `kind` of source (the kernel timer, a
//! terminal, a watcher) and only decides when it occurs. A driver wakes
//! nobody directly: it reports through its `WakeSink`.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentWake, Operation, Outcome, WakeEnd};
use std::sync::RwLock;

/// Result of a stop request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cancel {
    /// Nothing physical was left running; the ledger settles the wake now.
    Done,
    /// The source is stopping; the driver reports the end when it has stopped.
    Pending,
}

/// What a driver knows about a source armed before this kernel started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Recovery {
    Reattached,
    /// Reported as a terminal lost occurrence; the source is never relaunched.
    Lost(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Liveness {
    Live,
    Lost(String),
}

/// Drivers are synchronous: one that needs I/O spawns its own task and
/// reports through its sink.
pub(crate) trait WakeDriver: Send + Sync {
    fn kind(&self) -> &'static str;
    /// Called after the ledger committed the wake. The driver confirms it with
    /// `WakeSink::verified` (the kernel timer is verified by the scheduler) or
    /// fails the arm, which cancels the wake.
    fn arm(&self, wake: &AgentWake) -> Result<(), DaemonError>;
    fn cancel(&self, wake: &AgentWake) -> Result<Cancel, DaemonError>;
    fn recover(&self, wake: &AgentWake) -> Recovery;
    /// Read by the 30 s lifecycle sweep for every running wake of this kind.
    fn liveness(&self, wake: &AgentWake) -> Liveness;
}

#[derive(Default)]
pub(crate) struct WakeDrivers {
    drivers: RwLock<BTreeMap<&'static str, Arc<dyn WakeDriver>>>,
}

impl WakeDrivers {
    pub(super) fn register(&self, driver: Arc<dyn WakeDriver>) {
        let kind = driver.kind();
        let previous = self
            .drivers
            .write()
            .expect("wake drivers lock")
            .insert(kind, driver);
        assert!(previous.is_none(), "wake driver `{kind}` registered twice");
    }
    pub(super) fn get(&self, kind: &str) -> Result<Arc<dyn WakeDriver>, DaemonError> {
        self.drivers
            .read()
            .expect("wake drivers lock")
            .get(kind)
            .cloned()
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::error(format!(
                    "no wake driver is registered for `{kind}`"
                ))
            })
    }
}

/// The only way a driver wakes anyone.
pub(crate) struct WakeOccurrence {
    pub wake_id: String,
    /// Unique per wake; a replay of the same id is one admission.
    pub occurrence_id: String,
    /// What happened, e.g. `output_matched`. The ledger reserves its own kinds.
    pub kind: String,
    pub payload: serde_json::Value,
    /// Updated one-line rendering for clients.
    pub summary: Option<String>,
    /// `Some` ends the source and settles its obligation.
    pub end: Option<WakeEnd>,
}

#[derive(Clone)]
pub(crate) struct WakeSink {
    state: KernelRuntimeState,
    kind: &'static str,
}

impl KernelRuntimeState {
    /// The sink for the driver that owns `kind`; it refuses any wake of
    /// another kind before the ledger is touched.
    pub(crate) fn wake_sink(&self, kind: &'static str) -> WakeSink {
        WakeSink {
            state: self.clone(),
            kind,
        }
    }
    pub(crate) fn register_wake_driver(&self, driver: Arc<dyn WakeDriver>) {
        self.owned.agent_wakes.drivers.register(driver);
    }
}

impl WakeSink {
    fn own_wake(&self, id: &str) -> Result<AgentWake, DaemonError> {
        let wake = self
            .state
            .owned
            .durable_state_store
            .agent_wake(id)?
            .ok_or_else(|| crate::durable_state::agent_lifecycle::error("wake unavailable"))?;
        if wake.kind != self.kind {
            return Err(crate::durable_state::agent_lifecycle::error(
                "a driver can only report for its own wakes",
            ));
        }
        Ok(wake)
    }

    /// Proof of life for a `starting` wake.
    pub(crate) fn verified(&self, wake_id: &str) -> Result<(), DaemonError> {
        let wake = self.own_wake(wake_id)?;
        self.state
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::WakeVerified {
                id: wake.id,
                now: crate::session::unix_epoch_ms(),
            })?;
        self.state.refresh_wake_projection(&wake.room_id);
        Ok(())
    }

    pub(crate) fn occurred(&self, occurrence: WakeOccurrence) -> Result<(), DaemonError> {
        let wake = self.own_wake(&occurrence.wake_id)?;
        let Outcome::Wakes(changed) =
            self.state
                .owned
                .durable_state_store
                .agent_lifecycle(Operation::WakeOccurred {
                    id: wake.id.clone(),
                    occurrence: occurrence.occurrence_id,
                    kind: occurrence.kind,
                    payload: occurrence.payload,
                    summary: occurrence.summary,
                    end: occurrence.end,
                    now: crate::session::unix_epoch_ms(),
                })?
        else {
            return Err(crate::durable_state::agent_lifecycle::error(
                "wake occurrence outcome mismatch",
            ));
        };
        // A replay or an occurrence after the end changes nothing.
        if let Some(changed) = changed.first() {
            self.state.wake_notice(
                changed,
                format!("Wake '{}': {}", changed.label, changed.summary),
            );
            self.state.schedule_wake_delivery(changed);
        }
        Ok(())
    }
}

/// The kernel clock. Its wakes are verified, fired and cancelled inside the
/// ledger by the scheduler, so the driver has nothing physical to run.
pub(super) struct TimerDriver;

impl WakeDriver for TimerDriver {
    fn kind(&self) -> &'static str {
        "timer"
    }
    fn arm(&self, _wake: &AgentWake) -> Result<(), DaemonError> {
        Ok(())
    }
    fn cancel(&self, _wake: &AgentWake) -> Result<Cancel, DaemonError> {
        Ok(Cancel::Done)
    }
    fn recover(&self, _wake: &AgentWake) -> Recovery {
        Recovery::Reattached
    }
    fn liveness(&self, _wake: &AgentWake) -> Liveness {
        Liveness::Live
    }
}

impl KernelRuntimeState {
    /// Cancels one wake. The ledger records the intent first (a driver-held
    /// source becomes `cancelling`), then the driver stops it; the wake is
    /// settled once the driver has physically stopped.
    pub(super) fn cancel_wake_source(
        &self,
        wake: &AgentWake,
        task: &str,
        prompt: Option<&str>,
    ) -> Result<(), DaemonError> {
        let store = &self.owned.durable_state_store;
        let driver_held = wake.state != "scheduled";
        let cancel = |pending| Operation::CancelWake {
            id: wake.id.clone(),
            task: task.into(),
            prompt: prompt.map(str::to_owned),
            pending,
        };
        store.agent_lifecycle(cancel(driver_held))?;
        if driver_held
            && self
                .owned
                .agent_wakes
                .drivers
                .get(&wake.kind)?
                .cancel(wake)?
                == Cancel::Done
        {
            store.agent_lifecycle(cancel(false))?;
        }
        Ok(())
    }
}
