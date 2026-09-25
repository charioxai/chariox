//! Restricted structured-state migrations inside a supervised local update:
//! quiesce snapshots, only the staged worker's scope migrates, commit requires
//! the target schema and every pre-commit exit restores the snapshot.
use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationError, InstallationRegistry,
        StageToken, StageTrustBinding, VerifiedInstallCandidate, VerifiedStageError,
    },
    managed_state::{ManagedStateStore, StateChanges, StateError, StateScope, StateWrite},
    publisher_trust::{PublisherTrustRegistry, TrustDecision, TrustedPublisherSnapshot},
};
use ed25519_dalek::SigningKey;
use rusqlite::{Connection, TransactionBehavior};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::PathBuf};

struct Database(PathBuf, Connection);
impl Database {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-state-migration-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        let mut connection = Connection::open(path.join("kernel.sqlite")).unwrap();
        PublisherTrustRegistry::new(&mut connection).initialize().unwrap();
        InstallationRegistry::new(&mut connection).initialize().unwrap();
        ManagedStateStore::new(&mut connection).initialize().unwrap();
        Self(path, connection)
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const OWNER: &str = "alice";
const INSTALLATION: &str = "todo";

fn package(version: &str, target: u32) -> (Vec<u8>, TrustedPublisher) {
    let mut manifest: Manifest = serde_json::from_value(json!({
        "schema":"chariox.app.v1", "appId":"com.example.todo", "version":version,
        "publisher":{"id":"com.example","keyId":"developer-1","name":"Developer"},
        "sdkVersion":"0.8.0", "appContractVersion":1, "minKernelProtocol":500,
        "resourcePolicy":"chariox.app.resources.v1", "runtime":{"engine":"node","entry":"runtime/main.js"},
        "ui":{"entry":"ui/index.html"}, "tools":"schemas/tools.json", "capabilities":{}
    }))
    .unwrap();
    let mut files = BTreeMap::from([
        ("runtime/main.js".to_string(), b"export default function register() {}".to_vec()),
        ("ui/index.html".into(), b"<!doctype html><title>Todo</title>".to_vec()),
        ("schemas/tools.json".into(), br#"{"tools":[]}"#.to_vec()),
    ]);
    if target > 0 {
        let steps: Vec<Value> = (0..target)
            .map(|from| json!({"from":from,"to":from+1,"entry":format!("migrations/{:03}.js", from + 1)}))
            .collect();
        manifest.migrations = Some(
            serde_json::from_value(json!({"directory":"migrations","targetVersion":target,"steps":steps}))
                .unwrap(),
        );
        for step in 1..=target {
            files.insert(
                format!("migrations/{step:03}.js"),
                b"export default async function migrate() {}".to_vec(),
            );
        }
    }
    let key = SigningKey::from_bytes(&[27; 32]);
    let publisher = TrustedPublisher {
        publisher_id: "com.example".into(),
        key_id: "developer-1".into(),
        public_key: key.verifying_key(),
    };
    (pack(&manifest, &files, &key, &Limits::default()).unwrap(), publisher)
}

fn trust(connection: &mut Connection, publisher: &TrustedPublisher) -> TrustedPublisherSnapshot {
    let mut publishers = PublisherTrustRegistry::new(connection);
    if publishers
        .trusted_publisher(OWNER, &publisher.publisher_id, &publisher.key_id)
        .is_err()
    {
        let decision = TrustDecision {
            decision_id: "trust".into(),
            authority_ref: "kernel-confirmation".into(),
        };
        publishers.enroll(OWNER, publisher, 0, &decision, 1).unwrap();
    }
    publishers
        .trusted_publisher(OWNER, &publisher.publisher_id, &publisher.key_id)
        .unwrap()
}

fn approval() -> CapabilityApproval {
    CapabilityApproval {
        decision_id: "capabilities".into(),
        authority_ref: "kernel-confirmation".into(),
    }
}

/// Stages a release (first install or update) and approves it; returns the
/// stage token and the enrollment snapshot.
fn stage(
    connection: &mut Connection,
    version: &str,
    target: u32,
    base: u64,
) -> Result<(StageToken, TrustedPublisherSnapshot), VerifiedStageError> {
    let (bytes, publisher) = package(version, target);
    let trust = trust(connection, &publisher);
    let verified = verify(&bytes, &VerificationPolicy::new(500, vec![publisher])).unwrap();
    let candidate = VerifiedInstallCandidate::from_verified(&verified, &trust).unwrap();
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let record = if base == 0 {
        candidate.stage_first_in(&tx, OWNER, INSTALLATION, 10)?
    } else {
        candidate.stage_update_in(&tx, OWNER, INSTALLATION, base, 10)?
    };
    StageTrustBinding::staged_in(&tx, OWNER, &record.token)
        .unwrap()
        .decide_in(
            &tx,
            OWNER,
            &trust,
            CapabilityDecision::Approved {
                approval: approval(),
            },
            11,
        )
        .unwrap();
    tx.commit().unwrap();
    Ok((record.token, trust))
}

fn quiesce(connection: &mut Connection, token: &StageToken, trust: &TrustedPublisherSnapshot) {
    let tx = connection.transaction().unwrap();
    StageTrustBinding::staged_in(&tx, OWNER, token)
        .unwrap()
        .quiesce_in(&tx, OWNER, trust, 12)
        .unwrap();
    tx.commit().unwrap();
}

fn commit(
    connection: &mut Connection,
    token: &StageToken,
    trust: &TrustedPublisherSnapshot,
) -> Result<(), VerifiedStageError> {
    let tx = connection.transaction().unwrap();
    StageTrustBinding::staged_in(&tx, OWNER, token)
        .unwrap()
        .commit_in(&tx, OWNER, trust, &approval(), 13)?;
    tx.commit().unwrap();
    Ok(())
}

fn abort(connection: &mut Connection, token: &StageToken) {
    let tx = connection.transaction().unwrap();
    StageTrustBinding::staged_in(&tx, OWNER, token)
        .unwrap()
        .abort_in(&tx, OWNER, "health_failed", 14)
        .unwrap();
    tx.commit().unwrap();
}

fn put(schema: u32, key: &str, value: Value) -> StateChanges {
    StateChanges::new(
        schema,
        vec![],
        vec![StateWrite::Put {
            key: key.into(),
            value,
        }],
    )
    .unwrap()
}

fn write(
    connection: &mut Connection,
    generation: u64,
    schema: u32,
    key: &str,
    value: Value,
) -> Result<u64, StateError> {
    let scope = StateScope::new(OWNER, INSTALLATION, generation).unwrap();
    ManagedStateStore::new(connection).transaction(scope, &put(schema, key, value))
}

fn read(connection: &mut Connection, generation: u64, key: &str) -> Option<Value> {
    let scope = StateScope::new(OWNER, INSTALLATION, generation).unwrap();
    ManagedStateStore::new(connection)
        .get(scope, key)
        .unwrap()
        .map(|record| record.value)
}

fn step(connection: &mut Connection, generation: u64, to: u32) -> Result<(), StateError> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let scope = StateScope::new(OWNER, INSTALLATION, generation).unwrap();
    ManagedStateStore::migration_step_in(&tx, scope, to)?;
    tx.commit().unwrap();
    Ok(())
}

fn schema(connection: &mut Connection) -> u32 {
    InstallationRegistry::new(connection)
        .get(INSTALLATION)
        .unwrap()
        .active
        .unwrap()
        .release
        .schema_version
}

/// Installs 1.0.0 (schema 0) with one Todo, then stages and quiesces 1.1.0.
fn updating(db: &mut Database, target: u32) -> (StageToken, TrustedPublisherSnapshot) {
    let (first, trust) = stage(&mut db.1, "1.0.0", 0, 0).unwrap();
    quiesce(&mut db.1, &first, &trust);
    commit(&mut db.1, &first, &trust).unwrap();
    write(&mut db.1, first.generation, 0, "todos", json!(["milk"])).unwrap();
    let (update, trust) = stage(&mut db.1, "1.1.0", target, first.generation).unwrap();
    quiesce(&mut db.1, &update, &trust);
    (update, trust)
}

#[test]
fn a_migrated_update_commits_the_new_schema_and_its_migrated_data() {
    let mut db = Database::new("commit");
    let (update, trust) = updating(&mut db, 2);
    let old = update.base_generation;
    // The old generation is fenced; the staged worker migrates step by step.
    assert!(matches!(
        write(&mut db.1, old, 0, "todos", json!([])),
        Err(StateError::Installation(InstallationError::AdmissionPaused))
    ));
    assert_eq!(read(&mut db.1, update.generation, "todos"), Some(json!(["milk"])));
    assert!(matches!(
        write(&mut db.1, update.generation, 2, "todos", json!([])),
        Err(StateError::SchemaMismatch)
    ));
    write(&mut db.1, update.generation, 1, "todos", json!([{"title":"milk"}])).unwrap();
    assert!(matches!(step(&mut db.1, update.generation, 2), Err(StateError::SchemaMismatch)));
    step(&mut db.1, update.generation, 1).unwrap();
    write(&mut db.1, update.generation, 2, "todos", json!({"items":[{"title":"milk"}]})).unwrap();
    step(&mut db.1, update.generation, 2).unwrap();
    commit(&mut db.1, &update, &trust).unwrap();
    assert_eq!(schema(&mut db.1), 2);
    assert_eq!(
        read(&mut db.1, update.generation, "todos"),
        Some(json!({"items":[{"title":"milk"}]}))
    );
    write(&mut db.1, update.generation, 2, "todos", json!({"items":[]})).unwrap();
}

#[test]
fn an_aborted_update_restores_the_data_it_migrated() {
    let mut db = Database::new("abort");
    let (update, _) = updating(&mut db, 1);
    let before = write(&mut db.1, update.generation, 1, "todos", json!("migrated")).unwrap();
    write(&mut db.1, update.generation, 1, "extra", json!(1)).unwrap();
    abort(&mut db.1, &update);
    let old = update.base_generation;
    assert_eq!(schema(&mut db.1), 0);
    assert_eq!(read(&mut db.1, old, "todos"), Some(json!(["milk"])));
    assert_eq!(read(&mut db.1, old, "extra"), None);
    // The head revision keeps moving forward past the migrated writes.
    assert!(write(&mut db.1, old, 0, "todos", json!(["milk", "eggs"])).unwrap() > before + 1);
}

#[test]
fn a_commit_is_refused_until_every_step_is_reported() {
    let mut db = Database::new("incomplete");
    let (update, trust) = updating(&mut db, 1);
    assert!(matches!(
        commit(&mut db.1, &update, &trust),
        Err(VerifiedStageError::Installation(InstallationError::Invalid(
            "data migration incomplete"
        )))
    ));
    step(&mut db.1, update.generation, 1).unwrap();
    commit(&mut db.1, &update, &trust).unwrap();
    assert_eq!(schema(&mut db.1), 1);
}

#[test]
fn data_is_never_migrated_down() {
    let mut db = Database::new("downgrade");
    let (update, trust) = updating(&mut db, 1);
    step(&mut db.1, update.generation, 1).unwrap();
    commit(&mut db.1, &update, &trust).unwrap();
    assert!(matches!(
        stage(&mut db.1, "1.0.1", 0, update.generation),
        Err(VerifiedStageError::Installation(InstallationError::Invalid(
            "data schema downgrade"
        )))
    ));
}
