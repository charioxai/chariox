//! Real component calls through WorkerPeer and the production state store.
//! Kernel trust/admission and the durable-writer adapter have their own drills.
use super::{
    broker::{outcome, submit},
    support::Worker,
};
use chariox_app_runtime::{
    installation::{CapabilityApproval, CapabilityDecision, InstallationRegistry, ReleaseMetadata},
    managed_state::{
        ManagedStateStore, StateChanges, StateCheck, StateError, StateScope, StateWrite,
    },
    wire::Outcome,
    worker_peer::{Broker, BrokerFuture, BrokerRequest},
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

struct Storage {
    database: Arc<Mutex<rusqlite::Connection>>,
    reads: AtomicUsize,
    conflicts: Arc<AtomicUsize>,
    first_reads: Arc<tokio::sync::Barrier>,
}
fn scope() -> StateScope<'static> {
    StateScope::new("alice", "counter-app", 1).unwrap()
}
impl Broker for Storage {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        let database = self.database.clone();
        let first = request.method == "state.get" && self.reads.fetch_add(1, Ordering::SeqCst) < 64;
        let barrier = self.first_reads.clone();
        let conflicts = self.conflicts.clone();
        Box::pin(async move {
            let result = tokio::task::spawn_blocking(move || {
                assert!(!request.cancellation.is_cancelled());
                let mut db = database.lock().unwrap();
                let mut store = ManagedStateStore::new(&mut db);
                match request.method.as_str() {
                    "state.get" => Ok(store
                        .get(scope(), request.params["key"].as_str().unwrap())
                        .unwrap()
                        .map(|record| json!({"value":record.value,"version":record.version}))
                        .unwrap_or(Value::Null)),
                    "state.transaction" => {
                        let checks = request.params["checks"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|check| StateCheck {
                                key: check["key"].as_str().unwrap().into(),
                                version: check["version"].as_u64(),
                            })
                            .collect();
                        let writes = request.params["writes"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|write| StateWrite::Put {
                                key: write["key"].as_str().unwrap().into(),
                                value: write["value"].clone(),
                            })
                            .collect();
                        let change = StateChanges::new(1, checks, writes).unwrap();
                        match store.transaction(scope(), &change) {
                            Ok(revision) => Ok(json!({"revision":revision,"receipts":[]})),
                            Err(StateError::Conflict) => {
                                conflicts.fetch_add(1, Ordering::SeqCst);
                                Err(chariox_app_wasm_worker::ipc::error(
                                    "CONFLICT",
                                    "retry the version check",
                                ))
                            }
                            Err(error) => panic!("unexpected state error: {error}"),
                        }
                    }
                    other => panic!("unexpected storage method {other}"),
                }
            })
            .await
            .unwrap();
            // All 64 reads observe the absent key before any transaction starts.
            if first {
                barrier.wait().await;
            }
            result
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source App build required; use scripts/drill.sh"]
async fn kernel_peer_real_worker_transactions_preserve_all_eighty_updates() {
    let root = std::path::PathBuf::from(std::env::var("CHARIOX_WORKER_DRILL_DIR").unwrap());
    let path = root.join(format!("state-{}.sqlite", std::process::id()));
    let mut db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .unwrap();
    InstallationRegistry::new(&mut db).initialize().unwrap();
    ManagedStateStore::new(&mut db).initialize().unwrap();
    let mut registry = InstallationRegistry::new(&mut db);
    let token = registry
        .create_and_stage(
            "counter-app",
            "alice",
            ReleaseMetadata {
                app_id: "com.example.counter".into(),
                version: "1.0.0".into(),
                publisher_id: "com.example".into(),
                package_digest: format!("sha256:{:064x}", 1),
                schema_version: 1,
                capabilities_digest: format!("sha256:{:064x}", 2),
                catalog_digest: format!("sha256:{:064x}", 3),
                view_digest: format!("sha256:{:064x}", 4),
            },
            1,
        )
        .unwrap()
        .token;
    registry
        .decide(
            &token,
            CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: "drill".into(),
                    authority_ref: "test-policy".into(),
                },
            },
            2,
        )
        .unwrap();
    registry.quiesce(&token, 3).unwrap();
    registry.mark_prepared(&token, 4).unwrap();
    registry.commit(&token, 5).unwrap();
    let store = Arc::new(Storage {
        database: Arc::new(Mutex::new(db)),
        reads: AtomicUsize::new(0),
        conflicts: Arc::default(),
        first_reads: Arc::new(tokio::sync::Barrier::new(64)),
    });
    let mut worker = Worker::start(vec![]).await;
    let (peer, _events, task) = worker.peer(store.clone());
    let calls = (0..80)
        .map(|_| submit(&peer, "increment"))
        .collect::<Vec<_>>();
    let mut counts = std::collections::BTreeSet::new();
    for call in calls {
        let Outcome::Success(success) = outcome(call).await else {
            panic!("transaction failed")
        };
        counts.insert(success.result.as_u64().unwrap());
    }
    assert_eq!(counts, (1..=80).collect());
    assert!(store.conflicts.load(Ordering::SeqCst) >= 63);
    peer.close();
    task.join().await.unwrap();
    drop(peer);
    drop(store);
    let mut reopened = rusqlite::Connection::open(&path).unwrap();
    let record = ManagedStateStore::new(&mut reopened)
        .get(scope(), "counter")
        .unwrap()
        .unwrap();
    assert_eq!(record.value, json!(80));
    assert_eq!(record.version, 80);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
    println!("R2 DRILL 80 concurrent CAS increments, real SQLite reopen, no lost updates: PASS");
}
