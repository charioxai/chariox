use chariox_app_runtime::{
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationError, InstallationRegistry,
        ReleaseMetadata,
    },
    managed_state::{
        complete_wake, defer_wake, due_wakes, next_wake_at_ms, postpone_wake, ManagedStateStore, StateChanges,
        StateCheck, StateError, StateScope, StateWrite, Wake, WakeChange, WakeFailureOutcome,
        MAX_CHANGES, MAX_KEYS, MAX_REVISION, MAX_STATE_BYTES, MAX_VALUE_BYTES, MAX_WAKES,
    },
};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Database(PathBuf);
impl Database {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "chariox-managed-state-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> Connection {
        let mut db = Connection::open(self.0.join("kernel.sqlite")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .unwrap();
        InstallationRegistry::new(&mut db).initialize().unwrap();
        ManagedStateStore::new(&mut db).initialize().unwrap();
        db
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn release(version: u32) -> ReleaseMetadata {
    ReleaseMetadata {
        app_id: "com.example.state".into(),
        version: format!("{version}.0.0"),
        publisher_id: "com.example".into(),
        package_digest: format!("sha256:{version:064x}"),
        schema_version: version,
        capabilities_digest: format!("sha256:{:064x}", 10),
        catalog_digest: format!("sha256:{:064x}", 11),
        view_digest: format!("sha256:{:064x}", 12),
    }
}
fn approval() -> CapabilityDecision {
    CapabilityDecision::Approved {
        approval: CapabilityApproval {
            decision_id: "fixture".into(),
            authority_ref: "test-policy".into(),
        },
    }
}
fn install(db: &mut Connection, id: &str) {
    // Metadata-only fixtures exercise SQL behavior, not package/runtime approval.
    let mut registry = InstallationRegistry::new(db);
    let token = registry
        .create_and_stage(id, "alice", release(1), 1)
        .unwrap()
        .token;
    registry.decide(&token, approval(), 2).unwrap();
    registry.quiesce(&token, 3).unwrap();
    registry.mark_prepared(&token, 4).unwrap();
    registry.commit(&token, 5).unwrap();
}
fn scope(id: &str) -> StateScope<'_> {
    StateScope::new("alice", id, 1).unwrap()
}
fn put(key: &str, value: Value) -> StateWrite {
    StateWrite::Put {
        key: key.into(),
        value,
    }
}
fn change(checks: Vec<StateCheck>, writes: Vec<StateWrite>) -> StateChanges {
    StateChanges::new(1, checks, writes).unwrap()
}
fn check(key: &str, version: Option<u64>) -> StateCheck {
    StateCheck {
        key: key.into(),
        version,
    }
}

#[test]
fn concurrent_readers_have_one_cas_winner_and_delete_recreate_does_not_reuse_versions() {
    let files = Database::new();
    let mut first = files.open();
    install(&mut first, "one");
    install(&mut first, "two");
    let mut second = files.open();
    for db in [&mut first, &mut second] {
        assert!(ManagedStateStore::new(db)
            .get(scope("one"), "item")
            .unwrap()
            .is_none());
    }
    let proposed = change(
        vec![check("item", None)],
        vec![put("item", json!({"done":false}))],
    );
    assert_eq!(
        ManagedStateStore::new(&mut first)
            .transaction(scope("one"), &proposed)
            .unwrap(),
        1
    );
    assert!(matches!(
        ManagedStateStore::new(&mut second).transaction(scope("one"), &proposed),
        Err(StateError::Conflict)
    ));
    assert!(ManagedStateStore::new(&mut second)
        .get(scope("two"), "item")
        .unwrap()
        .is_none());
    assert!(matches!(
        ManagedStateStore::new(&mut second).get(StateScope::new("bob", "one", 1).unwrap(), "item"),
        Err(StateError::Installation(InstallationError::NotFound))
    ));
    let deleted = change(
        vec![check("item", Some(1))],
        vec![StateWrite::Delete { key: "item".into() }],
    );
    assert_eq!(
        ManagedStateStore::new(&mut first)
            .transaction(scope("one"), &deleted)
            .unwrap(),
        2
    );
    assert_eq!(
        ManagedStateStore::new(&mut second)
            .transaction(scope("one"), &proposed)
            .unwrap(),
        3
    );
    assert!(matches!(
        ManagedStateStore::new(&mut first).transaction(scope("one"), &deleted),
        Err(StateError::Conflict)
    ));
    drop(first);
    drop(second);
    let mut reopened = files.open();
    let record = ManagedStateStore::new(&mut reopened)
        .get(scope("one"), "item")
        .unwrap()
        .unwrap();
    assert_eq!(record.version, 3);
    assert_eq!(record.value, json!({"done":false}));
}

#[test]
fn state_and_encompassing_work_roll_back_together_and_savepoint_prevents_partial_state() {
    let files = Database::new();
    let mut db = files.open();
    install(&mut db, "one");
    db.execute_batch("CREATE TABLE fixture_outbox(id TEXT); CREATE TRIGGER fail_state BEFORE INSERT ON app_state_values WHEN NEW.key='z' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    let both = change(vec![], vec![put("a", json!(1)), put("z", json!(2))]);
    {
        let mut transaction = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        transaction
            .execute("INSERT INTO fixture_outbox VALUES('unrelated')", [])
            .unwrap();
        assert!(matches!(
            ManagedStateStore::apply_in(&mut transaction, scope("one"), &both),
            Err(StateError::Database(_))
        ));
        // Even if a trusted encompassing caller deliberately commits unrelated
        // work, the failed state's first write and revision cannot leak through.
        transaction.commit().unwrap();
    }
    assert!(ManagedStateStore::new(&mut db)
        .get(scope("one"), "a")
        .unwrap()
        .is_none());
    db.execute_batch("DROP TRIGGER fail_state;").unwrap();
    {
        let mut transaction = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            ManagedStateStore::apply_in(&mut transaction, scope("one"), &both).unwrap(),
            1
        );
        transaction
            .execute("INSERT INTO fixture_outbox VALUES('correlated')", [])
            .unwrap();
        // Simulated failure after state and outbox writes, before outer commit.
    }
    drop(db);
    let mut db = files.open();
    assert!(ManagedStateStore::new(&mut db)
        .get(scope("one"), "a")
        .unwrap()
        .is_none());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM fixture_outbox", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        ManagedStateStore::new(&mut db)
            .transaction(scope("one"), &both)
            .unwrap(),
        1
    );
}

#[test]
fn quiescence_and_generation_schema_changes_fence_structured_writers() {
    let files = Database::new();
    let mut db = files.open();
    install(&mut db, "one");
    let mutation = change(vec![], vec![put("a", json!(1))]);
    ManagedStateStore::new(&mut db)
        .transaction(scope("one"), &mutation)
        .unwrap();
    let mut registry = InstallationRegistry::new(&mut db);
    let token = registry.stage("one", 1, release(2), 10).unwrap().token;
    registry.decide(&token, approval(), 11).unwrap();
    registry.quiesce(&token, 12).unwrap();
    assert!(matches!(
        ManagedStateStore::new(&mut db).transaction(scope("one"), &mutation),
        Err(StateError::Installation(InstallationError::AdmissionPaused))
    ));
    assert!(matches!(
        ManagedStateStore::new(&mut db).get(scope("one"), "a"),
        Err(StateError::Installation(InstallationError::AdmissionPaused))
    ));
    let mut registry = InstallationRegistry::new(&mut db);
    registry.mark_prepared(&token, 13).unwrap();
    registry.commit(&token, 14).unwrap();
    assert!(matches!(
        ManagedStateStore::new(&mut db).transaction(scope("one"), &mutation),
        Err(StateError::Installation(InstallationError::Conflict))
    ));
    let current = StateScope::new("alice", "one", 2).unwrap();
    assert!(matches!(
        ManagedStateStore::new(&mut db).transaction(current, &mutation),
        Err(StateError::SchemaMismatch)
    ));
    let valid = StateChanges::new(2, vec![check("a", Some(1))], vec![put("a", json!(2))]).unwrap();
    assert_eq!(
        ManagedStateStore::new(&mut db)
            .transaction(current, &valid)
            .unwrap(),
        2
    );
}

#[test]
fn exact_key_ceiling_allows_order_independent_replacement_and_preserves_overflow_state() {
    let files = Database::new();
    let mut db = files.open();
    install(&mut db, "one");
    for start in (0..MAX_KEYS).step_by(MAX_CHANGES) {
        let writes = (start..start + MAX_CHANGES)
            .map(|i| put(&format!("k{i}"), Value::Null))
            .collect();
        ManagedStateStore::new(&mut db)
            .transaction(scope("one"), &change(vec![], writes))
            .unwrap();
    }
    assert!(matches!(
        ManagedStateStore::new(&mut db).transaction(
            scope("one"),
            &change(vec![], vec![put("extra", Value::Null)])
        ),
        Err(StateError::Limit)
    ));
    let replacement = change(
        vec![],
        vec![
            put("extra", Value::Null),
            StateWrite::Delete { key: "k0".into() },
        ],
    );
    let revision = ManagedStateStore::new(&mut db)
        .transaction(scope("one"), &replacement)
        .unwrap();
    assert_eq!(revision, 65);
    assert!(ManagedStateStore::new(&mut db)
        .get(scope("one"), "k0")
        .unwrap()
        .is_none());
    db.execute(
        "UPDATE app_state_heads SET revision=?1",
        [MAX_REVISION as i64],
    )
    .unwrap();
    assert!(matches!(
        ManagedStateStore::new(&mut db).transaction(
            scope("one"),
            &change(
                vec![],
                vec![StateWrite::Delete {
                    key: "extra".into()
                }]
            )
        ),
        Err(StateError::Limit)
    ));
    assert!(ManagedStateStore::new(&mut db)
        .get(scope("one"), "extra")
        .unwrap()
        .is_some());
}

#[test]
fn actual_payload_byte_ceiling_is_exact_and_oversized_replacement_keeps_old_value() {
    let files = Database::new();
    let mut db = files.open();
    install(&mut db, "one");
    let mut used = 0;
    for index in 0..64 {
        let key = format!("k{index:02}");
        let size = MAX_VALUE_BYTES.min(MAX_STATE_BYTES - used - key.len());
        let value = Value::String("x".repeat(size - 2));
        ManagedStateStore::new(&mut db)
            .transaction(scope("one"), &change(vec![], vec![put(&key, value)]))
            .unwrap();
        used += key.len() + size;
    }
    assert_eq!(used, MAX_STATE_BYTES);
    let previous = ManagedStateStore::new(&mut db)
        .get(scope("one"), "k63")
        .unwrap()
        .unwrap();
    let larger = Value::String("x".repeat(MAX_VALUE_BYTES - 2));
    assert!(matches!(
        ManagedStateStore::new(&mut db)
            .transaction(scope("one"), &change(vec![], vec![put("k63", larger)])),
        Err(StateError::Limit)
    ));
    assert_eq!(
        ManagedStateStore::new(&mut db)
            .get(scope("one"), "k63")
            .unwrap()
            .unwrap(),
        previous
    );
}

#[test]
fn change_validation_bounds_values_and_rejects_ambiguous_duplicate_keys() {
    assert!(matches!(
        StateChanges::new(1, vec![check("a", Some(0))], vec![]),
        Err(StateError::Invalid)
    ));
    assert!(matches!(
        StateChanges::new(1, vec![check("a", None), check("a", None)], vec![]),
        Err(StateError::Invalid)
    ));
    assert!(matches!(
        StateChanges::new(
            1,
            vec![],
            vec![
                put("a", Value::Null),
                StateWrite::Delete { key: "a".into() }
            ]
        ),
        Err(StateError::Invalid)
    ));
    assert!(matches!(
        StateChanges::new(
            1,
            vec![],
            vec![put("a", Value::String("x".repeat(MAX_VALUE_BYTES)))]
        ),
        Err(StateError::Limit)
    ));
    let mut nested = Value::Null;
    for _ in 0..34 {
        nested = json!([nested]);
    }
    assert!(matches!(
        StateChanges::new(1, vec![], vec![put("a", nested)]),
        Err(StateError::Limit)
    ));
    assert!(StateChanges::new(
        1,
        vec![],
        vec![put("合法的 key", json!({"$ref":"ordinary data"}))]
    )
    .is_ok());
}

fn wake(id: &str, due_at_ms: u64, revision: &str) -> WakeChange {
    WakeChange::Set(Wake {
        id: id.into(),
        due_at_ms,
        revision: revision.into(),
    })
}
fn apply_wakes(db: &mut Connection, id: &str, changes: &[WakeChange]) -> Result<(), StateError> {
    arm(db, id, changes, false)
}
fn arm(
    db: &mut Connection,
    id: &str,
    changes: &[WakeChange],
    counts_as_use: bool,
) -> Result<(), StateError> {
    let mut tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ManagedStateStore::apply_wakes_in(&mut tx, scope(id), changes, counts_as_use)?;
    tx.commit()?;
    Ok(())
}
fn origins(db: &Connection, now_ms: u64) -> Vec<(String, bool)> {
    due_wakes(db, now_ms, 8)
        .unwrap()
        .into_iter()
        .map(|due| (due.wake.id, due.counts_as_use))
        .collect()
}

#[test]
fn a_wake_keeps_its_arming_context_until_it_is_set_again() {
    let fixture = Database::new();
    let mut db = fixture.open();
    install(&mut db, "todo");
    arm(&mut db, "todo", &[wake("tool", 100, "")], true).unwrap();
    arm(&mut db, "todo", &[wake("self", 100, "")], false).unwrap();
    assert_eq!(
        origins(&db, 100),
        [("self".into(), false), ("tool".into(), true)]
    );
    // Waiting for an on-demand start or a retry keeps the origin.
    let due = due_wakes(&db, 100, 8).unwrap();
    assert_eq!(
        defer_wake(&db, &due[0], 100).unwrap(),
        WakeFailureOutcome::Retried
    );
    postpone_wake(&db, &due[1], 200).unwrap();
    assert_eq!(origins(&db, 200), [("tool".into(), true)]);
    // Re-armed from the App's own wake handler, it is the App's own wake.
    arm(&mut db, "todo", &[wake("tool", 300, "")], false).unwrap();
    assert_eq!(origins(&db, 300), [("tool".into(), false)]);
    arm(&mut db, "todo", &[wake("tool", 400, "")], true).unwrap();
    assert_eq!(origins(&db, 400), [("tool".into(), true)]);
}

#[test]
fn wakes_armed_before_their_origin_was_recorded_do_not_count_as_use() {
    let fixture = Database::new();
    let mut db = Connection::open(fixture.0.join("kernel.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TABLE app_wakes (
           owner_id TEXT NOT NULL, installation_id TEXT NOT NULL, wake_id TEXT NOT NULL,
           due_at_ms INTEGER NOT NULL CHECK(due_at_ms >= 0), revision TEXT NOT NULL,
           attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0),
           next_attempt_at_ms INTEGER NOT NULL CHECK(next_attempt_at_ms >= 0),
           PRIMARY KEY(installation_id, wake_id));
         INSERT INTO app_wakes VALUES('alice','todo','old',100,'',0,100);",
    )
    .unwrap();
    ManagedStateStore::new(&mut db).initialize().unwrap();
    ManagedStateStore::new(&mut db).initialize().unwrap();
    assert_eq!(origins(&db, 100), [("old".into(), false)]);
}

#[test]
fn wakes_are_due_in_order_replaced_by_id_and_completed_only_at_their_revision() {
    let fixture = Database::new();
    let mut db = fixture.open();
    install(&mut db, "todo");
    apply_wakes(
        &mut db,
        "todo",
        &[wake("b", 200, "r1"), wake("a", 100, "r1")],
    )
    .unwrap();
    assert!(due_wakes(&db, 99, 8).unwrap().is_empty());
    let due = due_wakes(&db, 250, 8).unwrap();
    assert_eq!(
        due.iter().map(|w| w.wake.id.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(due[0].owner_id, "alice");
    // A replacement after delivery began keeps the newer due time and revision.
    apply_wakes(&mut db, "todo", &[wake("a", 300, "r2")]).unwrap();
    complete_wake(&db, &due[0]).unwrap();
    let pending = due_wakes(&db, 1_000, 8).unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|w| (w.wake.id.as_str(), w.wake.revision.as_str()))
            .collect::<Vec<_>>(),
        [("b", "r1"), ("a", "r2")]
    );
    apply_wakes(&mut db, "todo", &[WakeChange::Cancel { id: "b".into() }]).unwrap();
    assert_eq!(due_wakes(&db, 1_000, 8).unwrap().len(), 1);
}

#[test]
fn failed_wake_delivery_backs_off_and_is_dropped_after_bounded_attempts() {
    let fixture = Database::new();
    let mut db = fixture.open();
    install(&mut db, "todo");
    apply_wakes(&mut db, "todo", &[wake("a", 100, "")]).unwrap();
    let mut now = 100;
    let mut kept = true;
    let mut attempts = 0;
    while kept {
        let due = due_wakes(&db, now, 8).unwrap();
        assert_eq!(due.len(), 1);
        let outcome = defer_wake(&db, &due[0], now).unwrap();
        assert_eq!(
            outcome,
            if attempts < 7 {
                WakeFailureOutcome::Retried
            } else {
                WakeFailureOutcome::Dropped
            }
        );
        kept = outcome == WakeFailureOutcome::Retried;
        if kept {
            // A deferred wake is not redelivered before its backoff delay.
            assert!(due_wakes(&db, now, 8).unwrap().is_empty());
        }
        attempts += 1;
        now = now.saturating_add(24 * 3_600_000);
    }
    assert_eq!(attempts, 8);
    assert!(due_wakes(&db, u64::MAX >> 12, 8).unwrap().is_empty());
}

#[test]
fn wakes_require_an_active_installation_and_are_bounded() {
    let fixture = Database::new();
    let mut db = fixture.open();
    assert!(apply_wakes(&mut db, "missing", &[wake("a", 1, "")]).is_err());
    install(&mut db, "todo");
    assert!(matches!(
        apply_wakes(&mut db, "todo", &[wake("bad id", 1, "")]),
        Err(StateError::Invalid)
    ));
    for batch in 0..MAX_WAKES / 16 {
        let changes: Vec<_> = (0..16)
            .map(|n| wake(&format!("w{batch}-{n}"), 1, ""))
            .collect();
        apply_wakes(&mut db, "todo", &changes).unwrap();
    }
    assert!(matches!(
        apply_wakes(&mut db, "todo", &[wake("overflow", 1, "")]),
        Err(StateError::Limit)
    ));
    let tx = db.transaction().unwrap();
    assert_eq!(
        ManagedStateStore::wakes_in(&tx, scope("todo"))
            .unwrap()
            .len(),
        MAX_WAKES
    );
}

const DAY_MS: u64 = 86_400_000;
const NOW_MS: u64 = 1_790_000_000_000;

fn due_ids(db: &Connection, now_ms: u64) -> Vec<(String, String, u32)> {
    due_wakes(db, now_ms, 8)
        .unwrap()
        .into_iter()
        .map(|due| (due.wake.id, due.wake.revision, due.attempts))
        .collect()
}

#[test]
fn wake_due_times_are_absolute_epoch_ms_however_far_ahead() {
    let fixture = Database::new();
    let mut db = fixture.open();
    install(&mut db, "todo");
    // Past a 32-bit millisecond timer (~24.8 days) and up to the JSON-safe limit.
    let far = NOW_MS + 40 * DAY_MS;
    apply_wakes(
        &mut db,
        "todo",
        &[wake("far", far, "r1"), wake("limit", MAX_REVISION, "r1")],
    )
    .unwrap();
    assert!(matches!(
        apply_wakes(&mut db, "todo", &[wake("over", MAX_REVISION + 1, "r1")]),
        Err(StateError::Invalid)
    ));
    assert!(due_ids(&db, NOW_MS).is_empty());
    assert!(due_ids(&db, far - 1).is_empty());
    assert_eq!(due_ids(&db, far), [("far".into(), "r1".into(), 0)]);
    let tx = db.transaction().unwrap();
    let stored = ManagedStateStore::wakes_in(&tx, scope("todo")).unwrap();
    assert!(stored
        .iter()
        .any(|w| w.id == "limit" && w.due_at_ms == MAX_REVISION));
}

#[test]
fn a_clock_jump_neither_loses_nor_strands_a_due_wake() {
    let fixture = Database::new();
    let mut db = fixture.open();
    install(&mut db, "todo");
    apply_wakes(
        &mut db,
        "todo",
        &[
            wake("a", NOW_MS, "r1"),
            wake("b", NOW_MS + 60_000, "r1"),
            wake("later", NOW_MS + DAY_MS, "r1"),
        ],
    )
    .unwrap();
    // Forward: everything that fell due during the jump is due, oldest first.
    let ahead = NOW_MS + DAY_MS / 2;
    let due = due_wakes(&db, ahead, 8).unwrap();
    assert_eq!(
        due.iter().map(|w| w.wake.id.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    // Deliveries fail or wait while the clock is ahead...
    defer_wake(&db, &due[0], ahead).unwrap();
    postpone_wake(&db, &due[1], ahead + 60_000).unwrap();
    // ...then it is corrected back: both are due again at once, not in half a day.
    let corrected = NOW_MS + 120_000;
    assert_eq!(
        due_ids(&db, corrected),
        [("a".into(), "r1".into(), 1), ("b".into(), "r1".into(), 0)]
    );
    // A wake not yet due by the corrected clock waits for its own due time.
    assert!(due_ids(&db, corrected).iter().all(|(id, ..)| id != "later"));
    // Backward before a due time: nothing is delivered early or lost.
    let fixture = Database::new();
    let mut db = fixture.open();
    install(&mut db, "todo");
    apply_wakes(&mut db, "todo", &[wake("a", NOW_MS, "r1")]).unwrap();
    assert!(due_ids(&db, NOW_MS - DAY_MS).is_empty());
    assert_eq!(due_ids(&db, NOW_MS), [("a".into(), "r1".into(), 0)]);
}

#[test]
fn outcomes_recorded_after_an_edit_or_delete_across_the_due_time_keep_the_new_schedule() {
    let fixture = Database::new();
    let mut db = fixture.open();
    install(&mut db, "todo");
    apply_wakes(
        &mut db,
        "todo",
        &[wake("edit", NOW_MS, "r1"), wake("gone", NOW_MS, "r1")],
    )
    .unwrap();
    // The pump reads both as due; the App moves one and deletes the other
    // before the delivery outcome is recorded.
    let due = due_wakes(&db, NOW_MS, 8).unwrap();
    apply_wakes(
        &mut db,
        "todo",
        &[
            wake("edit", NOW_MS + DAY_MS, "r2"),
            WakeChange::Cancel { id: "gone".into() },
        ],
    )
    .unwrap();
    for stale in &due {
        defer_wake(&db, stale, NOW_MS).unwrap();
        postpone_wake(&db, stale, NOW_MS + 2_000).unwrap();
        complete_wake(&db, stale).unwrap();
    }
    assert!(due_ids(&db, NOW_MS + DAY_MS - 1).is_empty());
    // The edited wake keeps its new time and revision with no spent attempts;
    // the deleted one never comes back.
    assert_eq!(
        due_ids(&db, MAX_REVISION),
        [("edit".into(), "r2".into(), 0)]
    );
}

#[test]
fn a_short_clock_correction_recovers_retries_in_due_order_after_reopen() {
    let fixture = Database::new();
    let mut db = fixture.open();
    install(&mut db, "todo");
    let now = 1_000_000;
    apply_wakes(
        &mut db,
        "todo",
        &[
            wake("older-z", now, "r1"),
            wake("newer-a", now + 60_000, "r1"),
            wake("later", now + 3_600_000, "r1"),
        ],
    )
    .unwrap();
    let ahead = now + 1_000_000;
    let due = due_wakes(&db, ahead, 8).unwrap();
    for wake in &due {
        defer_wake(&db, wake, ahead).unwrap();
    }
    assert_eq!(next_wake_at_ms(&db, ahead).unwrap(), Some(ahead + 10_000));
    // Sub-second jitter preserves retry backoff.
    assert_eq!(next_wake_at_ms(&db, ahead - 1).unwrap(), Some(ahead + 10_000));
    // An unchanged clock must still honor real retry backoff.
    assert!(due_wakes(&db, ahead, 8).unwrap().is_empty());
    drop(db);
    let db = fixture.open();
    let corrected = now + 120_000;
    // The scheduler must see the overdue original deadline before asking the
    // writer to perform rollback recovery. This read itself mutates nothing.
    assert_eq!(next_wake_at_ms(&db, corrected).unwrap(), Some(now));
    let recovered = due_wakes(&db, corrected, 8).unwrap();
    assert_eq!(
        recovered
            .iter()
            .map(|w| (w.wake.id.as_str(), w.attempts))
            .collect::<Vec<_>>(),
        [("older-z", 1), ("newer-a", 1)]
    );
    for wake in &recovered {
        complete_wake(&db, wake).unwrap();
    }
    assert!(due_wakes(&db, corrected, 8).unwrap().is_empty());
    assert_eq!(next_wake_at_ms(&db, corrected).unwrap(), Some(now + 3_600_000));
    assert_eq!(
        due_wakes(&db, now + 3_600_000, 8).unwrap()[0].wake.id,
        "later"
    );
}

#[test]
fn postponed_pages_step_aside_for_later_deliverable_wakes() {
    let fixture = Database::new();
    let mut db = fixture.open();
    install(&mut db, "todo");
    let mut changes = (0..8)
        .map(|i| wake(&format!("waiting-{i}"), 100 + i, "r1"))
        .collect::<Vec<_>>();
    changes.push(wake("deliverable", 900, "r1"));
    apply_wakes(&mut db, "todo", &changes).unwrap();
    for waiting in due_wakes(&db, 2_000, 8).unwrap() {
        postpone_wake(&db, &waiting, 4_000).unwrap();
    }
    // The idle cadence is longer than the postponed start delay. A full page
    // of eligible waiting wakes must still yield to later work on this pass.
    assert_eq!(due_wakes(&db, 7_000, 8).unwrap()[0].wake.id, "deliverable");
}

#[test]
fn empty_and_future_only_wake_polls_do_not_write_durable_state() {
    let fixture = Database::new();
    let mut db = fixture.open();
    let observer = fixture.open();
    let version = || {
        observer
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
            .unwrap()
    };
    let before = version();
    assert!(due_wakes(&db, 1_000, 8).unwrap().is_empty());
    assert_eq!(version(), before);
    install(&mut db, "todo");
    apply_wakes(&mut db, "todo", &[wake("future", 10_000, "r1")]).unwrap();
    let before = version();
    assert!(due_wakes(&db, 2_000, 8).unwrap().is_empty());
    assert_eq!(version(), before);
    let due = due_wakes(&db, 10_000, 8).unwrap();
    postpone_wake(&db, &due[0], 70_000).unwrap();
    let before = version();
    // A stopped App can leave a postponed wake for a minute. Polling it
    // before the retry must not sync clock state or collapse its backoff.
    assert!(due_wakes(&db, 11_000, 8).unwrap().is_empty());
    assert!(due_wakes(&db, 12_000, 8).unwrap().is_empty());
    assert!(due_wakes(&db, 9_999, 8).unwrap().is_empty());
    assert_eq!(version(), before);
    // A real correction resets the future retry to its original due time.
    assert!(due_wakes(&db, 8_000, 8).unwrap().is_empty());
    assert_ne!(version(), before);
    let before = version();
    assert!(due_wakes(&db, 9_000, 8).unwrap().is_empty());
    assert_eq!(version(), before);
    assert_eq!(due_wakes(&db, 10_000, 8).unwrap().len(), 1);
}

#[test]
fn wake_deadline_tracks_committed_replacements_cancellation_and_postponement() {
    let fixture = Database::new();
    let mut db = fixture.open();
    install(&mut db, "todo");
    assert_eq!(next_wake_at_ms(&db, 0).unwrap(), None);
    apply_wakes(&mut db, "todo", &[wake("a", 100, "r1"), wake("b", 200, "r1")]).unwrap();
    assert_eq!(next_wake_at_ms(&db, 0).unwrap(), Some(100));
    let due = due_wakes(&db, 100, 8).unwrap();
    postpone_wake(&db, &due[0], 400).unwrap();
    assert_eq!(next_wake_at_ms(&db, 100).unwrap(), Some(200));
    apply_wakes(&mut db, "todo", &[WakeChange::Cancel { id: "b".into() }]).unwrap();
    assert_eq!(next_wake_at_ms(&db, 100).unwrap(), Some(400));
    apply_wakes(&mut db, "todo", &[wake("a", 300, "r2")]).unwrap();
    complete_wake(&db, &due[0]).unwrap(); // stale completion preserves replacement
    assert_eq!(next_wake_at_ms(&db, 100).unwrap(), Some(300));
}
