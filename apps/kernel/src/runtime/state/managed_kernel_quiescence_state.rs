use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;

use super::managed_activity_persistence::{ManagedActivityObservation, ManagedActivityTransitionState};

const QUIESCENCE_EVENT_KIND: &str = "managed_kernel.auto_stop_quiescence.changed";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedKernelQuiescenceChallenge {
    pub(crate) challenge_id: String,
    pub(crate) account_id: String,
    pub(crate) environment_id: String,
    pub(crate) machine_id: String,
    pub(crate) kernel_id: String,
    pub(crate) desired_revision: u64,
    pub(crate) idle_sequence: u32,
    pub(crate) idle_deadline_at: String,
    pub(crate) stop_operation_id: String,
    pub(crate) nonce: String,
}

impl std::fmt::Debug for ManagedKernelQuiescenceChallenge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedKernelQuiescenceChallenge")
            .field("challenge_id", &self.challenge_id)
            .field("account_id", &self.account_id)
            .field("environment_id", &self.environment_id)
            .field("machine_id", &self.machine_id)
            .field("kernel_id", &self.kernel_id)
            .field("desired_revision", &self.desired_revision)
            .field("idle_sequence", &self.idle_sequence)
            .field("idle_deadline_at", &self.idle_deadline_at)
            .field("stop_operation_id", &self.stop_operation_id)
            .field("nonce", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ManagedKernelQuiescenceOutcome {
    Stopped,
    KeepRunning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedQuiescenceState {
    kernel_id: String,
    reservation: Option<PersistedReservation>,
    tombstones: Vec<PersistedReservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedReservation {
    challenge: ManagedKernelQuiescenceChallenge,
    local_idle_transition_sequence: u64,
    local_idle_changed_at_ms: u64,
    decisions: Vec<PersistedReleaseDecision>,
    admission_fenced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedReleaseDecision {
    outcome: ManagedKernelQuiescenceOutcome,
    result_sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConfirmedActivity {
    cloud_sequence: u32,
    local_transition_sequence: u64,
    running_agent_count: u8,
    changed_at_ms: u64,
}

#[derive(Debug)]
struct GateInner {
    state: PersistedQuiescenceState,
    confirmed_activity: Option<ConfirmedActivity>,
    restore_error: Option<String>,
}

/// Shared durable fence for real provider starts and activity admissions.
/// `admission_lock` serializes reservation with all admission commits.
pub(crate) struct ManagedKernelQuiescenceGate {
    store: DurableKernelStateStore,
    kernel_id: String,
    mutation_lock: Arc<Mutex<()>>,
    activity_transitions: ManagedActivityTransitionState,
    admission_lock: Mutex<()>,
    inner: Mutex<GateInner>,
}

pub(crate) struct ManagedKernelAdmissionGuard<'a> {
    _guard: MutexGuard<'a, ()>,
}

impl ManagedKernelQuiescenceGate {
    pub(crate) fn restore(
        store: DurableKernelStateStore,
        kernel_id: String,
        mutation_lock: Arc<Mutex<()>>,
        activity_transitions: ManagedActivityTransitionState,
    ) -> Result<Arc<Self>, DaemonError> {
        let persisted = store
            .load_subject_events_by_kind(&kernel_id, QUIESCENCE_EVENT_KIND, 1)?
            .into_iter()
            .last()
            .map(|event| {
                let state: PersistedQuiescenceState = serde_json::from_value(event.payload)
                    .map_err(|error| quiescence_error(format!("stored quiescence state is invalid: {error}")))?;
                validate_persisted_state(&state, &kernel_id)?;
                Ok(state)
            })
            .transpose()?
            .unwrap_or(PersistedQuiescenceState {
                kernel_id: kernel_id.clone(),
                reservation: None,
                tombstones: Vec::new(),
            });
        Ok(Arc::new(Self {
            store,
            kernel_id,
            mutation_lock,
            activity_transitions,
            admission_lock: Mutex::new(()),
            inner: Mutex::new(GateInner {
                state: persisted,
                // The Cloud sequence-to-local-transition binding is intentionally
                // process-local; it must be re-confirmed by the reporter after restart.
                confirmed_activity: None,
                restore_error: None,
            }),
        }))
    }

    pub(crate) fn unavailable(
        store: DurableKernelStateStore,
        kernel_id: String,
        mutation_lock: Arc<Mutex<()>>,
        activity_transitions: ManagedActivityTransitionState,
        error: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            kernel_id: kernel_id.clone(),
            mutation_lock,
            activity_transitions,
            admission_lock: Mutex::new(()),
            inner: Mutex::new(GateInner {
                state: PersistedQuiescenceState {
                    kernel_id,
                    reservation: None,
                    tombstones: Vec::new(),
                },
                confirmed_activity: None,
                restore_error: Some(error.into()),
            }),
        })
    }

    pub(crate) fn admission_guard(&self) -> Result<ManagedKernelAdmissionGuard<'_>, DaemonError> {
        let guard = self
            .admission_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.ensure_admission_open()?;
        Ok(ManagedKernelAdmissionGuard { _guard: guard })
    }

    pub(crate) fn with_open_admission<T>(
        &self,
        action: impl FnOnce() -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let _guard = self.admission_guard()?;
        action()
    }

    fn ensure_admission_open(&self) -> Result<(), DaemonError> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(error) = &inner.restore_error {
            return Err(quiescence_error(format!("admission is closed because quiescence state could not be restored: {error}")));
        }
        if inner
            .state
            .reservation
            .as_ref()
            .is_some_and(|reservation| reservation.admission_fenced)
        {
            return Err(quiescence_error(
                "managed kernel provider admission is fenced pending an authoritative Cloud release",
            ));
        }
        Ok(())
    }

    pub(crate) fn confirm_activity_report(
        &self,
        cloud_sequence: u32,
        local_transition_sequence: u64,
        observation: ManagedActivityObservation,
    ) {
        let _admission = self
            .admission_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.restore_error.is_some() {
            return;
        }
        inner.confirmed_activity = Some(ConfirmedActivity {
            cloud_sequence,
            local_transition_sequence,
            running_agent_count: observation.running_agent_count,
            changed_at_ms: observation.changed_at_ms,
        });
    }

    /// Called while the shared admission lock is held, then checks current durable
    /// activity under the same mutation boundary before persisting a fence.
    pub(crate) fn reserve_if_current(
        &self,
        challenge: ManagedKernelQuiescenceChallenge,
        current: impl FnOnce() -> Result<(u64, ManagedActivityObservation), DaemonError>,
    ) -> Result<bool, DaemonError> {
        let _admission = self
            .admission_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        {
            let inner = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if inner.restore_error.is_some() {
                return Ok(false);
            }
            if inner.state.tombstones.iter().any(|tombstone| {
                tombstone.challenge.challenge_id == challenge.challenge_id
            }) {
                return Ok(false);
            }
            if let Some(existing) = inner.state.reservation.as_ref() {
                if existing.challenge.challenge_id == challenge.challenge_id
                    && existing.challenge != challenge
                {
                    return Ok(false);
                }
                if existing.challenge == challenge {
                    if existing
                        .decisions
                        .iter()
                        .any(|decision| decision.outcome == ManagedKernelQuiescenceOutcome::KeepRunning)
                    {
                        return Ok(false);
                    }
                    if existing.admission_fenced {
                        return Ok(true);
                    }
                }
                if existing.admission_fenced {
                    return Ok(false);
                }
            }
        }
        let _mutation = self
            .mutation_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (transition_sequence, observation) = current()?;
        if observation.running_agent_count != 0 {
            return Ok(false);
        }
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let confirmed = inner.confirmed_activity;
        if !confirmed.is_some_and(|confirmed| {
            confirmed.cloud_sequence == challenge.idle_sequence
                && confirmed.local_transition_sequence == transition_sequence
                && confirmed.running_agent_count == 0
                && confirmed.changed_at_ms == observation.changed_at_ms
        }) {
            return Ok(false);
        }
        let reservation = PersistedReservation {
            challenge,
            local_idle_transition_sequence: transition_sequence,
            local_idle_changed_at_ms: observation.changed_at_ms,
            decisions: Vec::new(),
            admission_fenced: true,
        };
        let mut state = inner.state.clone();
        state.reservation = Some(reservation);
        self.persist_state(&state)?;
        inner.state = state;
        Ok(true)
    }

    pub(crate) fn apply_release(
        &self,
        challenge: &ManagedKernelQuiescenceChallenge,
        outcome: ManagedKernelQuiescenceOutcome,
        result_sequence: u64,
        current: impl FnOnce() -> Result<(u64, ManagedActivityObservation), DaemonError>,
    ) -> Result<(), DaemonError> {
        let _admission = self
            .admission_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(error) = &inner.restore_error {
            return Err(quiescence_error(format!("cannot apply Cloud release without restored state: {error}")));
        }
        if let Some(tombstone) = inner
            .state
            .tombstones
            .iter()
            .find(|tombstone| tombstone.challenge.challenge_id == challenge.challenge_id)
        {
            if &tombstone.challenge == challenge
                && tombstone.decisions.iter().any(|decision| {
                    decision.outcome == outcome && decision.result_sequence == result_sequence
                })
            {
                return Ok(());
            }
            return Err(quiescence_error("Cloud release conflicts with a durable challenge tombstone"));
        }
        let existing_matches = inner
            .state
            .reservation
            .as_ref()
            .is_some_and(|existing| existing.challenge == *challenge);
        if !existing_matches {
            let can_revoke_before_reservation = inner
                .state
                .reservation
                .as_ref()
                .is_none_or(|existing| !existing.admission_fenced);
            if can_revoke_before_reservation
                && outcome == ManagedKernelQuiescenceOutcome::KeepRunning
                && result_sequence == 1
            {
                drop(inner);
                return self.record_pre_dispatch_keep_running(challenge, result_sequence, current);
            }
            return Err(quiescence_error("Cloud release has no matching durable reservation"));
        }
        let existing = inner
            .state
            .reservation
            .as_ref()
            .expect("matching reservation was checked");
        if &existing.challenge != challenge {
            return Err(quiescence_error("Cloud release does not match the durable reservation"));
        }
        if existing.decisions.iter().any(|decision| {
            decision.outcome == outcome && decision.result_sequence == result_sequence
        }) {
            return Ok(());
        }
        if result_sequence != existing.decisions.len() as u64 + 1
            || existing.decisions.len() >= 2
            || existing.decisions.last().is_some_and(|decision| {
                decision.outcome != ManagedKernelQuiescenceOutcome::Stopped
                    || outcome != ManagedKernelQuiescenceOutcome::KeepRunning
            })
        {
            return Err(quiescence_error("Cloud release sequence or outcome is not authoritative"));
        }
        let mut state = inner.state.clone();
        let tombstone = {
            let reservation = state
                .reservation
                .as_mut()
                .expect("reservation validated above");
            reservation.decisions.push(PersistedReleaseDecision {
                outcome,
                result_sequence,
            });
            reservation.admission_fenced = outcome != ManagedKernelQuiescenceOutcome::KeepRunning;
            (outcome == ManagedKernelQuiescenceOutcome::KeepRunning)
                .then(|| reservation.clone())
        };
        if let Some(tombstone) = tombstone {
            state.tombstones.push(tombstone);
        }
        self.persist_state(&state)?;
        inner.state = state;
        Ok(())
    }

    /// The caller holds `admission_lock`; only an authenticated first keep-running
    /// result for a challenge that has never reserved admission may use this path.
    fn record_pre_dispatch_keep_running(
        &self,
        challenge: &ManagedKernelQuiescenceChallenge,
        result_sequence: u64,
        current: impl FnOnce() -> Result<(u64, ManagedActivityObservation), DaemonError>,
    ) -> Result<(), DaemonError> {
        if result_sequence != 1 {
            return Err(quiescence_error("pre-dispatch keep-running must be the first challenge decision"));
        }
        let _mutation = self
            .mutation_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (transition_sequence, observation) = current()?;
        if observation.running_agent_count != 0 {
            return Err(quiescence_error("pre-dispatch keep-running release is not bound to current idle activity"));
        }
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.restore_error.is_some()
            || inner
                .state
                .reservation
                .as_ref()
                .is_some_and(|reservation| reservation.admission_fenced)
        {
            return Err(quiescence_error("pre-dispatch keep-running cannot alter an active admission fence"));
        }
        let current_confirmation = inner.confirmed_activity;
        if !current_confirmation.is_some_and(|confirmed| {
            confirmed.cloud_sequence == challenge.idle_sequence
                && confirmed.local_transition_sequence == transition_sequence
                && confirmed.running_agent_count == 0
                && confirmed.changed_at_ms == observation.changed_at_ms
        }) {
            return Err(quiescence_error("pre-dispatch keep-running release is stale or unconfirmed"));
        }
        if inner.state.reservation.as_ref().is_some_and(|reservation| {
            reservation.challenge.challenge_id == challenge.challenge_id
        }) {
            return Err(quiescence_error("pre-dispatch keep-running conflicts with another challenge tuple"));
        }
        let mut state = inner.state.clone();
        state.tombstones.push(PersistedReservation {
            challenge: challenge.clone(),
            local_idle_transition_sequence: transition_sequence,
            local_idle_changed_at_ms: observation.changed_at_ms,
            decisions: vec![PersistedReleaseDecision {
                outcome: ManagedKernelQuiescenceOutcome::KeepRunning,
                result_sequence,
            }],
            admission_fenced: false,
        });
        self.persist_state(&state)?;
        inner.state = state;
        Ok(())
    }

    fn persist_state(&self, state: &PersistedQuiescenceState) -> Result<(), DaemonError> {
        self.store.append_event(
            QUIESCENCE_EVENT_KIND,
            Some(self.kernel_id.clone()),
            serde_json::to_value(state).map_err(|error| {
                quiescence_error(format!("could not encode durable quiescence state: {error}"))
            })?,
        )?;
        Ok(())
    }
}

impl super::KernelRuntimeState {
    pub(crate) fn spawn_managed_kernel_quiescence(
        &self,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Option<tokio::task::JoinHandle<Result<(), DaemonError>>> {
        self.owned
            .managed_kernel_quiescence_client
            .clone()
            .map(|client| {
                tokio::spawn(client.run(self.clone(), shutdown))
            })
    }

    pub(crate) fn confirm_managed_activity_report(
        &self,
        cloud_sequence: u32,
        local_transition_sequence: u64,
        observation: ManagedActivityObservation,
    ) {
        if let Some(gate) = &self.owned.managed_kernel_quiescence {
            gate.confirm_activity_report(
                cloud_sequence,
                local_transition_sequence,
                observation,
            );
        }
    }

    pub(crate) fn reserve_managed_kernel_for_stop(
        &self,
        challenge: ManagedKernelQuiescenceChallenge,
    ) -> Result<bool, DaemonError> {
        let Some(gate) = &self.owned.managed_kernel_quiescence else {
            return Ok(false);
        };
        gate.reserve_if_current(challenge, || {
            let running_agent_count = self.owned.managed_running_agent_count_unlocked();
            self.owned
                .managed_activity_transitions
                .current_observation_with_sequence(running_agent_count)
        })
    }

    pub(crate) fn apply_managed_kernel_stop_release(
        &self,
        challenge: &ManagedKernelQuiescenceChallenge,
        outcome: ManagedKernelQuiescenceOutcome,
        result_sequence: u64,
    ) -> Result<(), DaemonError> {
        let gate = self
            .owned
            .managed_kernel_quiescence
            .as_ref()
            .ok_or_else(|| quiescence_error("managed kernel quiescence is not configured"))?;
        gate.apply_release(challenge, outcome, result_sequence, || {
            let running_agent_count = self.owned.managed_running_agent_count_unlocked();
            self.owned
                .managed_activity_transitions
                .current_observation_with_sequence(running_agent_count)
        })
    }
}

fn validate_persisted_state(
    state: &PersistedQuiescenceState,
    kernel_id: &str,
) -> Result<(), DaemonError> {
    let mut challenge_ids = std::collections::HashSet::with_capacity(state.tombstones.len());
    let valid_tombstones = state.tombstones.iter().all(|tombstone| {
        valid_reservation(tombstone, kernel_id)
            && !tombstone.admission_fenced
            && tombstone.decisions.last().is_some_and(|decision| {
                decision.outcome == ManagedKernelQuiescenceOutcome::KeepRunning
            })
            // challenge_id is the existing replay key used by reserve and release handling.
            && challenge_ids.insert(tombstone.challenge.challenge_id.as_str())
    });
    let valid_reservation_state = state.reservation.as_ref().is_none_or(|reservation| {
        valid_reservation(reservation, kernel_id)
            && state
                .tombstones
                .iter()
                .find(|tombstone| {
                    tombstone.challenge.challenge_id == reservation.challenge.challenge_id
                })
                .is_none_or(|tombstone| {
                    // apply_release records a KeepRunning tombstone while retaining
                    // the exact, unfenced reservation as the current challenge.
                    // Only that field-for-field-equivalent replay record may overlap.
                    tombstone == reservation
                })
    });
    let valid = state.kernel_id == kernel_id
        && valid_reservation_state
        && valid_tombstones;
    if valid {
        Ok(())
    } else {
        Err(quiescence_error("stored quiescence identity or fence state is invalid"))
    }
}

fn valid_reservation(reservation: &PersistedReservation, kernel_id: &str) -> bool {
    reservation.challenge.kernel_id == kernel_id
        && reservation.local_idle_transition_sequence > 0
        && reservation.local_idle_changed_at_ms > 0
        && reservation.decisions.len() <= 2
        && reservation
            .decisions
            .iter()
            .all(|decision| decision.result_sequence > 0)
        && match reservation.decisions.as_slice() {
            [] => true,
            [first] => {
                first.result_sequence == 1
                    && matches!(
                        first.outcome,
                        ManagedKernelQuiescenceOutcome::Stopped
                            | ManagedKernelQuiescenceOutcome::KeepRunning
                    )
            }
            [first, second] => {
                first.outcome == ManagedKernelQuiescenceOutcome::Stopped
                    && first.result_sequence == 1
                    && second.outcome == ManagedKernelQuiescenceOutcome::KeepRunning
                    && second.result_sequence == 2
            }
            _ => false,
        }
        && reservation.admission_fenced
            == reservation.decisions.last().is_none_or(|decision| {
                decision.outcome != ManagedKernelQuiescenceOutcome::KeepRunning
            })
}

fn quiescence_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "manage kernel auto-stop quiescence",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_STATE: AtomicU64 = AtomicU64::new(1);

    fn fixture(label: &str) -> (
        std::path::PathBuf,
        DurableKernelStateStore,
        ManagedActivityTransitionState,
        Arc<ManagedKernelQuiescenceGate>,
        Arc<Mutex<()>>,
    ) {
        let path = std::env::temp_dir().join(format!(
            "chariox-quiescence-{label}-{}-{}.sqlite",
            std::process::id(),
            NEXT_STATE.fetch_add(1, Ordering::Relaxed),
        ));
        let store = DurableKernelStateStore::open(path.clone()).expect("open durable store");
        let transitions = ManagedActivityTransitionState::new(store.clone(), Some("kernel-1".into()));
        transitions
            .record_current_transition(|| (0, 0, 1_000))
            .expect("persist initial idle observation");
        let mutation_lock = Arc::new(Mutex::new(()));
        let gate = ManagedKernelQuiescenceGate::restore(
            store.clone(),
            "kernel-1".into(),
            Arc::clone(&mutation_lock),
            transitions.clone(),
        )
        .expect("restore empty admission fence");
        (path, store, transitions, gate, mutation_lock)
    }

    fn challenge(challenge_id: &str, nonce: &str) -> ManagedKernelQuiescenceChallenge {
        ManagedKernelQuiescenceChallenge {
            challenge_id: challenge_id.into(),
            account_id: "account-1".into(),
            environment_id: "environment-1".into(),
            machine_id: "machine-1".into(),
            kernel_id: "kernel-1".into(),
            desired_revision: 4,
            idle_sequence: 7,
            idle_deadline_at: "2026-09-26T00:00:00.000Z".into(),
            stop_operation_id: "stop-1".into(),
            nonce: nonce.into(),
        }
    }

    fn current_idle(
        transitions: &ManagedActivityTransitionState,
    ) -> Result<(u64, ManagedActivityObservation), DaemonError> {
        transitions.current_observation_with_sequence(0)
    }

    fn cleanup(path: &std::path::Path, store: DurableKernelStateStore) {
        drop(store);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        let _ = std::fs::remove_file(path.with_extension("managed-activity.pending.json"));
    }

    #[test]
    fn reservation_requires_report_confirmed_current_idle_transition() {
        let (path, store, transitions, gate, _) = fixture("current-idle");
        gate.confirm_activity_report(
            7,
            1,
            ManagedActivityObservation {
                running_agent_count: 0,
                changed_at_ms: 1_000,
            },
        );
        transitions
            .record_current_transition(|| (1, 1, 2_000))
            .expect("persist busy transition");
        transitions
            .record_current_transition(|| (2, 0, 3_000))
            .expect("persist new idle transition");

        assert!(!gate
            .reserve_if_current(challenge("challenge-1", "nonce-1"), || {
                current_idle(&transitions)
            })
            .expect("stale accepted sequence must return busy"));
        assert!(gate.admission_guard().is_ok(), "busy must not install a fence");
        cleanup(&path, store);
    }

    #[test]
    fn restart_requires_activity_confirmation_before_first_reservation() {
        let (path, store, transitions, gate, _) = fixture("restart-unconfirmed");
        assert!(!gate
            .reserve_if_current(challenge("challenge-1", "nonce-1"), || {
                current_idle(&transitions)
            })
            .expect("unconfirmed reporter state must return busy"));
        assert!(gate.admission_guard().is_ok());
        cleanup(&path, store);
    }

    #[test]
    fn stopped_retains_fence_and_exact_newer_keep_running_releases_it() {
        let (path, store, transitions, gate, mutation_lock) = fixture("release-sequence");
        let challenge = challenge("challenge-1", "nonce-1");
        gate.confirm_activity_report(
            challenge.idle_sequence,
            1,
            ManagedActivityObservation {
                running_agent_count: 0,
                changed_at_ms: 1_000,
            },
        );
        assert!(gate
            .reserve_if_current(challenge.clone(), || current_idle(&transitions))
            .expect("current accepted idle can be fenced"));
        let mut admission_ran = false;
        assert!(gate
            .with_open_admission(|| {
                admission_ran = true;
                Ok(())
            })
            .is_err());
        assert!(!admission_ran, "provider or queue action must not cross a fence");

        let current = || current_idle(&transitions);
        gate.apply_release(
            &challenge,
            ManagedKernelQuiescenceOutcome::Stopped,
            1,
            current,
        )
        .expect("terminal stop receipt should persist");
        assert!(gate.admission_guard().is_err(), "stopped keeps admission closed");
        drop(gate);
        let gate = ManagedKernelQuiescenceGate::restore(
            store.clone(),
            "kernel-1".into(),
            mutation_lock,
            transitions.clone(),
        )
        .expect("restore stopped receipt and retained fence");
        assert!(gate.admission_guard().is_err(), "restart cannot clear a stopped fence");
        gate.apply_release(
            &challenge,
            ManagedKernelQuiescenceOutcome::Stopped,
            1,
            || current_idle(&transitions),
        )
        .expect("exact stopped ACK replay is idempotent");
        assert!(gate
            .apply_release(
                &challenge,
                ManagedKernelQuiescenceOutcome::KeepRunning,
                1,
                || current_idle(&transitions),
            )
            .is_err(), "a conflicting sequence cannot replace the stopped decision");
        gate.apply_release(
            &challenge,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            2,
            || current_idle(&transitions),
        )
        .expect("newer exact keep-running release should persist");
        gate.apply_release(
            &challenge,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            2,
            || current_idle(&transitions),
        )
        .expect("exact keep-running ACK replay is idempotent");
        assert!(gate.admission_guard().is_ok());
        assert!(gate
            .apply_release(
                &challenge,
                ManagedKernelQuiescenceOutcome::KeepRunning,
                3,
                || current_idle(&transitions),
            )
            .is_err(), "a keep-running receipt cannot be advanced later");
        cleanup(&path, store);
    }

    #[test]
    fn reserved_keep_running_sequence_one_cancels_unclaimed_stop_and_survives_restart() {
        let (path, store, transitions, gate, mutation_lock) = fixture("reserved-cancel");
        let challenge = challenge("challenge-canceled", "nonce-canceled");
        gate.confirm_activity_report(
            challenge.idle_sequence,
            1,
            ManagedActivityObservation {
                running_agent_count: 0,
                changed_at_ms: 1_000,
            },
        );
        assert!(gate
            .reserve_if_current(challenge.clone(), || current_idle(&transitions))
            .expect("current accepted idle can be fenced"));
        assert!(gate.admission_guard().is_err(), "reservation closes admission");

        gate.apply_release(
            &challenge,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || current_idle(&transitions),
        )
        .expect("Cloud may cancel a reserved but unclaimed stop with sequence one");
        assert!(gate.admission_guard().is_ok(), "exact cancellation releases its fence");
        gate.apply_release(
            &challenge,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || current_idle(&transitions),
        )
        .expect("lost release ACK can be retried idempotently");
        assert!(!gate
            .reserve_if_current(challenge.clone(), || current_idle(&transitions))
            .expect("delayed reserve for canceled challenge stays busy"));

        drop(gate);
        let restored = ManagedKernelQuiescenceGate::restore(
            store.clone(),
            "kernel-1".into(),
            mutation_lock,
            transitions.clone(),
        )
        .expect("restore durable cancellation receipt");
        assert!(restored.admission_guard().is_ok(), "restart keeps matching fence released");
        restored
            .apply_release(
                &challenge,
                ManagedKernelQuiescenceOutcome::KeepRunning,
                1,
                || current_idle(&transitions),
            )
            .expect("exact release replay remains ACKable after restart");
        assert!(restored
            .apply_release(
                &challenge,
                ManagedKernelQuiescenceOutcome::Stopped,
                1,
                || current_idle(&transitions),
            )
            .is_err(), "conflicting outcome cannot replace durable cancellation");
        let conflicting = ManagedKernelQuiescenceChallenge {
            nonce: "nonce-conflict".into(),
            ..challenge.clone()
        };
        assert!(restored
            .apply_release(
                &conflicting,
                ManagedKernelQuiescenceOutcome::KeepRunning,
                1,
                || current_idle(&transitions),
            )
            .is_err(), "challenge identity cannot be rebound after cancellation");
        cleanup(&path, store);
    }

    #[test]
    fn durable_fence_restores_before_admission_and_lost_ack_is_idempotent() {
        let (path, store, transitions, gate, mutation_lock) = fixture("restore-fence");
        let challenge = challenge("challenge-1", "nonce-1");
        gate.confirm_activity_report(
            challenge.idle_sequence,
            1,
            ManagedActivityObservation {
                running_agent_count: 0,
                changed_at_ms: 1_000,
            },
        );
        assert!(gate
            .reserve_if_current(challenge.clone(), || current_idle(&transitions))
            .expect("reserve current idle"));
        drop(gate);

        let restored = ManagedKernelQuiescenceGate::restore(
            store.clone(),
            "kernel-1".into(),
            mutation_lock,
            transitions.clone(),
        )
        .expect("restore persisted fence");
        assert!(restored.admission_guard().is_err());
        assert!(restored
            .reserve_if_current(challenge.clone(), || current_idle(&transitions))
            .expect("lost reservation ACK retries as fenced"));
        assert!(restored.admission_guard().is_err());
        let entries = store
            .load_subject_events_by_kind("kernel-1", QUIESCENCE_EVENT_KIND, 10)
            .expect("read durable quiescence events");
        assert_eq!(entries.len(), 1, "lost ACK retry must not append a second reservation");
        cleanup(&path, store);
    }

    #[test]
    fn pre_dispatch_keep_running_tombstone_rejects_only_its_old_challenge() {
        let (path, store, transitions, gate, mutation_lock) = fixture("pre-dispatch-cancel");
        let canceled = challenge("challenge-canceled", "nonce-canceled");
        gate.confirm_activity_report(
            canceled.idle_sequence,
            1,
            ManagedActivityObservation {
                running_agent_count: 0,
                changed_at_ms: 1_000,
            },
        );
        gate.apply_release(
            &canceled,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || current_idle(&transitions),
        )
        .expect("pre-dispatch cancellation should persist a no-fence tombstone");
        gate.apply_release(
            &canceled,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || current_idle(&transitions),
        )
        .expect("exact tombstone replay is idempotent");
        assert!(gate.admission_guard().is_ok(), "tombstone must not alter open admission");
        drop(gate);
        let gate = ManagedKernelQuiescenceGate::restore(
            store.clone(),
            "kernel-1".into(),
            mutation_lock,
            transitions.clone(),
        )
        .expect("restore canceled challenge tombstone");
        gate.apply_release(
            &canceled,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || current_idle(&transitions),
        )
        .expect("exact canceled tombstone ACK remains idempotent after restart");
        assert!(gate
            .apply_release(
                &challenge("unconfirmed-challenge", "nonce-unconfirmed"),
                ManagedKernelQuiescenceOutcome::KeepRunning,
                1,
                || current_idle(&transitions),
            )
            .is_err(), "restart must not create a new tombstone before reporter confirmation");
        assert!(gate.admission_guard().is_ok(), "unconfirmed tombstone cannot affect admission");
        gate.confirm_activity_report(
            canceled.idle_sequence,
            1,
            ManagedActivityObservation {
                running_agent_count: 0,
                changed_at_ms: 1_000,
            },
        );
        let conflicting_tuple = challenge("challenge-canceled", "nonce-conflict");
        assert!(gate
            .apply_release(
                &conflicting_tuple,
                ManagedKernelQuiescenceOutcome::KeepRunning,
                1,
                || current_idle(&transitions),
            )
            .is_err(), "challenge ID cannot be rebound to another nonce tuple");
        assert!(!gate
            .reserve_if_current(canceled.clone(), || current_idle(&transitions))
            .expect("delayed reserve for canceled tuple is permanently busy"));

        let new_challenge = challenge("challenge-new", "nonce-new");
        assert!(gate
            .reserve_if_current(new_challenge.clone(), || current_idle(&transitions))
            .expect("a new challenge for the same confirmed idle state is independent"));
        assert!(gate.admission_guard().is_err());
        gate.apply_release(
            &new_challenge,
            ManagedKernelQuiescenceOutcome::Stopped,
            1,
            || current_idle(&transitions),
        )
        .expect("new challenge may record terminal stop");
        gate.apply_release(
            &new_challenge,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            2,
            || current_idle(&transitions),
        )
        .expect("new challenge may be safely released");
        assert!(!gate
            .reserve_if_current(canceled.clone(), || current_idle(&transitions))
            .expect("old canceled tuple remains stale after later challenge"));
        gate.apply_release(
            &canceled,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || current_idle(&transitions),
        )
        .expect("old exact release replay remains idempotent after newer challenge");
        assert!(gate
            .apply_release(
                &canceled,
                ManagedKernelQuiescenceOutcome::KeepRunning,
                2,
                || current_idle(&transitions),
            )
            .is_err(), "an old tombstone cannot advance its result sequence");
        cleanup(&path, store);
    }

    #[test]
    fn admission_and_reservation_share_one_linearization_boundary() {
        let (path, store, transitions, gate, mutation_lock) = fixture("admission-race");
        let challenge = challenge("challenge-1", "nonce-1");
        gate.confirm_activity_report(
            challenge.idle_sequence,
            1,
            ManagedActivityObservation {
                running_agent_count: 0,
                changed_at_ms: 1_000,
            },
        );
        {
            let _admission = gate.admission_guard().expect("idle admission is open");
            let _mutation = mutation_lock.lock().expect("activity lock");
            transitions
                .record_current_transition(|| (1, 1, 2_000))
                .expect("admitted work records its activity transition");
        }
        assert!(!gate
            .reserve_if_current(challenge, || {
                let count = 1;
                transitions.current_observation_with_sequence(count)
            })
            .expect("provider/prompt work admitted before reservation must make it busy"));
        assert!(gate.admission_guard().is_ok());
        cleanup(&path, store);
    }

    #[test]
    fn provider_store_start_launch_and_resume_seams_honor_the_durable_fence() {
        let (path, durable_store, transitions, gate, _) = fixture("provider-store-fence");
        let challenge = challenge("challenge-1", "nonce-1");
        gate.confirm_activity_report(
            challenge.idle_sequence,
            1,
            ManagedActivityObservation {
                running_agent_count: 0,
                changed_at_ms: 1_000,
            },
        );
        assert!(gate
            .reserve_if_current(challenge, || current_idle(&transitions))
            .expect("reserve current idle"));

        let providers = crate::provider::ProviderProcessServiceStore::new(
            crate::provider::ProviderProcessService::new(),
        );
        providers.set_managed_kernel_admission_gate(Some(gate));
        let request = || {
            crate::provider::LaunchProviderRequest::new(
                "session-1",
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
        };
        for error in [
            providers
                .start_run_provider_only(request())
                .err()
                .expect("provider start must be fenced"),
            providers
                .launch_run_detached(request())
                .err()
                .expect("detached provider launch must be fenced"),
            providers
                .resume_run_provider_only("session-1", "run-1")
                .err()
                .expect("provider resume must be fenced"),
            providers
                .resume_run_detached("run-1")
                .err()
                .expect("detached provider resume must be fenced"),
        ] {
            assert!(error.to_string().contains("fenced"), "unexpected error: {error}");
        }
        assert!(providers.list_runs().is_empty(), "fenced calls must not reach the provider service");
        cleanup(&path, durable_store);
    }
}

#[cfg(test)]
#[path = "managed_kernel_quiescence_state_restore_tests.rs"]
mod restore_tests;
