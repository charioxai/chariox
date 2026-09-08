//! Storage fixtures enroll automation rows as trusted kernel configuration.
//! This does not prove a public automation grant protocol exists.
use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    app_catalog::AppCatalog,
    app_outbox::{
        AppOutbox, EventCatalog, Invocation, Occurrence, OutboxError, Receipt, ReceiptState,
        VerifiedAutomation, MAX_FUTURE_SKEW_MS, MAX_OCCURRENCE_AGE_MS, MAX_PENDING, MAX_RECEIPTS,
        MAX_SAFE_TIMESTAMP,
    },
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationRegistry, StageToken,
        VerifiedInstallCandidate,
    },
    managed_state::{ManagedStateStore, StateChanges, StateScope, StateWrite},
    publisher_trust::{PublisherTrustRegistry, TrustDecision, TrustedPublisherSnapshot},
};
use ed25519_dalek::SigningKey;
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};

#[path = "app_outbox/configuration.rs"]
mod configuration;
#[path = "app_outbox/contract.rs"]
mod contract;
#[path = "app_outbox/invocations.rs"]
mod invocations;
#[path = "app_outbox/limits.rs"]
mod limits;

struct Database(PathBuf);
impl Database {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-app-outbox-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> Connection {
        let mut db = Connection::open(self.0.join("kernel.sqlite")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .unwrap();
        InstallationRegistry::new(&mut db).initialize().unwrap();
        PublisherTrustRegistry::new(&mut db).initialize().unwrap();
        ManagedStateStore::new(&mut db).initialize().unwrap();
        AppOutbox::initialize(&db).unwrap();
        db
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
struct Package {
    manifest: Manifest,
    files: BTreeMap<String, Vec<u8>>,
    key: SigningKey,
}
impl Package {
    fn new() -> Self {
        Self { manifest:serde_json::from_value(json!({
            "schema":"chariox.app.v1","appId":"com.example.events","version":"1.0.0",
            "publisher":{"id":"com.example","keyId":"developer","name":"Developer"},
            "sdkVersion":"0.6.0","appContractVersion":1,"minKernelProtocol":500,
            "resourcePolicy":"chariox.app.resources.v1","runtime":{"engine":"node","entry":"runtime/main.js"},
            "ui":{"entry":"ui/index.html"},"events":"schemas/events.json","capabilities":{}
        })).unwrap(),key:SigningKey::from_bytes(&[73;32]), files:BTreeMap::from([
            ("runtime/main.js".into(),b"export default function register() {}".to_vec()),
            ("ui/index.html".into(),b"<!doctype html><title>Outbox fixture</title>".to_vec()),
            ("schemas/events.json".into(),serde_json::to_vec(&json!({"events":[{"name":"changed","schemaVersion":1,"direction":"outgoing","payloadSchema":{
                "type":"object","additionalProperties":false,"required":["text"],
                "properties":{"text":{"type":"string"},"count":{"type":"integer"}}
            }}]})).unwrap()),
        ])}
    }
    fn publisher(&self) -> TrustedPublisher {
        TrustedPublisher {
            publisher_id: "com.example".into(),
            key_id: "developer".into(),
            public_key: self.key.verifying_key(),
        }
    }
    fn bytes(&self) -> Vec<u8> {
        pack(&self.manifest, &self.files, &self.key, &Limits::default()).unwrap()
    }
    fn activate(
        &self,
        db: &mut Connection,
        trust: &TrustedPublisherSnapshot,
        old: Option<u64>,
    ) -> (StageToken, Arc<EventCatalog>) {
        let bytes = self.bytes();
        let verified = verify(
            &bytes,
            &VerificationPolicy::new(500, vec![self.publisher()]),
        )
        .unwrap();
        let candidate = VerifiedInstallCandidate::from_verified(&verified, trust).unwrap();
        let mut registry = InstallationRegistry::new(db);
        let token = match old {
            None => {
                registry
                    .create_and_stage_verified("installed", "owner", &candidate, 1)
                    .unwrap()
                    .token
            }
            Some(generation) => {
                registry
                    .stage_verified("installed", "owner", generation, &candidate, 10)
                    .unwrap()
                    .token
            }
        };
        registry
            .decide(
                &token,
                CapabilityDecision::Approved {
                    approval: CapabilityApproval {
                        decision_id: format!("approved-{}", token.generation),
                        authority_ref: "kernel-human".into(),
                    },
                },
                11,
            )
            .unwrap();
        registry.quiesce(&token, 12).unwrap();
        registry.mark_prepared(&token, 13).unwrap();
        registry
            .commit_verified(&token, "owner", trust, 14)
            .unwrap();
        let binding = registry.staged_trust("owner", &token).unwrap();
        let catalog = Arc::new(AppCatalog::compile(&verified, &binding, trust).unwrap());
        (
            token,
            Arc::new(EventCatalog::compile(&verified, catalog).unwrap()),
        )
    }
}
fn decision(id: &str) -> TrustDecision {
    TrustDecision {
        decision_id: id.into(),
        authority_ref: "kernel-human".into(),
    }
}
fn setup(db: &mut Connection) -> (Package, TrustedPublisherSnapshot, Arc<EventCatalog>) {
    let package = Package::new();
    let mut publishers = PublisherTrustRegistry::new(db);
    publishers
        .enroll("owner", &package.publisher(), 0, &decision("enroll"), 1)
        .unwrap();
    let trust = publishers
        .trusted_publisher("owner", "com.example", "developer")
        .unwrap();
    let (_, catalog) = package.activate(db, &trust, None);
    // The target is trusted fixture state; event version and schema must match
    // the actual publisher-signed declaration when the snapshot is loaded.
    db.execute("INSERT INTO app_automations (owner_id,installation_id,automation_id,revision,event_name,event_version,schema_digest,session_id,publication_id,endpoint_id,queue_id,status)
        VALUES ('owner','installed','automation',1,'changed',1,?1,'session','publication','endpoint','default','active')",
        [catalog.schema_digest("changed").unwrap()]).unwrap();
    (package, trust, catalog)
}
fn automation(db: &mut Connection, catalog: &Arc<EventCatalog>) -> VerifiedAutomation {
    let tx = db.transaction().unwrap();
    VerifiedAutomation::load_in(&tx, catalog.clone(), "owner", "automation", 1).unwrap()
}
fn occurrence(id: &str, text: &str) -> Occurrence {
    Occurrence {
        automation_id: "automation".into(),
        occurrence_id: chariox_app_runtime::app_outbox::occurrence_id(id, 100).unwrap(),
        event_version: 1,
        occurred_at_ms: 100,
        schedule_revision: None,
        payload: json!({"text":text}),
        invocation: Invocation {
            prompt: "Handle the event".into(),
            artifacts: vec![],
        },
    }
}
fn accept(db: &mut Connection, automation: &VerifiedAutomation, id: &str) -> Receipt {
    let mut tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let receipt = AppOutbox::apply_in(&mut tx, automation, &[occurrence(id, "value")], 100)
        .unwrap()
        .remove(0);
    tx.commit().unwrap();
    receipt
}
fn count(db: &Connection) -> i64 {
    db.query_row("SELECT count(*) FROM app_outbox", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn accepted_occurrences_reopen_canonical_retry_conflicts_and_owner_isolation() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let original = accept(&mut db, &authority, "first");
    drop(db);
    let mut db = directory.open();
    let authority = automation(&mut db, &catalog);
    let duplicate = accept(&mut db, &authority, "first");
    assert_eq!(duplicate, original);
    assert_eq!(count(&db), 1);
    let mut tx = db.transaction().unwrap();
    assert!(matches!(
        AppOutbox::apply_in(&mut tx, &authority, &[occurrence("first", "changed")], 101),
        Err(OutboxError::Conflict)
    ));
    assert!(VerifiedAutomation::load_in(&tx, catalog.clone(), "other", "automation", 1).is_err());
    assert!(AppOutbox::status_in(&tx, &catalog, "other", &original.receipt_id).is_err());
    let mut with_count = occurrence("canonical", "value");
    with_count.payload = json!({"text":"value","count":1});
    let first = AppOutbox::apply_in(&mut tx, &authority, &[with_count], 102).unwrap();
    let mut reordered = occurrence("canonical", "value");
    reordered.payload = serde_json::from_str(r#"{"count":1.0,"text":"value"}"#).unwrap();
    assert_eq!(
        AppOutbox::apply_in(&mut tx, &authority, &[reordered], 103).unwrap(),
        first
    );
    tx.commit().unwrap();
}

#[test]
fn state_outbox_and_batch_failures_rollback_on_the_same_sqlite_transaction() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    accept(&mut db, &authority, "existing");
    let changes = StateChanges::new(
        0,
        vec![],
        vec![StateWrite::Put {
            key: "saved".into(),
            value: json!(true),
        }],
    )
    .unwrap();
    let scope = StateScope::new("owner", "installed", 1).unwrap();
    {
        let mut tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        ManagedStateStore::apply_in(&mut tx, scope, &changes).unwrap();
        assert!(matches!(
            AppOutbox::apply_in(
                &mut tx,
                &authority,
                &[
                    occurrence("new", "value"),
                    occurrence("existing", "different")
                ],
                100
            ),
            Err(OutboxError::Conflict)
        ));
        // The enclosing state+occurrences operation propagates failure and rolls back.
    }
    assert!(ManagedStateStore::new(&mut db)
        .get(scope, "saved")
        .unwrap()
        .is_none());
    assert_eq!(count(&db), 1);
    db.execute_batch(&format!("CREATE TRIGGER outbox_fault BEFORE INSERT ON app_outbox WHEN NEW.occurrence_id='{}' BEGIN SELECT RAISE(ABORT,'fault'); END;",chariox_app_runtime::app_outbox::occurrence_id("fault",100).unwrap())).unwrap();
    let mut tx = db.transaction().unwrap();
    assert!(AppOutbox::apply_in(
        &mut tx,
        &authority,
        &[occurrence("new", "value"), occurrence("fault", "value")],
        100
    )
    .is_err());
    tx.commit().unwrap();
    assert_eq!(count(&db), 1); // Failed savepoint leaked no first occurrence.
    db.execute_batch("DROP TRIGGER outbox_fault;").unwrap();
    let mut tx = db.transaction().unwrap();
    ManagedStateStore::apply_in(&mut tx, scope, &changes).unwrap();
    AppOutbox::apply_in(
        &mut tx,
        &authority,
        &[occurrence("committed", "value")],
        100,
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(ManagedStateStore::new(&mut db)
        .get(scope, "saved")
        .unwrap()
        .is_some());
    assert_eq!(count(&db), 2);
}

#[test]
fn immutable_receipts_survive_payload_expiry_state_restore_and_generation_change() {
    let directory = Database::new();
    let mut db = directory.open();
    let (mut package, trust, catalog) = setup(&mut db);
    let old = automation(&mut db, &catalog);
    let original = accept(&mut db, &old, "expired");
    let tx = db.transaction().unwrap();
    let expired = AppOutbox::settle_in(
        &tx,
        &catalog,
        "owner",
        &original.receipt_id,
        1,
        ReceiptState::Expired,
        original.expires_at_ms,
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(expired.payload.is_none());
    assert_eq!(expired.content_digest, original.content_digest);
    db.execute_batch("DELETE FROM app_state_values;").unwrap();
    package.manifest.version = "2.0.0".into();
    let (_, current) = package.activate(&mut db, &trust, Some(1));
    let mut tx = db.transaction().unwrap();
    assert!(AppOutbox::apply_in(&mut tx, &old, &[occurrence("stale", "value")], 200).is_err());
    drop(tx);
    let current = automation(&mut db, &current);
    let mut tx = db.transaction().unwrap();
    let replay = AppOutbox::apply_in(
        &mut tx,
        &current,
        &[occurrence("expired", "value")],
        original.expires_at_ms + 1,
    )
    .unwrap()
    .remove(0);
    assert_eq!(replay, expired);
    assert!(matches!(
        AppOutbox::apply_in(
            &mut tx,
            &current,
            &[occurrence("expired", "different")],
            original.expires_at_ms + 1
        ),
        Err(OutboxError::Conflict)
    ));
}

#[test]
fn queued_transition_composes_with_queue_receipt_and_cannot_repeat_or_retarget() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let original = accept(&mut db, &authority, "work");
    db.execute_batch("CREATE TABLE fixture_existing_workflow_queue (id TEXT PRIMARY KEY);")
        .unwrap();
    {
        let tx = db.transaction().unwrap();
        tx.execute(
            "INSERT INTO fixture_existing_workflow_queue VALUES ('queued')",
            [],
        )
        .unwrap();
        AppOutbox::mark_queued_in(&tx, &authority, &original.receipt_id, 1, "queued", 100).unwrap();
        // Simulated interruption before the combined commit.
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM fixture_existing_workflow_queue",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let tx = db.transaction().unwrap();
    assert_eq!(
        AppOutbox::status_in(&tx, &catalog, "owner", &original.receipt_id)
            .unwrap()
            .state,
        ReceiptState::Accepted
    );
    tx.execute(
        "INSERT INTO fixture_existing_workflow_queue VALUES ('queued')",
        [],
    )
    .unwrap();
    let queued =
        AppOutbox::mark_queued_in(&tx, &authority, &original.receipt_id, 1, "queued", 100).unwrap();
    tx.commit().unwrap();
    assert_eq!(queued.state, ReceiptState::Queued);
    assert!(queued.payload.is_none());
    let tx = db.transaction().unwrap();
    assert!(
        AppOutbox::mark_queued_in(&tx, &authority, &original.receipt_id, 1, "other", 100).is_err()
    );
    let delivered = AppOutbox::settle_in(
        &tx,
        &catalog,
        "owner",
        &original.receipt_id,
        queued.revision,
        ReceiptState::Delivered,
        101,
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(accept(&mut db, &authority, "work"), delivered);
    let pending = accept(&mut db, &authority, "pending");
    db.execute(
        "UPDATE app_automations SET revision=revision+1,endpoint_id='other'",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert!(
        AppOutbox::mark_queued_in(&tx, &authority, &pending.receipt_id, 1, "wrong", 100).is_err()
    );
    let new = VerifiedAutomation::load_in(&tx, catalog.clone(), "owner", "automation", 2).unwrap();
    assert!(AppOutbox::mark_queued_in(&tx, &new, &pending.receipt_id, 1, "wrong", 100).is_err());
}

#[test]
fn competing_writers_retry_backoff_and_retained_receipt_capacity_are_bounded() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let original = accept(&mut db, &authority, "work");
    let mut second = directory.open();
    second.busy_timeout(std::time::Duration::ZERO).unwrap();
    let second_authority = automation(&mut second, &catalog);
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let retry =
        AppOutbox::mark_retryable_in(&tx, &authority, &original.receipt_id, 1, 200, 100).unwrap();
    assert!(second
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .is_err());
    tx.commit().unwrap();
    let tx = second.transaction().unwrap();
    assert!(AppOutbox::mark_retryable_in(
        &tx,
        &second_authority,
        &original.receipt_id,
        1,
        300,
        200
    )
    .is_err());
    assert!(AppOutbox::mark_queued_in(
        &tx,
        &second_authority,
        &original.receipt_id,
        retry.revision,
        "early",
        199
    )
    .is_err());
    assert!(AppOutbox::pending_in(&tx, &catalog, "owner", 199, 64)
        .unwrap()
        .is_empty());
    assert_eq!(
        AppOutbox::pending_in(&tx, &catalog, "owner", 200, 64)
            .unwrap()
            .len(),
        1
    );
    drop(tx);
    let tx = db.transaction().unwrap();
    AppOutbox::settle_in(
        &tx,
        &catalog,
        "owner",
        &original.receipt_id,
        retry.revision,
        ReceiptState::Failed,
        200,
    )
    .unwrap();
    tx.commit().unwrap();
    // Seed compact terminal receipts to exercise the real admission ceiling
    // without thousands of package/trust verifications in a fixture loop.
    db.execute("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?1)
        INSERT INTO app_outbox(owner_id,installation_id,receipt_id,automation_id,event_version,occurrence_id,event_name,schema_digest,content_digest,automation_revision,accepted_generation,payload_json,accepted_at_ms,expires_at_ms,state,revision,attempts,next_attempt_at_ms,occurred_at_ms,schedule_revision)
        SELECT owner_id,installation_id,'fixture-'||i,automation_id,event_version,'fixture-'||i,event_name,schema_digest,content_digest,automation_revision,accepted_generation,NULL,accepted_at_ms,expires_at_ms,'expired',1,0,next_attempt_at_ms,occurred_at_ms,schedule_revision FROM app_outbox,n WHERE receipt_id=?2",
        params![(MAX_RECEIPTS-1) as i64,original.receipt_id]).unwrap();
    assert_eq!(count(&db), MAX_RECEIPTS as i64);
    let mut tx = db.transaction().unwrap();
    assert!(matches!(
        AppOutbox::apply_in(&mut tx, &authority, &[occurrence("new", "value")], 300),
        Err(OutboxError::Limit)
    ));
    assert_eq!(
        AppOutbox::apply_in(&mut tx, &authority, &[occurrence("work", "value")], 300).unwrap()[0]
            .receipt_id,
        original.receipt_id
    );
}

#[test]
fn signed_schemas_version_checks_publisher_revocation_and_pending_capacity_are_enforced() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, trust, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let mut bad = occurrence("version", "value");
    bad.event_version = 2;
    let mut tx = db.transaction().unwrap();
    assert!(AppOutbox::apply_in(&mut tx, &authority, &[bad], 100).is_err());
    let mut bad = occurrence("schema", "value");
    bad.payload = json!({"extra":"not declared"});
    assert!(matches!(
        AppOutbox::apply_in(&mut tx, &authority, &[bad], 100),
        Err(OutboxError::Schema)
    ));
    assert!(AppOutbox::apply_in(
        &mut tx,
        &authority,
        &[occurrence("huge", &"x".repeat(65536))],
        100
    )
    .is_err());
    drop(tx);
    let original = accept(&mut db, &authority, "seed");
    db.execute("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?1)
        INSERT INTO app_outbox(owner_id,installation_id,receipt_id,automation_id,event_version,occurrence_id,event_name,schema_digest,content_digest,automation_revision,accepted_generation,payload_json,accepted_at_ms,expires_at_ms,state,revision,attempts,next_attempt_at_ms,occurred_at_ms,schedule_revision)
        SELECT owner_id,installation_id,'pending-'||i,automation_id,event_version,'pending-'||i,event_name,schema_digest,content_digest,automation_revision,accepted_generation,payload_json,accepted_at_ms,expires_at_ms,'accepted',1,0,next_attempt_at_ms,occurred_at_ms,schedule_revision FROM app_outbox,n WHERE receipt_id=?2",
        params![(MAX_PENDING-1) as i64,original.receipt_id]).unwrap();
    let mut tx = db.transaction().unwrap();
    assert!(matches!(
        AppOutbox::apply_in(&mut tx, &authority, &[occurrence("full", "value")], 100),
        Err(OutboxError::Limit)
    ));
    drop(tx);
    PublisherTrustRegistry::new(&mut db)
        .revoke(
            "owner",
            "com.example",
            "developer",
            trust.revision(),
            &decision("revoke"),
            200,
        )
        .unwrap();
    let mut tx = db.transaction().unwrap();
    assert!(AppOutbox::apply_in(&mut tx, &authority, &[occurrence("seed", "value")], 201).is_err());
}

#[test]
fn original_timestamp_window_applies_to_new_receipts_and_retained_duplicates_stay_stable() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    let authority = automation(&mut db, &catalog);
    let now = MAX_OCCURRENCE_AGE_MS + 1000;
    let mut oldest = occurrence("oldest", "value");
    oldest.occurred_at_ms = 1000;
    oldest.occurrence_id = chariox_app_runtime::app_outbox::occurrence_id("oldest", 1000).unwrap();
    let mut tx = db.transaction().unwrap();
    let accepted = AppOutbox::apply_in(&mut tx, &authority, &[oldest], now)
        .unwrap()
        .remove(0);
    let mut too_old = occurrence("too-old", "value");
    too_old.occurred_at_ms = 999;
    too_old.occurrence_id = chariox_app_runtime::app_outbox::occurrence_id("too-old", 999).unwrap();
    assert!(matches!(
        AppOutbox::apply_in(&mut tx, &authority, &[too_old], now),
        Err(OutboxError::TooOld)
    ));
    let mut future = occurrence("future", "value");
    future.occurred_at_ms = now + MAX_FUTURE_SKEW_MS;
    future.occurrence_id =
        chariox_app_runtime::app_outbox::occurrence_id("future", now + MAX_FUTURE_SKEW_MS).unwrap();
    AppOutbox::apply_in(&mut tx, &authority, &[future], now).unwrap();
    let mut future = occurrence("too-future", "value");
    future.occurred_at_ms = now + MAX_FUTURE_SKEW_MS + 1;
    future.occurrence_id =
        chariox_app_runtime::app_outbox::occurrence_id("too-future", now + MAX_FUTURE_SKEW_MS + 1)
            .unwrap();
    assert!(matches!(
        AppOutbox::apply_in(&mut tx, &authority, &[future], now),
        Err(OutboxError::Invalid)
    ));
    let mut unsafe_time = occurrence("unsafe", "value");
    unsafe_time.occurred_at_ms = MAX_SAFE_TIMESTAMP + 1;
    assert!(matches!(
        AppOutbox::apply_in(&mut tx, &authority, &[unsafe_time], now),
        Err(OutboxError::Invalid)
    ));
    tx.commit().unwrap();
    let mut tx = db.transaction().unwrap();
    let expired = AppOutbox::settle_in(
        &tx,
        &catalog,
        "owner",
        &accepted.receipt_id,
        1,
        ReceiptState::Expired,
        accepted.expires_at_ms,
    )
    .unwrap();
    let mut replay = occurrence("oldest", "value");
    replay.occurred_at_ms = 1000;
    replay.occurrence_id = chariox_app_runtime::app_outbox::occurrence_id("oldest", 1000).unwrap();
    assert_eq!(
        AppOutbox::apply_in(&mut tx, &authority, &[replay], now + MAX_OCCURRENCE_AGE_MS).unwrap()
            [0],
        expired
    );
    let mut changed = occurrence("oldest", "value");
    changed.occurred_at_ms = now;
    assert!(matches!(
        AppOutbox::apply_in(&mut tx, &authority, &[changed], now),
        Err(OutboxError::Invalid)
    ));
}

#[test]
fn signed_event_version_and_scheduled_occurrence_revisions_define_admission() {
    let directory = Database::new();
    let mut db = directory.open();
    let (_, _, catalog) = setup(&mut db);
    assert_eq!(catalog.event_version("changed"), Some(1));
    let ordinary = automation(&mut db, &catalog);
    let mut noise = occurrence("scheduled", "value");
    noise.schedule_revision = Some("revision-a".into());
    let mut tx = db.transaction().unwrap();
    assert!(AppOutbox::apply_in(&mut tx, &ordinary, &[noise], 100).is_err());
    drop(tx);
    db.execute("UPDATE app_automations SET event_version=2", [])
        .unwrap();
    let tx = db.transaction().unwrap();
    assert!(matches!(
        VerifiedAutomation::load_in(&tx, catalog.clone(), "owner", "automation", 1),
        Err(OutboxError::Schema)
    ));
    drop(tx);
    db.execute(
        "UPDATE app_automations SET event_version=1,scheduled=1,revision=2",
        [],
    )
    .unwrap();
    let mut tx = db.transaction().unwrap();
    let scheduled =
        VerifiedAutomation::load_in(&tx, catalog.clone(), "owner", "automation", 2).unwrap();
    assert!(AppOutbox::apply_in(
        &mut tx,
        &scheduled,
        &[occurrence("scheduled", "value")],
        100
    )
    .is_err());
    let mut first = occurrence("scheduled", "value");
    first.schedule_revision = Some("revision-a".into());
    let mut second = occurrence("scheduled", "value");
    second.schedule_revision = Some("revision-b".into());
    let receipts = AppOutbox::apply_in(&mut tx, &scheduled, &[first, second], 100).unwrap();
    assert_ne!(receipts[0].receipt_id, receipts[1].receipt_id);
    assert_eq!(receipts[0].schedule_revision.as_deref(), Some("revision-a"));
    let mut retry = occurrence("scheduled", "value");
    retry.schedule_revision = Some("revision-a".into());
    assert_eq!(
        AppOutbox::apply_in(&mut tx, &scheduled, &[retry], 101).unwrap()[0],
        receipts[0]
    );
    let mut invalid = occurrence("scheduled", "value");
    invalid.schedule_revision = Some("r".repeat(129));
    assert!(AppOutbox::apply_in(&mut tx, &scheduled, &[invalid], 101).is_err());
    tx.commit().unwrap();
    assert_eq!(count(&db), 2);
}

#[path = "app_outbox/retention.rs"]
mod retention;
