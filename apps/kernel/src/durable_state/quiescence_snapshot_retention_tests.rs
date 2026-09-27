use super::*;

fn database_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "chariox-quiescence-snapshot-{label}-{}-{}.sqlite",
        std::process::id(),
        rand_suffix(),
    ))
}

fn cleanup(path: &Path, store: DurableKernelStateStore) {
    drop(store);
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = fs::remove_file(path.with_extension("sqlite-shm"));
}

#[test]
fn quiescence_snapshot_history_is_bounded_by_exact_kind_and_kernel_after_restart() {
    let path = database_path("bounded");
    let store = DurableKernelStateStore::open(path.clone()).expect("open durable store");
    for revision in 0..100 {
        store
            .append_quiescence_snapshot(
                "kernel-1",
                serde_json::json!({"revision": revision}),
            )
            .expect("append and compact quiescence snapshot");
    }
    store
        .append_quiescence_snapshot(
            "kernel-2",
            serde_json::json!({"revision": 1}),
        )
        .expect("append another kernel snapshot");
    store
        .append_event(
            "session.updated",
            Some("kernel-1".to_string()),
            serde_json::json!({"session": "retained"}),
        )
        .expect("append session history");
    store
        .append_event(
            "provider.run.updated",
            Some("kernel-1".to_string()),
            serde_json::json!({"providerRun": "retained"}),
        )
        .expect("append provider history");
    store
        .append_event(
            "managed_kernel.auto_stop_quiescence.audit",
            Some("kernel-1".to_string()),
            serde_json::json!({"audit": "retained"}),
        )
        .expect("append unrelated quiescence audit history");

    drop(store);
    let restored = DurableKernelStateStore::open(path.clone()).expect("restart durable store");
    let kernel_one = restored
        .load_subject_events_by_kind("kernel-1", QUIESCENCE_STATE_SNAPSHOT_KIND, 200)
        .expect("load kernel one snapshot");
    assert_eq!(kernel_one.len(), 1);
    assert_eq!(kernel_one[0].payload["revision"], 99);
    assert_eq!(
        restored
            .load_subject_events_by_kind("kernel-2", QUIESCENCE_STATE_SNAPSHOT_KIND, 200)
            .expect("load kernel two snapshot")
            .len(),
        1,
    );
    for kind in [
        "session.updated",
        "provider.run.updated",
        "managed_kernel.auto_stop_quiescence.audit",
    ] {
        assert_eq!(
            restored
                .load_subject_events_by_kind("kernel-1", kind, 200)
                .expect("load unrelated event history")
                .len(),
            1,
            "targeted snapshot pruning must retain {kind}",
        );
    }
    cleanup(&path, restored);
}

#[test]
fn failed_quiescence_snapshot_prune_rolls_back_append_and_keeps_prior_snapshot() {
    let path = database_path("rollback");
    let store = DurableKernelStateStore::open(path.clone()).expect("open durable store");
    store
        .append_quiescence_snapshot(
            "kernel-rollback",
            serde_json::json!({"revision": 1}),
        )
        .expect("append original snapshot");

    let failure_connection = Connection::open(&path).expect("open failure-injection connection");
    failure_connection
        .execute_batch(
            "CREATE TRIGGER fail_quiescence_snapshot_prune
             BEFORE DELETE ON durable_state_events
             WHEN OLD.kind = 'managed_kernel.auto_stop_quiescence.changed'
               AND OLD.subject_id = 'kernel-rollback'
             BEGIN
                 SELECT RAISE(ABORT, 'injected snapshot prune failure');
             END;",
        )
        .expect("install deterministic prune failure");
    assert!(store
        .append_quiescence_snapshot(
            "kernel-rollback",
            serde_json::json!({"revision": 2}),
        )
        .is_err());
    failure_connection
        .execute_batch("DROP TRIGGER fail_quiescence_snapshot_prune;")
        .expect("remove prune failure");
    drop(failure_connection);

    drop(store);
    let restored = DurableKernelStateStore::open(path.clone()).expect("restart durable store");
    let events = restored
        .load_subject_events_by_kind("kernel-rollback", QUIESCENCE_STATE_SNAPSHOT_KIND, 200)
        .expect("load recoverable snapshot");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["revision"], 1);
    cleanup(&path, restored);
}
