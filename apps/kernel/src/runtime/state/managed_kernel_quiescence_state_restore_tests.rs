use super::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_STATE: AtomicU64 = AtomicU64::new(1);

fn challenge(challenge_id: String, nonce: String) -> ManagedKernelQuiescenceChallenge {
    challenge_at(challenge_id, nonce, 4, 7)
}

fn challenge_at(
    challenge_id: String,
    nonce: String,
    desired_revision: u64,
    idle_sequence: u32,
) -> ManagedKernelQuiescenceChallenge {
    let stop_operation_id = format!("stop-{challenge_id}");
    ManagedKernelQuiescenceChallenge {
        challenge_id,
        account_id: "account-1".into(),
        environment_id: "environment-1".into(),
        machine_id: "machine-1".into(),
        kernel_id: "kernel-1".into(),
        desired_revision,
        idle_sequence,
        idle_deadline_at: "2026-09-26T00:00:00.000Z".into(),
        stop_operation_id,
        nonce,
    }
}

fn keep_running_tombstone(challenge_id: String, nonce: String) -> PersistedReservation {
    keep_running_tombstone_at(challenge_id, nonce, 4, 7)
}

fn keep_running_tombstone_at(
    challenge_id: String,
    nonce: String,
    desired_revision: u64,
    idle_sequence: u32,
) -> PersistedReservation {
    PersistedReservation {
        challenge: challenge_at(challenge_id, nonce, desired_revision, idle_sequence),
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
    let payload = serde_json::to_value(state).expect("serialize persisted quiescence state");
    persist_snapshot_payload(label, payload)
}

fn persist_legacy_snapshot(
    label: &str,
    state: &PersistedQuiescenceState,
) -> (
    PathBuf,
    DurableKernelStateStore,
    ManagedActivityTransitionState,
    Arc<Mutex<()>>,
) {
    let mut payload = serde_json::to_value(state).expect("serialize legacy quiescence state");
    let object = payload
        .as_object_mut()
        .expect("serialized quiescence state is an object");
    object.remove("schemaVersion");
    object.remove("replayFloor");
    persist_snapshot_payload(label, payload)
}

fn persist_snapshot_payload(
    label: &str,
    payload: serde_json::Value,
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
        .append_event(QUIESCENCE_EVENT_KIND, Some("kernel-1".into()), payload)
        .expect("persist quiescence snapshot");
    let transitions = ManagedActivityTransitionState::new(store.clone(), Some("kernel-1".into()));
    (path, store, transitions, Arc::new(Mutex::new(())))
}

fn restore(
    store: &DurableKernelStateStore,
    mutation_lock: &Arc<Mutex<()>>,
) -> Result<Arc<ManagedKernelQuiescenceGate>, DaemonError> {
    ManagedKernelQuiescenceGate::restore(
        store.clone(),
        "kernel-1".into(),
        Arc::clone(mutation_lock),
        )
}

fn current_idle(
    transitions: &ManagedActivityTransitionState,
) -> Result<(u64, ManagedActivityObservation), DaemonError> {
    transitions.current_observation_with_sequence(0)
}

fn assert_restore_fails_closed(
    store: &DurableKernelStateStore,
    mutation_lock: &Arc<Mutex<()>>,
) {
    let error = match restore(store, mutation_lock) {
        Ok(_) => panic!("invalid persisted quiescence state must fail restore"),
        Err(error) => error,
    };
    let gate = ManagedKernelQuiescenceGate::unavailable(
        store.clone(),
        "kernel-1".into(),
        Arc::clone(mutation_lock),
        error.to_string(),
    );
    assert!(
        gate.admission_guard().is_err(),
        "failed restore must close admission"
    );
}

fn empty_bounded_state() -> PersistedQuiescenceState {
    PersistedQuiescenceState {
        schema_version: QUIESCENCE_STATE_SCHEMA_VERSION,
        kernel_id: "kernel-1".into(),
        reservation: None,
        tombstones: Vec::new(),
        replay_floor: None,
    }
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
        schema_version: 0,
        kernel_id: "kernel-1".into(),
        reservation: None,
        tombstones: vec![
            keep_running_tombstone("challenge-replay-key-1".into(), "nonce-first".into()),
            keep_running_tombstone("challenge-replay-key-1".into(), "nonce-rebound".into()),
        ],
        replay_floor: None,
    };
    // Keep the duplicate IDs distinct in the complete challenge tuple.
    state.tombstones[1].challenge.stop_operation_id = "stop-rebound".into();
    let (path, store, transitions, mutation_lock) = persist_legacy_snapshot("duplicate", &state);

    assert_restore_fails_closed(&store, &mutation_lock);

    cleanup(&path, store, transitions);
}

#[test]
fn legacy_restore_prunes_large_tombstones_and_preserves_replay_floor_after_restart() {
    const TOMBSTONE_COUNT: usize = 8_192;
    let state = PersistedQuiescenceState {
        schema_version: 0,
        kernel_id: "kernel-1".into(),
        reservation: None,
        tombstones: (0..TOMBSTONE_COUNT)
            .map(|index| {
                keep_running_tombstone_at(
                    format!("challenge-replay-key-{index:08}"),
                    format!("nonce-{index:08}"),
                    4,
                    index as u32 + 1,
                )
            })
            .collect(),
        replay_floor: None,
    };
    let replayed_old_challenge = state.tombstones[0].challenge.clone();
    let replayed_challenge = state.tombstones[TOMBSTONE_COUNT - 1].challenge.clone();
    let (path, store, transitions, mutation_lock) = persist_legacy_snapshot("large-unique", &state);
    transitions
        .record_current_transition(|| (0, 0, 1_000))
        .expect("persist current idle observation");
    let first_gate = restore(&store, &mutation_lock)
        .expect("upgrade legacy tombstones into a bounded replay floor");
    let upgraded = first_gate
        .inner
        .lock()
        .expect("lock restored quiescence state")
        .state
        .clone();
    assert_eq!(upgraded.schema_version, QUIESCENCE_STATE_SCHEMA_VERSION);
    assert_eq!(upgraded.tombstones.len(), MAX_QUIESCENCE_TOMBSTONES);
    assert_eq!(
        upgraded.replay_floor.as_ref().unwrap().idle_sequence,
        TOMBSTONE_COUNT as u32
    );
    drop(first_gate);

    let gate = restore(&store, &mutation_lock)
        .expect("restore the persisted bounded state after restart");

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
    assert!(
        gate.apply_release(
            &rebound_challenge,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || Err(quiescence_error(
                "conflicting replay must not need activity"
            )),
        )
        .is_err(),
        "a tombstone cannot be rebound to a different challenge tuple"
    );

    gate.confirm_activity_report(
        replayed_old_challenge.idle_sequence,
        1,
        ManagedActivityObservation {
            running_agent_count: 0,
            changed_at_ms: 1_000,
        },
    );
    let mut idle_readback_ran = false;
    assert!(!gate
        .reserve_if_current(replayed_old_challenge.clone(), || {
            idle_readback_ran = true;
            Err(quiescence_error(
                "pruned challenge replay must be rejected before liveness readback",
            ))
        })
        .expect("a pruned canceled challenge is stale"));
    assert!(
        !idle_readback_ran,
        "the replay floor rejects a canceled tuple before admission checks"
    );
    assert!(
        gate.apply_release(
            &replayed_old_challenge,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || Err(quiescence_error(
                "pruned challenge release replay must be rejected"
            )),
        )
        .is_err(),
        "a pruned canceled release remains rejected after restart"
    );

    let rebound_pruned_challenge = ManagedKernelQuiescenceChallenge {
        challenge_id: "new-challenge-id-for-canceled-order".into(),
        stop_operation_id: "new-operation-for-canceled-order".into(),
        nonce: "new-nonce-for-canceled-order".into(),
        ..replayed_old_challenge.clone()
    };
    assert!(!gate
        .reserve_if_current(rebound_pruned_challenge.clone(), || {
            Err(quiescence_error(
                "a pruned order replay must be rejected before liveness readback",
            ))
        })
        .expect(
            "the replay floor rejects a rebound challenge at a canceled revision and sequence"
        ));
    assert!(
        gate.apply_release(
            &rebound_pruned_challenge,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || Err(quiescence_error(
                "a rebound canceled-order release must be rejected"
            )),
        )
        .is_err(),
        "changing the challenge ID, operation ID, and nonce cannot revive a pruned order"
    );

    let fresh_challenge = challenge_at(
        "challenge-replay-key-fresh".into(),
        "nonce-fresh".into(),
        5,
        1,
    );
    gate.confirm_activity_report(
        fresh_challenge.idle_sequence,
        1,
        ManagedActivityObservation {
            running_agent_count: 0,
            changed_at_ms: 1_000,
        },
    );
    assert!(gate
        .reserve_if_current(fresh_challenge.clone(), || current_idle(&transitions))
        .expect("a newer Cloud desired revision may reserve after restart"));
    gate.apply_release(
        &fresh_challenge,
        ManagedKernelQuiescenceOutcome::KeepRunning,
        1,
        || current_idle(&transitions),
    )
    .expect("a fresh later challenge can be canceled");
    let retained = gate
        .inner
        .lock()
        .expect("lock bounded quiescence state")
        .state
        .clone();
    assert_eq!(retained.tombstones.len(), MAX_QUIESCENCE_TOMBSTONES);
    let retained_floor = retained.replay_floor.as_ref().unwrap();
    assert_eq!(
        retained_floor.desired_revision,
        fresh_challenge.desired_revision
    );
    assert_eq!(retained_floor.idle_sequence, fresh_challenge.idle_sequence);
    let mut unrelated_operation_ran = false;
    gate.with_open_admission(|| {
        unrelated_operation_ran = true;
        Ok(())
    })
    .expect("cancellation tombstones do not fence unrelated agent operations");
    assert!(unrelated_operation_ran);

    drop(gate);
    cleanup(&path, store, transitions);
}

#[test]
fn persisted_restore_rejects_reservation_that_overlaps_a_tombstone() {
    let challenge_id = "challenge-overlap";
    let state = PersistedQuiescenceState {
        schema_version: 0,
        kernel_id: "kernel-1".into(),
        reservation: Some(reservation(challenge(
            challenge_id.into(),
            "nonce-active".into(),
        ))),
        tombstones: vec![keep_running_tombstone(
            challenge_id.into(),
            "nonce-released".into(),
        )],
        replay_floor: None,
    };
    let (path, store, transitions, mutation_lock) =
        persist_legacy_snapshot("reservation-overlap", &state);

    assert_restore_fails_closed(&store, &mutation_lock);

    cleanup(&path, store, transitions);
}

#[test]
fn legacy_restore_fails_closed_when_tombstones_cross_replay_scopes() {
    let mut other_scope = keep_running_tombstone_at(
        "challenge-other-scope".into(),
        "nonce-other-scope".into(),
        4,
        8,
    );
    other_scope.challenge.environment_id = "environment-2".into();
    let state = PersistedQuiescenceState {
        schema_version: 0,
        kernel_id: "kernel-1".into(),
        reservation: None,
        tombstones: vec![
            keep_running_tombstone_at(
                "challenge-first-scope".into(),
                "nonce-first-scope".into(),
                4,
                7,
            ),
            other_scope,
        ],
        replay_floor: None,
    };
    let (path, store, transitions, mutation_lock) =
        persist_legacy_snapshot("legacy-mixed-scope", &state);

    assert_restore_fails_closed(&store, &mutation_lock);

    cleanup(&path, store, transitions);
}

#[test]
fn legacy_restore_migrates_exact_canceled_reservation_tombstone_mirror() {
    let released = keep_running_tombstone("challenge-canceled".into(), "nonce-canceled".into());
    let state = PersistedQuiescenceState {
        schema_version: 0,
        kernel_id: "kernel-1".into(),
        // `apply_release` stores the released KeepRunning receipt in tombstones and
        // leaves the exact unfenced reservation as the current challenge record.
        reservation: Some(released.clone()),
        tombstones: vec![released.clone()],
        replay_floor: None,
    };
    let (path, store, transitions, mutation_lock) =
        persist_legacy_snapshot("exact-cancel-mirror", &state);
    let gate = restore(&store, &mutation_lock)
        .expect("migrate the exact legacy cancellation receipt written by apply_release");
    let migrated = gate
        .inner
        .lock()
        .expect("lock migrated quiescence state")
        .state
        .clone();
    assert_eq!(migrated.schema_version, QUIESCENCE_STATE_SCHEMA_VERSION);
    assert_eq!(
        migrated.replay_floor,
        Some(replay_floor_for(&released.challenge))
    );

    assert!(
        gate.admission_guard().is_ok(),
        "a keep-running mirror must stay unfenced"
    );
    gate.apply_release(
        &released.challenge,
        ManagedKernelQuiescenceOutcome::KeepRunning,
        1,
        || {
            Err(quiescence_error(
                "exact cancellation replay must not need activity",
            ))
        },
    )
    .expect("the persisted cancellation receipt remains idempotent after restart");
    let conflicting_nonce = ManagedKernelQuiescenceChallenge {
        nonce: "nonce-rebound".into(),
        ..released.challenge.clone()
    };
    assert!(
        gate.apply_release(
            &conflicting_nonce,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || Err(quiescence_error(
                "conflicting replay must not need activity"
            )),
        )
        .is_err(),
        "an exact overlap exception must not allow nonce rebinding"
    );

    drop(gate);
    cleanup(&path, store, transitions);
}

#[test]
fn live_cancellations_keep_a_stable_bound_and_reject_pruned_replay_after_restart() {
    let (path, store, transitions, mutation_lock) =
        persist_snapshot("bounded-live", &empty_bounded_state());
    transitions
        .record_current_transition(|| (0, 0, 1_000))
        .expect("persist current idle observation");
    let gate = restore(&store, &mutation_lock).expect("restore empty bounded state");
    let observation = ManagedActivityObservation {
        running_agent_count: 0,
        changed_at_ms: 1_000,
    };
    let cancellation_count = MAX_QUIESCENCE_TOMBSTONES + 12;
    let mut oldest = None;

    for sequence in 1..=cancellation_count {
        let challenge = challenge_at(
            format!("challenge-live-{sequence:04}"),
            format!("nonce-live-{sequence:04}"),
            4,
            sequence as u32,
        );
        if sequence == 1 {
            oldest = Some(challenge.clone());
        }
        gate.confirm_activity_report(challenge.idle_sequence, 1, observation);
        assert!(gate
            .reserve_if_current(challenge.clone(), || current_idle(&transitions))
            .expect("newer idle sequence can reserve after cancellation"));
        gate.apply_release(
            &challenge,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || current_idle(&transitions),
        )
        .expect("current challenge cancellation is durably recorded");
        assert!(
            gate.inner
                .lock()
                .expect("lock quiescence state")
                .state
                .tombstones
                .len()
                <= MAX_QUIESCENCE_TOMBSTONES
        );
    }
    let retained = gate
        .inner
        .lock()
        .expect("lock quiescence state")
        .state
        .clone();
    assert_eq!(retained.tombstones.len(), MAX_QUIESCENCE_TOMBSTONES);
    assert_eq!(
        retained.replay_floor.as_ref().unwrap().idle_sequence,
        cancellation_count as u32
    );
    let compacted_snapshots = store
        .load_subject_events_by_kind("kernel-1", QUIESCENCE_EVENT_KIND, 200)
        .expect("load compacted quiescence snapshots");
    assert_eq!(compacted_snapshots.len(), 1);
    assert_eq!(
        compacted_snapshots[0].payload["replayFloor"]["idleSequence"],
        cancellation_count,
    );
    drop(gate);

    let restored = restore(&store, &mutation_lock)
        .expect("restart restores the bounded tombstones and durable replay floor");
    let oldest = oldest.expect("at least one challenge was canceled");
    restored.confirm_activity_report(oldest.idle_sequence, 1, observation);
    assert!(!restored
        .reserve_if_current(oldest.clone(), || current_idle(&transitions))
        .expect("pruned challenge is rejected by the persisted replay floor"));
    assert!(
        restored
            .apply_release(
                &oldest,
                ManagedKernelQuiescenceOutcome::KeepRunning,
                1,
                || current_idle(&transitions),
            )
            .is_err(),
        "a pruned cancellation cannot be replayed after restart"
    );

    let fresh = challenge_at(
        "challenge-live-fresh".into(),
        "nonce-live-fresh".into(),
        4,
        cancellation_count as u32 + 1,
    );
    restored.confirm_activity_report(fresh.idle_sequence, 1, observation);
    assert!(restored
        .reserve_if_current(fresh.clone(), || current_idle(&transitions))
        .expect("fresh later idle sequence remains valid"));
    restored
        .apply_release(
            &fresh,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || current_idle(&transitions),
        )
        .expect("fresh later cancellation remains valid");
    assert_eq!(
        restored
            .inner
            .lock()
            .expect("lock quiescence state")
            .state
            .tombstones
            .len(),
        MAX_QUIESCENCE_TOMBSTONES,
    );

    drop(restored);
    let after_fresh_restart = restore(&store, &mutation_lock)
        .expect("restart loads the compacted snapshot with the latest replay floor");
    after_fresh_restart
        .apply_release(
            &fresh,
            ManagedKernelQuiescenceOutcome::KeepRunning,
            1,
            || {
                Err(quiescence_error(
                    "exact release retry must not need activity",
                ))
            },
        )
        .expect("exact retained release receipt remains idempotent after compaction and restart");

    let mut unrelated_agent_operation_ran = false;
    after_fresh_restart
        .with_open_admission(|| {
            unrelated_agent_operation_ran = true;
            Ok(())
        })
        .expect("released cancellations do not fence unrelated agent operations");
    assert!(unrelated_agent_operation_ran);
    drop(after_fresh_restart);
    cleanup(&path, store, transitions);
}

#[test]
fn malformed_quiescence_snapshot_does_not_prune_the_last_valid_snapshot() {
    let (path, store, transitions, mutation_lock) =
        persist_snapshot("malformed-write-preserves-history", &empty_bounded_state());
    let mut malformed = empty_bounded_state();
    malformed.kernel_id = "wrong-kernel".into();

    assert!(persist_quiescence_state(&store, "kernel-1", &malformed).is_err());
    let snapshots = store
        .load_subject_events_by_kind("kernel-1", QUIESCENCE_EVENT_KIND, 200)
        .expect("load prior valid quiescence snapshot");
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].payload["kernelId"], "kernel-1");
    drop(store);
    drop(transitions);
    let restarted_store =
        DurableKernelStateStore::open(path.clone()).expect("restart durable store");
    let restarted_transitions =
        ManagedActivityTransitionState::new(restarted_store.clone(), Some("kernel-1".into()));
    restore(&restarted_store, &mutation_lock)
        .expect("valid snapshot remains recoverable after rejected malformed write");

    cleanup(&path, restarted_store, restarted_transitions);
}

#[test]
fn malformed_bounded_state_missing_its_replay_floor_fails_closed() {
    let mut state = empty_bounded_state();
    state.tombstones.push(keep_running_tombstone_at(
        "challenge-without-floor".into(),
        "nonce-without-floor".into(),
        4,
        8,
    ));
    let (path, store, transitions, mutation_lock) = persist_snapshot("missing-floor", &state);

    assert_restore_fails_closed(&store, &mutation_lock);

    cleanup(&path, store, transitions);
}
