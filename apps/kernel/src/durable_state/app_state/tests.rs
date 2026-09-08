use super::*;
use crate::durable_state::{
    app_installation_staging::AppVerifiedInstallationMutation,
    app_publishers::AppPublisherMutation,
    apps::{AppRegistryMutation, AppRegistryOutcome},
};
use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    app_catalog::AppCatalog,
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationRegistry, VerifiedInstallCandidate,
    },
    managed_state::StateWrite,
    publisher_trust::TrustDecision,
};
use ed25519_dalek::SigningKey;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-app-state-writer-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn open(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.0.join("kernel.sqlite")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn publisher() -> TrustedPublisher {
    TrustedPublisher {
        publisher_id: "com.example".into(),
        key_id: "state-key".into(),
        public_key: SigningKey::from_bytes(&[71; 32]).verifying_key(),
    }
}
fn decision(id: &str) -> TrustDecision {
    TrustDecision {
        decision_id: id.into(),
        authority_ref: "kernel-fixture-decision".into(),
    }
}
pub(super) fn catalog(store: &DurableKernelStateStore) -> Arc<EventCatalog> {
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Enroll {
                publisher: publisher(),
                expected_revision: 0,
                decision: decision("enroll"),
                now_ms: 1,
            },
        )
        .unwrap();
    let manifest: Manifest=serde_json::from_value(json!({
        "schema":"chariox.app.v1","appId":"com.example.state","version":"1.0.0",
        "publisher":{"id":"com.example","keyId":"state-key","name":"Developer"},
        "sdkVersion":"0.3.0","appContractVersion":1,"minKernelProtocol":291,
        "resourcePolicy":"chariox.app.resources.v1","runtime":{"engine":"node","entry":"runtime/main.js"},
        "ui":{"entry":"ui/index.html"},"events":"schemas/events.json","capabilities":{}
    })).unwrap();
    let files = BTreeMap::from([
        (
            "runtime/main.js".into(),
            b"export default function register() {}".to_vec(),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><title>State fixture</title>".to_vec(),
        ),
        (
            "schemas/events.json".into(),
            serde_json::to_vec(&json!({"events":[{
                "name":"changed","direction":"outgoing","schemaVersion":1,
                "payloadSchema":{"type":"object","additionalProperties":false,
                    "required":["text"],"properties":{"text":{"type":"string"}}}
            }]}))
            .unwrap(),
        ),
    ]);
    let bytes = pack(
        &manifest,
        &files,
        &SigningKey::from_bytes(&[71; 32]),
        &Limits::default(),
    )
    .unwrap();
    let trust = store
        .trusted_app_publisher("alice", "com.example", "state-key")
        .unwrap();
    let package = verify(&bytes, &VerificationPolicy::new(291, vec![publisher()])).unwrap();
    let candidate = VerifiedInstallCandidate::from_verified(&package, &trust).unwrap();
    let AppRegistryOutcome::Update(record) = store
        .mutate_verified_app_installation(
            "alice",
            AppVerifiedInstallationMutation::CreateAndStage {
                installation_id: "installed".into(),
                candidate,
                now_ms: 1,
            },
        )
        .unwrap()
    else {
        panic!("expected stage")
    };
    let binding = {
        let mut connection = store.connection.lock().unwrap();
        InstallationRegistry::new(&mut connection)
            .staged_trust("alice", &record.token)
            .unwrap()
    };
    for operation in [
        AppRegistryMutation::Decide {
            token: record.token.clone(),
            decision: CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: "state-grant".into(),
                    authority_ref: "kernel-fixture".into(),
                },
            },
            now_ms: 2,
        },
        AppRegistryMutation::Quiesce {
            token: record.token.clone(),
            now_ms: 3,
        },
        AppRegistryMutation::MarkPrepared {
            token: record.token.clone(),
            now_ms: 4,
        },
    ] {
        store.mutate_app_installation("alice", operation).unwrap();
    }
    store
        .mutate_verified_app_installation(
            "alice",
            AppVerifiedInstallationMutation::Commit {
                token: record.token,
                now_ms: 5,
            },
        )
        .unwrap();
    Arc::new(
        EventCatalog::compile(
            &package,
            Arc::new(AppCatalog::compile(&package, &binding, &trust).unwrap()),
        )
        .unwrap(),
    )
}
fn budget() -> AppOperationBudget {
    AppOperationBudget::fixture(
        tokio::time::Instant::now() + Duration::from_secs(30),
        || false,
    )
}
fn put(value: i32) -> AppStateOperation {
    AppStateOperation::Transaction {
        changes: StateChanges::new(
            0,
            vec![],
            vec![StateWrite::Put {
                key: "count".into(),
                value: json!(value),
            }],
        )
        .unwrap(),
        occurrences: Vec::new(),
    }
}
fn read() -> AppStateOperation {
    AppStateOperation::Get {
        key: "count".into(),
    }
}

#[test]
fn state_writer_uses_its_own_connection_and_persists_verified_values() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = catalog(&store);
    let held_reader = store.connection.lock().unwrap();
    let worker_store = store.clone();
    let worker_catalog = Arc::clone(&catalog);
    let (send, receive) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ =
            send.send(worker_store.execute_app_state("alice", worker_catalog, put(7), budget()));
    });
    let result = receive.recv_timeout(Duration::from_secs(5));
    drop(held_reader);
    worker.join().unwrap();
    assert_eq!(
        result.unwrap().unwrap(),
        AppStateOutcome::Transaction {
            revision: 1,
            receipts: Vec::new()
        }
    );
    assert_eq!(
        store
            .execute_app_state("alice", Arc::clone(&catalog), read(), budget())
            .unwrap(),
        AppStateOutcome::Value(Some(StateRecord {
            value: json!(7),
            version: 1
        }))
    );
    assert!(store
        .execute_app_state("bob", Arc::clone(&catalog), put(8), budget())
        .is_err());
    drop(store);
    let reopened = fixture.open();
    assert_eq!(
        reopened
            .execute_app_state("alice", catalog, read(), budget())
            .unwrap(),
        AppStateOutcome::Value(Some(StateRecord {
            value: json!(7),
            version: 1
        }))
    );
}

#[test]
fn revoked_signer_blocks_state_reads_and_writes_without_changing_generation_or_data() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = catalog(&store);
    store
        .execute_app_state("alice", Arc::clone(&catalog), put(7), budget())
        .unwrap();
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: 1,
                decision: decision("revoke"),
                now_ms: 10,
            },
        )
        .unwrap();
    assert!(store
        .execute_app_state("alice", Arc::clone(&catalog), read(), budget())
        .is_err());
    assert!(store
        .execute_app_state("alice", Arc::clone(&catalog), put(8), budget())
        .is_err());
    assert_eq!(
        store
            .get_app_installation("alice", "installed")
            .unwrap()
            .generation,
        1
    );
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Enroll {
                publisher: publisher(),
                expected_revision: 2,
                decision: decision("reenroll"),
                now_ms: 11,
            },
        )
        .unwrap();
    assert!(store
        .execute_app_state("alice", Arc::clone(&catalog), read(), budget())
        .is_err());
    let connection = store.connection.lock().unwrap();
    let saved: (String,i64)=connection.query_row("SELECT value_json,version FROM app_state_values WHERE installation_id='installed' AND key='count'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(saved, ("7".into(), 1));
}

#[test]
fn failed_state_head_publication_rolls_back_values_and_writer_continues() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = catalog(&store);
    let connection = Connection::open(store.path()).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_app_state_head BEFORE INSERT ON app_state_heads BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;").unwrap();
    assert!(matches!(
        store.execute_app_state("alice", Arc::clone(&catalog), put(1), budget()),
        Err(AppStateError::State(StateError::Database(_)))
    ));
    assert_eq!(
        store
            .execute_app_state("alice", Arc::clone(&catalog), read(), budget())
            .unwrap(),
        AppStateOutcome::Value(None)
    );
    connection
        .execute_batch("DROP TRIGGER fail_app_state_head;")
        .unwrap();
    assert_eq!(
        store
            .execute_app_state("alice", catalog, put(2), budget())
            .unwrap(),
        AppStateOutcome::Transaction {
            revision: 1,
            receipts: Vec::new()
        }
    );
}

#[test]
fn cancelled_work_waiting_in_the_writer_queue_never_starts_a_state_change() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = catalog(&store);
    let mut blocker = Connection::open(store.path()).unwrap();
    let held = blocker
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let (started, receive_started) = mpsc::channel();
    let checks = AtomicUsize::new(0);
    let first_budget = AppOperationBudget::fixture(
        tokio::time::Instant::now() + Duration::from_secs(30),
        move || {
            if checks.fetch_add(1, Ordering::SeqCst) == 1 {
                let _ = started.send(());
            }
            false
        },
    );
    let first_store = store.clone();
    let first_catalog = Arc::clone(&catalog);
    let first = std::thread::spawn(move || {
        first_store.execute_app_state("alice", first_catalog, put(7), first_budget)
    });
    // The writer has dequeued its first request and is about to wait for the
    // actual SQLite lock. Hold that lock while adding and cancelling the next.
    let started = receive_started.recv_timeout(Duration::from_secs(5));
    if started.is_err() {
        drop(held);
        first.join().unwrap().unwrap();
        panic!("writer did not start within fixture deadline");
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    let (response, receive) = mpsc::channel();
    store
        .writer
        .enqueue(DurableWriterRequest::AppState(Box::new(AppStateRequest {
            owner: "alice".into(),
            catalog: Arc::clone(&catalog),
            operation: put(9),
            budget: AppOperationBudget::fixture(
                tokio::time::Instant::now() + Duration::from_secs(30),
                move || flag.load(Ordering::Acquire),
            ),
            response,
        })))
        .unwrap();
    cancelled.store(true, Ordering::Release);
    drop(held);
    assert_eq!(
        first.join().unwrap().unwrap(),
        AppStateOutcome::Transaction {
            revision: 1,
            receipts: Vec::new()
        }
    );
    assert!(matches!(
        receive.recv_timeout(Duration::from_secs(5)).unwrap(),
        Err(AppStateError::Stopped(AppOperationStopped::Cancelled))
    ));
    assert_eq!(
        store
            .execute_app_state("alice", catalog, read(), budget())
            .unwrap(),
        AppStateOutcome::Value(Some(StateRecord {
            value: json!(7),
            version: 1
        }))
    );
}

#[test]
fn expired_after_sqlite_admission_does_not_change_state() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = catalog(&store);
    let mut blocker = Connection::open(store.path()).unwrap();
    let held = blocker
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let (started, receive_started) = mpsc::channel();
    let checks = AtomicUsize::new(0);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    let request_budget = AppOperationBudget::fixture(deadline, move || {
        if checks.fetch_add(1, Ordering::SeqCst) == 1 {
            let _ = started.send(());
        }
        false
    });
    let writer_store = store.clone();
    let writer_catalog = Arc::clone(&catalog);
    let writer = std::thread::spawn(move || {
        writer_store.execute_app_state("alice", writer_catalog, put(1), request_budget)
    });
    let started = receive_started.recv_timeout(Duration::from_secs(5));
    if started.is_err() {
        drop(held);
        let _ = writer.join();
        panic!("writer did not reach SQLite admission within fixture deadline");
    }
    // This is a real SQLite wait. The kernel's second expiry check must reject
    // after BEGIN succeeds, even though the writer had admitted a live request.
    std::thread::sleep(
        deadline.saturating_duration_since(tokio::time::Instant::now()) + Duration::from_millis(5),
    );
    drop(held);
    assert!(matches!(
        writer.join().unwrap(),
        Err(AppStateError::Stopped(AppOperationStopped::Deadline))
    ));
    assert_eq!(
        store
            .execute_app_state("alice", catalog, read(), budget())
            .unwrap(),
        AppStateOutcome::Value(None)
    );
}
