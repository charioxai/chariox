use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;

const MANAGED_ACTIVITY_EVENT_KIND: &str = "managed_kernel.activity.changed";
const MAX_PENDING_ACTIVITY_TRANSITIONS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedActivityObservation {
    pub(crate) running_agent_count: u8,
    pub(crate) changed_at_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedManagedActivityTransition {
    kernel_id: String,
    running_agent_count: u8,
    activity_changed_at_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingManagedActivityTransitions {
    kernel_id: String,
    observations: VecDeque<ManagedActivityObservation>,
}

#[derive(Debug, Default)]
struct ManagedActivityTransitionInner {
    restored: bool,
    latest_durable: Option<ManagedActivityObservation>,
    pending: VecDeque<ManagedActivityObservation>,
    pending_journal_dirty: bool,
    last_runtime_sequence: u64,
}

#[derive(Debug, Clone)]
pub(super) struct ManagedActivityTransitionState {
    store: DurableKernelStateStore,
    kernel_id: Arc<Mutex<Option<String>>>,
    inner: Arc<Mutex<ManagedActivityTransitionInner>>,
}

impl ManagedActivityTransitionState {
    pub(super) fn new(store: DurableKernelStateStore, kernel_id: Option<String>) -> Self {
        Self {
            store,
            kernel_id: Arc::new(Mutex::new(kernel_id)),
            inner: Arc::new(Mutex::new(ManagedActivityTransitionInner::default())),
        }
    }

    pub(super) fn is_enabled(&self) -> bool {
        self.kernel_id
            .lock()
            .expect("managed activity kernel identity mutex poisoned")
            .is_some()
    }

    pub(super) fn enable_before_activity(
        &self,
        kernel_id: &str,
        runtime_sequence: u64,
    ) -> Result<(), DaemonError> {
        let mut identity = self
            .kernel_id
            .lock()
            .expect("managed activity kernel identity mutex poisoned");
        match identity.as_deref() {
            Some(current) if current == kernel_id => Ok(()),
            Some(_) => Err(activity_state_error(
                "managed activity identity changed after runtime construction",
            )),
            None if runtime_sequence == 0 => {
                *identity = Some(kernel_id.to_string());
                Ok(())
            }
            None => Err(activity_state_error(
                "managed activity tracking cannot activate after runtime activity changed",
            )),
        }
    }

    #[cfg(test)]
    fn record_transition(
        &self,
        runtime_sequence: u64,
        running_agent_count: u8,
        changed_at_ms: u64,
    ) -> Result<Option<ManagedActivityObservation>, DaemonError> {
        self.record_current_transition(|| (runtime_sequence, running_agent_count, changed_at_ms))
    }

    pub(super) fn record_current_transition(
        &self,
        sample: impl FnOnce() -> (u64, u8, u64),
    ) -> Result<Option<ManagedActivityObservation>, DaemonError> {
        let Some(kernel_id) = self.kernel_id() else {
            return Ok(None);
        };
        let mut inner = self
            .inner
            .lock()
            .expect("managed activity transition mutex poisoned");
        // Sample while holding the same lock as persistence. Sampling first permits
        // a delayed caller to append stale busy state after a newer idle transition,
        // even at the same projection sequence.
        let (runtime_sequence, running_agent_count, changed_at_ms) = sample();
        if running_agent_count > 1 {
            return Err(activity_state_error(
                "managed activity running-agent count is not binary",
            ));
        }
        self.restore_locked(&kernel_id, &mut inner)?;
        if runtime_sequence < inner.last_runtime_sequence {
            return Ok(inner.pending.back().copied().or(inner.latest_durable));
        }
        inner.last_runtime_sequence = runtime_sequence;
        if inner
            .pending
            .back()
            .copied()
            .or(inner.latest_durable)
            .is_some_and(|latest| latest.running_agent_count == running_agent_count)
        {
            self.flush_pending_locked(&kernel_id, &mut inner)?;
            return Ok(inner.pending.back().copied().or(inner.latest_durable));
        }
        if inner.pending.len() >= MAX_PENDING_ACTIVITY_TRANSITIONS {
            return Err(activity_state_error(format!(
                "managed activity pending transition limit {MAX_PENDING_ACTIVITY_TRANSITIONS} is exhausted"
            )));
        }
        inner.pending.push_back(ManagedActivityObservation {
            running_agent_count,
            changed_at_ms,
        });
        inner.pending_journal_dirty = true;
        self.flush_pending_locked(&kernel_id, &mut inner)?;
        Ok(inner.latest_durable)
    }

    pub(super) fn current_observation(
        &self,
        running_agent_count: u8,
    ) -> Result<ManagedActivityObservation, DaemonError> {
        let kernel_id = self.kernel_id().ok_or_else(|| {
            activity_state_error("managed activity tracking is not enabled for this kernel")
        })?;
        let mut inner = self
            .inner
            .lock()
            .expect("managed activity transition mutex poisoned");
        self.restore_locked(&kernel_id, &mut inner)?;
        self.flush_pending_locked(&kernel_id, &mut inner)?;
        let observation = inner.latest_durable.ok_or_else(|| {
            activity_state_error("managed activity has no durable initial observation")
        })?;
        if observation.running_agent_count != running_agent_count {
            return Err(activity_state_error(
                "managed activity changed without a durable mutation-boundary transition",
            ));
        }
        Ok(observation)
    }

    fn kernel_id(&self) -> Option<String> {
        self.kernel_id
            .lock()
            .expect("managed activity kernel identity mutex poisoned")
            .clone()
    }

    fn restore_locked(
        &self,
        kernel_id: &str,
        inner: &mut ManagedActivityTransitionInner,
    ) -> Result<(), DaemonError> {
        if inner.restored {
            return Ok(());
        }
        let latest = self
            .store
            .load_subject_events_by_kind(kernel_id, MANAGED_ACTIVITY_EVENT_KIND, 1)?
            .pop()
            .map(|event| {
                let persisted =
                    serde_json::from_value::<PersistedManagedActivityTransition>(event.payload)
                        .map_err(|error| {
                            activity_state_error(format!(
                                "could not decode managed activity transition: {error}"
                            ))
                        })?;
                if persisted.kernel_id != kernel_id
                    || persisted.running_agent_count > 1
                    || persisted.activity_changed_at_ms == 0
                {
                    return Err(activity_state_error(
                        "stored managed activity transition is invalid",
                    ));
                }
                Ok(ManagedActivityObservation {
                    running_agent_count: persisted.running_agent_count,
                    changed_at_ms: persisted.activity_changed_at_ms,
                })
            })
            .transpose()?;
        let mut pending = match std::fs::read(self.pending_journal_path()) {
            Ok(bytes) => {
                let journal: PendingManagedActivityTransitions = serde_json::from_slice(&bytes)
                    .map_err(|error| {
                        activity_state_error(format!("invalid activity journal: {error}"))
                    })?;
                if journal.kernel_id != kernel_id
                    || journal.observations.len() > MAX_PENDING_ACTIVITY_TRANSITIONS
                    || journal.observations.iter().any(|observation| {
                        observation.running_agent_count > 1 || observation.changed_at_ms == 0
                    })
                {
                    return Err(activity_state_error(
                        "activity journal identity or observations are invalid",
                    ));
                }
                journal.observations
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => VecDeque::new(),
            Err(error) => {
                return Err(activity_state_error(format!(
                    "could not read activity journal: {error}"
                )))
            }
        };
        // A crash can occur after the database commits but before the pending journal
        // is cleared. Trim that acknowledged prefix instead of replaying older clocks.
        inner.pending_journal_dirty = !pending.is_empty();
        if let Some(index) =
            latest.and_then(|latest| pending.iter().rposition(|item| *item == latest))
        {
            pending.drain(..=index);
        }
        inner.pending = pending;
        inner.latest_durable = latest;
        inner.restored = true;
        Ok(())
    }

    fn pending_journal_path(&self) -> std::path::PathBuf {
        self.store
            .path()
            .with_extension("managed-activity.pending.json")
    }

    fn persist_pending_locked(
        &self,
        kernel_id: &str,
        inner: &mut ManagedActivityTransitionInner,
    ) -> Result<(), DaemonError> {
        if !inner.pending_journal_dirty {
            return Ok(());
        }
        let payload = serde_json::to_vec(&PendingManagedActivityTransitions {
            kernel_id: kernel_id.to_string(),
            observations: inner.pending.clone(),
        })
        .map_err(|error| {
            activity_state_error(format!("could not encode activity journal: {error}"))
        })?;
        // The existing private-file helper fsyncs both the atomic replacement and
        // parent directory. This kernel-owned outbox retains T0 when SQLite rejects
        // an append and the process exits before an in-memory retry succeeds.
        crate::config::write_private_file(&self.pending_journal_path(), &payload).map_err(
            |error| activity_state_error(format!("could not persist activity journal: {error}")),
        )?;
        inner.pending_journal_dirty = false;
        Ok(())
    }

    fn flush_pending_locked(
        &self,
        kernel_id: &str,
        inner: &mut ManagedActivityTransitionInner,
    ) -> Result<(), DaemonError> {
        self.persist_pending_locked(kernel_id, inner)?;
        while let Some(observation) = inner.pending.front().copied() {
            self.store.append_event(
                MANAGED_ACTIVITY_EVENT_KIND,
                Some(kernel_id.to_string()),
                serde_json::to_value(PersistedManagedActivityTransition {
                    kernel_id: kernel_id.to_string(),
                    running_agent_count: observation.running_agent_count,
                    activity_changed_at_ms: observation.changed_at_ms,
                })
                .map_err(|error| {
                    activity_state_error(format!(
                        "could not encode managed activity transition: {error}"
                    ))
                })?,
            )?;
            inner.latest_durable = Some(observation);
            inner.pending.pop_front();
            inner.pending_journal_dirty = true;
        }
        self.persist_pending_locked(kernel_id, inner)
    }
}

fn activity_state_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "persist managed kernel activity transition",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_observation_is_sampled_under_transition_lock() {
        let state_path = test_state_path("serialized-sample");
        let store = DurableKernelStateStore::open(state_path.clone()).expect("open store");
        let state =
            ManagedActivityTransitionState::new(store.clone(), Some("kernel-sample".into()));
        state
            .record_current_transition(|| {
                assert!(
                    state.inner.try_lock().is_err(),
                    "sample must not precede the persistence lock"
                );
                (0, 1, 1_000)
            })
            .expect("busy sample");
        state
            .record_current_transition(|| (0, 0, 2_000))
            .expect("same-sequence idle sample");
        assert_eq!(
            state
                .current_observation(0)
                .expect("latest idle")
                .changed_at_ms,
            2_000
        );
        drop(state);
        drop(store);
        remove_test_state(&state_path);
    }

    #[test]
    fn rapid_busy_idle_cycle_is_durable_before_reporter_polling() {
        let state_path = test_state_path("rapid-cycle");
        let store = DurableKernelStateStore::open(state_path.clone()).expect("open state store");
        let state =
            ManagedActivityTransitionState::new(store.clone(), Some("kernel-rapid".to_string()));

        assert_eq!(
            state.record_transition(0, 0, 1_000).expect("baseline"),
            Some(ManagedActivityObservation {
                running_agent_count: 0,
                changed_at_ms: 1_000,
            })
        );
        state
            .record_transition(1, 1, 2_000)
            .expect("busy transition");
        state
            .record_transition(2, 0, 3_000)
            .expect("idle transition");

        let events = store
            .load_subject_events_by_kind("kernel-rapid", MANAGED_ACTIVITY_EVENT_KIND, 10)
            .expect("load activity transitions");
        assert_eq!(events.len(), 3);
        assert_eq!(events[1].payload["activityChangedAtMs"], 2_000);
        assert_eq!(events[2].payload["activityChangedAtMs"], 3_000);
        assert_eq!(
            state.current_observation(0).expect("current idle state"),
            ManagedActivityObservation {
                running_agent_count: 0,
                changed_at_ms: 3_000,
            }
        );
        drop(state);
        drop(store);
        remove_test_state(&state_path);
    }

    #[test]
    fn failed_activity_append_preserves_idle_timestamp_across_restart() {
        let state_path = test_state_path("failed-append-restart");
        let store = DurableKernelStateStore::open(state_path.clone()).expect("open store");
        let state = ManagedActivityTransitionState::new(
            store.clone(),
            Some("kernel-failed-append".to_string()),
        );
        state.record_transition(0, 0, 1_000).expect("initial idle");
        let database = rusqlite::Connection::open(&state_path).expect("open failure injector");
        database
            .execute_batch(
                "CREATE TRIGGER fail_activity BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'managed_kernel.activity.changed'
             BEGIN SELECT RAISE(FAIL, 'injected activity append failure'); END;",
            )
            .expect("install failure");
        state
            .record_transition(1, 1, 2_000)
            .expect_err("busy append fails");
        state
            .record_transition(2, 0, 3_000)
            .expect_err("idle append fails");
        drop(state);
        drop(store);
        database
            .execute_batch("DROP TRIGGER fail_activity;")
            .expect("recover storage");
        drop(database);

        let store = DurableKernelStateStore::open(state_path.clone()).expect("reopen store");
        let state = ManagedActivityTransitionState::new(
            store.clone(),
            Some("kernel-failed-append".to_string()),
        );
        assert_eq!(
            state.current_observation(0).expect("recover original idle"),
            ManagedActivityObservation {
                running_agent_count: 0,
                changed_at_ms: 3_000
            }
        );
        let events = store
            .load_subject_events_by_kind("kernel-failed-append", MANAGED_ACTIVITY_EVENT_KIND, 10)
            .expect("read transitions");
        assert_eq!(
            events.len(),
            3,
            "retain both transitions from the busy/idle cycle"
        );
        drop(state);
        drop(store);
        remove_test_state(&state_path);
    }

    #[test]
    fn restart_trims_committed_journal_prefix_without_replaying_older_activity() {
        let state_path = test_state_path("journal-prefix");
        let store = DurableKernelStateStore::open(state_path.clone()).expect("open store");
        let state =
            ManagedActivityTransitionState::new(store.clone(), Some("kernel-prefix".into()));
        state.record_transition(0, 0, 1_000).expect("idle");
        state.record_transition(1, 1, 2_000).expect("busy");
        let pending = PendingManagedActivityTransitions {
            kernel_id: "kernel-prefix".into(),
            observations: VecDeque::from([
                ManagedActivityObservation {
                    running_agent_count: 0,
                    changed_at_ms: 1_000,
                },
                ManagedActivityObservation {
                    running_agent_count: 1,
                    changed_at_ms: 2_000,
                },
                ManagedActivityObservation {
                    running_agent_count: 0,
                    changed_at_ms: 3_000,
                },
            ]),
        };
        crate::config::write_private_file(
            &state.pending_journal_path(),
            &serde_json::to_vec(&pending).unwrap(),
        )
        .expect("model crash after partial database commit");
        drop(state);
        let restored =
            ManagedActivityTransitionState::new(store.clone(), Some("kernel-prefix".into()));
        assert_eq!(
            restored
                .current_observation(0)
                .expect("recovered final idle")
                .changed_at_ms,
            3_000
        );
        assert_eq!(
            store
                .load_subject_events_by_kind("kernel-prefix", MANAGED_ACTIVITY_EVENT_KIND, 10)
                .expect("read persisted events")
                .len(),
            3,
            "do not duplicate committed transitions"
        );
        drop(restored);
        drop(store);
        remove_test_state(&state_path);
    }

    #[test]
    fn activity_journal_rejects_foreign_kernel_identity() {
        let state_path = test_state_path("foreign-journal");
        let store = DurableKernelStateStore::open(state_path.clone()).expect("open store");
        let state = ManagedActivityTransitionState::new(store.clone(), Some("kernel-owner".into()));
        crate::config::write_private_file(
            &state.pending_journal_path(),
            br#"{"kernelId":"foreign","observations":[]}"#,
        )
        .expect("write foreign journal");
        assert!(state
            .current_observation(0)
            .expect_err("reject foreign journal")
            .to_string()
            .contains("identity"));
        drop(state);
        drop(store);
        remove_test_state(&state_path);
    }

    #[test]
    fn restart_does_not_confuse_same_millisecond_activity_transitions() {
        let state_path = test_state_path("journal-aba");
        let store = DurableKernelStateStore::open(state_path.clone()).expect("open store");
        let state = ManagedActivityTransitionState::new(store.clone(), Some("kernel-aba".into()));
        let database = rusqlite::Connection::open(&state_path).expect("failure injector");
        database
            .execute_batch(
                "CREATE TRIGGER fail_activity BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'managed_kernel.activity.changed'
             BEGIN SELECT RAISE(FAIL, 'injected activity append failure'); END;",
            )
            .expect("install failure");
        for (sequence, count) in [(0, 0), (1, 1), (2, 0)] {
            state
                .record_transition(sequence, count, 1_000)
                .expect_err("journal survives failed database append");
        }
        let journal_bytes = std::fs::read(state.pending_journal_path()).expect("saved journal");
        database
            .execute_batch("DROP TRIGGER fail_activity;")
            .expect("recover storage");
        database.execute_batch(
            "CREATE TRIGGER fail_after_first BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'managed_kernel.activity.changed'
               AND EXISTS (SELECT 1 FROM durable_state_events WHERE kind = 'managed_kernel.activity.changed')
             BEGIN SELECT RAISE(FAIL, 'partial activity append failure'); END;",
        ).expect("stop after first commit");
        state
            .current_observation(0)
            .expect_err("only first transition commits");
        // Model the crash window before the pending journal acknowledges that commit.
        assert_eq!(
            std::fs::read(state.pending_journal_path()).unwrap(),
            journal_bytes
        );
        drop(state);
        database
            .execute_batch("DROP TRIGGER fail_after_first;")
            .expect("recover storage");
        drop(database);
        let restored =
            ManagedActivityTransitionState::new(store.clone(), Some("kernel-aba".into()));
        restored.current_observation(0).expect("recover final idle");
        let events = store
            .load_subject_events_by_kind("kernel-aba", MANAGED_ACTIVITY_EVENT_KIND, 10)
            .expect("read transitions");
        assert_eq!(
            events
                .iter()
                .map(|event| event.payload["runningAgentCount"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![0, 1, 0],
            "equal observation values must not acknowledge uncommitted transitions"
        );
        drop(restored);
        drop(store);
        remove_test_state(&state_path);
    }

    #[test]
    fn idle_transition_survives_reporter_and_kernel_state_restart() {
        let state_path = test_state_path("restart");
        {
            let store =
                DurableKernelStateStore::open(state_path.clone()).expect("open first state store");
            let state =
                ManagedActivityTransitionState::new(store, Some("kernel-restart".to_string()));
            state.record_transition(0, 1, 10_000).expect("busy");
            state.record_transition(1, 0, 20_000).expect("idle");
            assert_eq!(
                state.current_observation(0).expect("reporter restart"),
                ManagedActivityObservation {
                    running_agent_count: 0,
                    changed_at_ms: 20_000,
                }
            );
        }
        {
            let store = DurableKernelStateStore::open(state_path.clone())
                .expect("reopen state store after kernel restart");
            let restored =
                ManagedActivityTransitionState::new(store, Some("kernel-restart".to_string()));
            assert_eq!(
                restored.current_observation(0).expect("restored idle"),
                ManagedActivityObservation {
                    running_agent_count: 0,
                    changed_at_ms: 20_000,
                }
            );
        }
        remove_test_state(&state_path);
    }

    #[test]
    fn ordinary_kernel_activity_does_not_touch_durable_state() {
        let state_path = test_state_path("ordinary");
        let store = DurableKernelStateStore::open(state_path.clone()).expect("open state store");
        let state = ManagedActivityTransitionState::new(store.clone(), None);
        assert_eq!(state.record_transition(0, 1, 1_000).expect("no-op"), None);
        assert!(store
            .load_events_by_kind(MANAGED_ACTIVITY_EVENT_KIND)
            .expect("load ordinary events")
            .is_empty());
        drop(state);
        drop(store);
        remove_test_state(&state_path);
    }

    #[test]
    fn late_activation_fails_instead_of_inventing_a_transition_time() {
        let state_path = test_state_path("late-activation");
        let store = DurableKernelStateStore::open(state_path.clone()).expect("open state store");
        let state = ManagedActivityTransitionState::new(store.clone(), None);
        let error = state
            .enable_before_activity("kernel-late", 1)
            .expect_err("late activation must fail");
        assert!(error
            .to_string()
            .contains("cannot activate after runtime activity changed"));
        assert!(store
            .load_events_by_kind(MANAGED_ACTIVITY_EVENT_KIND)
            .expect("load late activation events")
            .is_empty());
        drop(state);
        drop(store);
        remove_test_state(&state_path);
    }

    fn test_state_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join("chariox-tests").join(format!(
            "managed-activity-{name}-{}-{}.db",
            std::process::id(),
            crate::session::unix_epoch_ms(),
        ))
    }

    fn remove_test_state(path: &std::path::Path) {
        let _ = std::fs::remove_file(path.with_extension("managed-activity.pending.json"));
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    }
}
