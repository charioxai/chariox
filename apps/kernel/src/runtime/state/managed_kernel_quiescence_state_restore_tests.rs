use super::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_STATE: AtomicU64 = AtomicU64::new(1);

fn challenge(challenge_id: String, nonce: String) -> ManagedKernelQuiescenceChallenge {
    ManagedKernelQuiescenceChallenge {
        challenge_id,
        account_id: "account-1".into(),
        environment_id: "environment-1".into(),
        machine_id: "machine-1".into(),
        kernel_id: "kernel-1".into(),
        desired_revision: 4,
        idle_sequence: 7,
        idle_deadline_at: "2026-09-26T00:00:00.000Z".into(),
        stop_operation_id: "stop-1".into(),
        nonce,
    }
}

fn keep_running_tombstone(challenge_id: String, nonce: String) -> PersistedReservation {
    PersistedReservation {
        challenge: challenge(challenge_id, nonce),
        local_idle_transition_sequence: 1,
        local_idle_changed_at_ms: 1_000,
        decisions: vec![PersistedReleaseDecision {
            outcome: ManagedKernelQuiescenceOutcome::KeepRunning,
            result_sequence: 1,
        }],
        admission_fenced: false,
    }
}

fn reservation(challenge: ManagedKernelQuiescenceChallenge) -> PersistedReservation {
    PersistedReservation {
        challenge,
        local_idle_transition_sequence: 1,
        local_idle_changed_at_ms: 1_000,
        decisions: Vec::new(),
        admission_fenced: true,
    }
}

fn persist_snapshot(
    label: &str,
    state: &PersistedQuiescenceState,
) -> (
    PathBuf,
    DurableKernelStateStore,
    ManagedActivityTransitionState,
    Arc<Mutex<()>>,
) {
    let path = std::env::temp_dir().join(format!(
        "chariox-quiescence-restore-{label}-{}-{}.sqlite",
        std::process::id(),
        NEXT_STATE.fetch_add(1, Ordering::Relaxed),
    ));
    let store = DurableKernelStateStore::open(path.clone()).expect("open durable store");
    store
        .append_event(
            QUIESCENCE_EVENT_KIND,
            Some("kernel-1".into()),
            serde_json::to_value(state).expect("serialize persisted quiescence state"),
        )
        .expect("persist quiescence snapshot");
    let transitions = ManagedActivityTransitionState::new(store.clone(), Some("kernel-1".into()));
    (path, store, transitions, Arc::new(Mutex::new(())))
}

fn restore(
    store: &DurableKernelStateStore,
    transitions: &ManagedActivityTransitionState,
    mutation_lock: &Arc<Mutex<()>>,
) -> Result<Arc<ManagedKernelQuiescenceGate>, DaemonError> {
    ManagedKernelQuiescenceGate::restore(
        store.clone(),
        "kernel-1".into(),
        Arc::clone(mutation_lock),
        transitions.clone(),
    )
}

fn assert_restore_fails_closed(
    store: &DurableKernelStateStore,
    transitions: &ManagedActivityTransitionState,
    mutation_lock: &Arc<Mutex<()>>,
) {
    let error = match restore(store, transitions, mutation_lock) {
        Ok(_) => panic!("invalid persisted quiescence state must fail restore"),
        Err(error) => error,
    };
    let gate = ManagedKernelQuiescenceGate::unavailable(
        store.clone(),
        "kernel-1".into(),
        Arc::clone(mutation_lock),
        transitions.clone(),
        error.to_string(),
    );
    assert!(gate.admission_guard().is_err(), "failed restore must close admission");
}

fn cleanup(
    path: &Path,
    store: DurableKernelStateStore,
    transitions: ManagedActivityTransitionState,
) {
    drop(transitions);
    drop(store);
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    let _ = std::fs::remove_file(path.with_extension("managed-activity.pending.json"));
}

#[test]
fn persisted_restore_rejects_duplicate_tombstone_challenge_ids_and_fails_closed() {
    let mut state = PersistedQuiescenceState {
        kernel_id: "kernel-1".into(),
        reservation: None,
        tombstones: vec![
            keep_running_tombstone("challenge-replay-key-1".into(), "nonce-first".into()),
            keep_running_tombstone("challenge-replay-key-1".into(), "nonce-rebound".into()),
        ],
    };
    // Keep the duplicate IDs distinct in the complete challenge tuple.
    state.tombstones[1].challenge.stop_operation_id = "stop-rebound".into();
    let (path, store, transitions, mutation_lock) = persist_snapshot("duplicate", &state);

    assert_restore_fails_closed(&store, &transitions, &mutation_lock);

    cleanup(&path, store, transitions);
}

#[test]
fn persisted_restore_accepts_large_unique_tombstone_state_and_keeps_exact_replay_checks() {
    const TOMBSTONE_COUNT: usize = 8_192;
    let state = PersistedQuiescenceState {
        kernel_id: "kernel-1".into(),
        reservation: None,
        tombstones: (0..TOMBSTONE_COUNT)
            .map(|index| {
                keep_running_tombstone(
                    format!("challenge-replay-key-{index:08}"),
                    format!("nonce-{index:08}"),
                )
            })
            .collect(),
    };
    let replayed_challenge = state.tombstones[TOMBSTONE_COUNT - 1].challenge.clone();
    let (path, store, transitions, mutation_lock) = persist_snapshot("large-unique", &state);
    let gate = restore(&store, &transitions, &mutation_lock).expect("restore unique tombstones");

    gate.apply_release(
        &replayed_challenge,
        ManagedKernelQuiescenceOutcome::KeepRunning,
        1,
        || Err(quiescence_error("tombstone replay must not need activity")),
    )
    .expect("the exact persisted release replay remains idempotent");
    let rebound_challenge = ManagedKernelQuiescenceChallenge {
        nonce: "nonce-rebound".into(),
        ..replayed_challenge.clone()
    };
    assert!(gate
        .apply_release(
            &rebound_challenge,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || Err(quiescence_error("conflicting replay must not need activity")),
        )
        .is_err(), "a tombstone cannot be rebound to a different challenge tuple");

    drop(gate);
    cleanup(&path, store, transitions);
}

#[test]
fn persisted_restore_rejects_reservation_that_overlaps_a_tombstone() {
    let challenge_id = "challenge-overlap";
    let state = PersistedQuiescenceState {
        kernel_id: "kernel-1".into(),
        reservation: Some(reservation(challenge(
            challenge_id.into(),
            "nonce-active".into(),
        ))),
        tombstones: vec![keep_running_tombstone(
            challenge_id.into(),
            "nonce-released".into(),
        )],
    };
    let (path, store, transitions, mutation_lock) = persist_snapshot("reservation-overlap", &state);

    assert_restore_fails_closed(&store, &transitions, &mutation_lock);

    cleanup(&path, store, transitions);
}

#[test]
fn persisted_restore_accepts_exact_canceled_reservation_tombstone_mirror() {
    let released = keep_running_tombstone("challenge-canceled".into(), "nonce-canceled".into());
    let state = PersistedQuiescenceState {
        kernel_id: "kernel-1".into(),
        // `apply_release` stores the released KeepRunning receipt in tombstones and
        // leaves the exact unfenced reservation as the current challenge record.
        reservation: Some(released.clone()),
        tombstones: vec![released.clone()],
    };
    let (path, store, transitions, mutation_lock) = persist_snapshot("exact-cancel-mirror", &state);
    let gate = restore(&store, &transitions, &mutation_lock)
        .expect("restore the exact durable cancellation receipt written by apply_release");

    assert!(gate.admission_guard().is_ok(), "a keep-running mirror must stay unfenced");
    gate.apply_release(
        &released.challenge,
        ManagedKernelQuiescenceOutcome::KeepRunning,
        1,
        || Err(quiescence_error("exact cancellation replay must not need activity")),
    )
    .expect("the persisted cancellation receipt remains idempotent after restart");
    let conflicting_nonce = ManagedKernelQuiescenceChallenge {
        nonce: "nonce-rebound".into(),
        ..released.challenge.clone()
    };
    assert!(gate
        .apply_release(
            &conflicting_nonce,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || Err(quiescence_error("conflicting replay must not need activity")),
        )
        .is_err(), "an exact overlap exception must not allow nonce rebinding");

    drop(gate);
    cleanup(&path, store, transitions);
}
