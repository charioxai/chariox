use chariox_app_runtime::{
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationError, InstallationRegistry,
        ReleaseMetadata,
    },
    managed_state::{
        ManagedStateStore, StateChanges, StateCheck, StateError, StateScope, StateWrite,
        MAX_CHANGES, MAX_KEYS, MAX_REVISION, MAX_STATE_BYTES, MAX_VALUE_BYTES,
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
