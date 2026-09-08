use super::super::*;
use crate::durable_state::{app_publishers::AppPublisherMutation, app_state::fixture_catalog};
use chariox_app_runtime::{
    publisher_trust::TrustDecision,
    wire::{Channel, Message, Outcome, Sender, WIRE_VERSION},
    worker_peer::{Broker, BrokerFuture, PeerLimits, WorkerPeer},
};
use std::{path::PathBuf, time::Duration};
use tokio::{io::DuplexStream, time::timeout};

const WAIT: Duration = Duration::from_secs(2);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-app-state-broker-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
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
struct Delegate(AppStateBroker);
impl Broker for Delegate {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        let service = self.0.clone();
        Box::pin(async move { service.dispatch(request).await })
    }
}
fn start(
    store: &DurableKernelStateStore,
    catalog: Arc<AppCatalog>,
    owner: &str,
    admission: Arc<Semaphore>,
) -> (
    WorkerPeer,
    Channel<DuplexStream>,
    chariox_app_runtime::worker_peer::PeerTask,
) {
    start_observed(store, catalog, owner, admission, None)
}
fn start_observed(
    store: &DurableKernelStateStore,
    catalog: Arc<AppCatalog>,
    owner: &str,
    admission: Arc<Semaphore>,
    observe: Option<Arc<dyn Fn() + Send + Sync>>,
) -> (
    WorkerPeer,
    Channel<DuplexStream>,
    chariox_app_runtime::worker_peer::PeerTask,
) {
    let (host, worker) = tokio::io::duplex(64 * 1024);
    let mut service = AppStateBroker::new(store.clone(), owner.into(), catalog, admission);
    service.budget_observer = observe;
    let broker = Arc::new(Delegate(service));
    let (peer, _events, task) = WorkerPeer::start(
        Channel::new(host, "1".into(), Sender::Worker).unwrap(),
        broker,
        PeerLimits::default(),
    )
    .unwrap();
    (
        peer,
        Channel::new(worker, "1".into(), Sender::Supervisor).unwrap(),
        task,
    )
}
async fn send(worker: &mut Channel<DuplexStream>, id: &str, method: &str, params: Value) {
    let message = Message::Request {
        version: WIRE_VERSION,
        generation: "1".into(),
        id: id.into(),
        method: method.into(),
        params,
        deadline_ms: crate::session::unix_epoch_ms() + 5_000,
        context: None,
    };
    timeout(WAIT, worker.send(&message, WAIT))
        .await
        .unwrap()
        .unwrap();
}
async fn receive(worker: &mut Channel<DuplexStream>) -> Result<Value, String> {
    match timeout(WAIT, worker.receive(WAIT)).await.unwrap().unwrap() {
        Message::Response {
            outcome: Outcome::Success(success),
            ..
        } => Ok(success.result),
        Message::Response {
            outcome: Outcome::Failure(failure),
            ..
        } => Err(failure.error.code),
        _ => panic!("state response required"),
    }
}
fn change(value: Value) -> Value {
    json!({"schemaVersion":0,"checks":[],"writes":[{"key":"status","value":value}]})
}

#[tokio::test]
async fn actual_sdk_peer_uses_verified_state_and_returns_exact_shapes_without_dropping_occurrences()
{
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = fixture_catalog(&store);
    let (peer, mut worker, task) = start(&store, catalog, "alice", Arc::new(Semaphore::new(8)));
    send(
        &mut worker,
        "read-absent",
        "state.get",
        json!({"key":"status"}),
    )
    .await;
    assert_eq!(receive(&mut worker).await.unwrap(), Value::Null);
    send(
        &mut worker,
        "write",
        "state.transaction",
        change(json!({"text":"saved"})),
    )
    .await;
    assert_eq!(
        receive(&mut worker).await.unwrap(),
        json!({"revision":1,"receipts":[]})
    );
    send(&mut worker, "read", "state.get", json!({"key":"status"})).await;
    assert_eq!(
        receive(&mut worker).await.unwrap(),
        json!({"value":{"text":"saved"},"version":1})
    );
    let mut conflict = change(json!("wrong"));
    conflict["checks"] = json!([{"key":"status","version":2}]);
    send(&mut worker, "conflict", "state.transaction", conflict).await;
    assert_eq!(receive(&mut worker).await.unwrap_err(), "CONFLICT");
    let mut occurrence = change(json!("must not commit"));
    occurrence["occurrences"] =
        json!([{"automationId":"a","occurrenceId":"event","eventVersion":1,"payload":{}}]);
    send(&mut worker, "unsupported", "state.transaction", occurrence).await;
    assert_eq!(
        receive(&mut worker).await.unwrap_err(),
        "UNSUPPORTED_OPERATION"
    );
    send(
        &mut worker,
        "spoof",
        "state.get",
        json!({"key":"status","owner":"bob"}),
    )
    .await;
    assert_eq!(receive(&mut worker).await.unwrap_err(), "INVALID_ARGUMENT");
    send(
        &mut worker,
        "read-again",
        "state.get",
        json!({"key":"status"}),
    )
    .await;
    assert_eq!(
        receive(&mut worker).await.unwrap(),
        json!({"value":{"text":"saved"},"version":1})
    );
    peer.close();
    timeout(WAIT, task.join()).await.unwrap().unwrap();
}

#[tokio::test]
async fn current_publisher_and_captured_owner_are_checked_through_the_actual_peer_and_writer() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = fixture_catalog(&store);
    let admission = Arc::new(Semaphore::new(8));
    let (wrong_peer, mut wrong_worker, wrong_task) =
        start(&store, catalog.clone(), "bob", admission.clone());
    send(
        &mut wrong_worker,
        "wrong-owner",
        "state.get",
        json!({"key":"status"}),
    )
    .await;
    assert_eq!(
        receive(&mut wrong_worker).await.unwrap_err(),
        "APP_UNAVAILABLE"
    );
    wrong_peer.close();
    timeout(WAIT, wrong_task.join()).await.unwrap().unwrap();
    let (peer, mut worker, task) = start(&store, catalog, "alice", admission);
    let trust = store
        .trusted_app_publisher("alice", "com.example", "state-key")
        .unwrap();
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: trust.revision(),
                decision: TrustDecision {
                    decision_id: "broker-revoke".into(),
                    authority_ref: "kernel-fixture".into(),
                },
                now_ms: 20,
            },
        )
        .unwrap();
    send(
        &mut worker,
        "revoked",
        "state.transaction",
        change(json!("must not commit")),
    )
    .await;
    assert_eq!(receive(&mut worker).await.unwrap_err(), "APP_UNAVAILABLE");
    peer.close();
    timeout(WAIT, task.join()).await.unwrap().unwrap();
}

async fn permits(admission: &Semaphore, count: usize) {
    timeout(WAIT, async {
        while admission.available_permits() != count {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn shared_admission_survives_cancellation_until_the_blocked_writer_rechecks_its_budget() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = fixture_catalog(&store);
    let admission = Arc::new(Semaphore::new(8));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started = Mutex::new(Some(started_tx));
    let checks = AtomicUsize::new(0);
    let observer = Arc::new(move || {
        // Ignore the initial BUSY request's one decoder-entry check. For the
        // mutation, observe dispatch, blocking service, then writer admission.
        if checks.fetch_add(1, Ordering::SeqCst) == 3 {
            let _ = started.lock().unwrap().take().unwrap().send(());
        }
    });
    let (peer, mut worker, task) =
        start_observed(&store, catalog, "alice", admission.clone(), Some(observer));
    let held = admission.clone().acquire_many_owned(8).await.unwrap();
    send(&mut worker, "busy", "state.get", json!({"key":"status"})).await;
    assert_eq!(receive(&mut worker).await.unwrap_err(), "BUSY");
    drop(held);
    let blocker = rusqlite::Connection::open(store.path()).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    send(
        &mut worker,
        "blocked",
        "state.transaction",
        change(json!("must not commit")),
    )
    .await;
    // The test-only observer preserves the actual deadline/cancellation value.
    // It locates the real writer immediately before its SQLite BEGIN wait; it
    // neither changes admission nor replaces the transaction with a fake.
    if !matches!(timeout(WAIT, started_rx).await, Ok(Ok(()))) {
        blocker.execute_batch("ROLLBACK").unwrap();
        peer.close();
        let _ = timeout(WAIT, task.join()).await;
        panic!("state writer did not reach the held SQLite lock");
    }
    let admitted = admission.available_permits();
    let cancel_sent = timeout(
        WAIT,
        worker.send(
            &Message::Cancel {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: "blocked".into(),
            },
            WAIT,
        ),
    )
    .await;
    let acknowledged = if matches!(cancel_sent, Ok(Ok(()))) {
        matches!(timeout(WAIT,worker.receive(WAIT)).await,Ok(Ok(Message::Response{outcome:Outcome::Failure(failure),..})) if failure.error.code=="CANCELLED")
    } else {
        false
    };
    // An acknowledged IPC cancellation cannot release the blocking writer's
    // permit while SQLite BEGIN is still waiting for the external lock.
    tokio::time::sleep(Duration::from_millis(20)).await;
    let retained = admission.available_permits();
    blocker.execute_batch("ROLLBACK").unwrap();
    permits(&admission, 8).await;
    if admitted != 7 || retained != 7 || !acknowledged {
        peer.close();
        let _ = timeout(WAIT, task.join()).await;
        panic!("cancelled broker did not retain writer admission through the actual SQLite wait");
    }
    send(
        &mut worker,
        "not-written",
        "state.get",
        json!({"key":"status"}),
    )
    .await;
    assert_eq!(receive(&mut worker).await.unwrap(), Value::Null);
    peer.close();
    timeout(WAIT, task.join()).await.unwrap().unwrap();
}
