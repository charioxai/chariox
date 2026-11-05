//! R2 protocol drills through the kernel's actual peer and the R3 subprocess.
use super::{code, support::Worker};
use chariox_app_runtime::{
    wire::{Message, Outcome},
    worker_peer::{Broker, BrokerFuture, BrokerRequest, CallResponse, WorkerPeer},
};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{mpsc, Semaphore};

const BUDGET: Duration = Duration::from_secs(30);
struct Gate {
    started: mpsc::UnboundedSender<String>,
    release: Arc<Semaphore>,
    active: Arc<AtomicUsize>,
    streams: Arc<AtomicUsize>,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Broker for Gate {
    fn handle(&self, mut request: BrokerRequest) -> BrokerFuture {
        let started = self.started.clone();
        let release = self.release.clone();
        let active = self.active.clone();
        let streams = self.streams.clone();
        Box::pin(async move {
            match request.method.as_str() {
                "state.get" | "http.read" => {
                    active.fetch_add(1, Ordering::SeqCst);
                    let _active = Active(active);
                    started.send(request.method.clone()).unwrap();
                    tokio::select! {
                        biased;
                        _ = request.cancellation.cancelled() => Err(chariox_app_wasm_worker::ipc::error("CANCELLED", "cancelled")),
                        permit = release.acquire() => { permit.unwrap().forget(); Ok(Value::Null) },
                    }
                }
                "http.open" => {
                    streams.fetch_add(1, Ordering::SeqCst);
                    Ok(json!({"streamId":"stream-1"}))
                }
                "http.headers" => {
                    Ok(json!({"status":200,"headers":[],"url":"https://example.com/"}))
                }
                "http.cancel" => {
                    assert_eq!(streams.fetch_sub(1, Ordering::SeqCst), 1);
                    Ok(Value::Null)
                }
                other => panic!("unexpected broker method {other}"),
            }
        })
    }
}
fn new_gate() -> (Arc<Gate>, mpsc::UnboundedReceiver<String>) {
    let (started, receiver) = mpsc::unbounded_channel();
    (
        Arc::new(Gate {
            started,
            release: Arc::new(Semaphore::new(0)),
            active: Arc::default(),
            streams: Arc::default(),
        }),
        receiver,
    )
}
pub(super) fn submit(peer: &WorkerPeer, name: &str) -> CallResponse {
    let slot = peer.reserve(BUDGET).unwrap();
    let message = Message::Request {
        version: 1,
        generation: slot.generation().into(),
        id: slot.id().into(),
        method: "tools.invoke".into(),
        params: json!({"name":name,"input":{}}),
        deadline_ms: slot.deadline_ms(),
        context: Some(json!({"actor":{"kind":"human","id":"broker-drill"}})),
    };
    slot.submit(message).unwrap()
}
pub(super) async fn outcome(call: CallResponse) -> Outcome {
    let Message::Response { outcome, .. } =
        tokio::time::timeout(BUDGET, call).await.unwrap().unwrap()
    else {
        panic!("response required")
    };
    outcome
}
async fn started(receiver: &mut mpsc::UnboundedReceiver<String>) -> String {
    tokio::time::timeout(BUDGET, receiver.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source App build required; use scripts/drill.sh"]
async fn kernel_peer_real_worker_64_running_16_queued_and_neighbour() {
    let mut worker = Worker::start(vec![]).await;
    let (gate, mut seen) = new_gate();
    let (peer, _events, task) = worker.peer(gate.clone());
    let calls = (0..80).map(|_| submit(&peer, "shared")).collect::<Vec<_>>();
    assert!(matches!(
        peer.reserve(BUDGET),
        Err(chariox_app_runtime::worker_peer::PeerError::Busy)
    ));
    for _ in 0..64 {
        assert_eq!(started(&mut seen).await, "state.get");
    }
    assert_eq!(gate.active.load(Ordering::SeqCst), 64);
    assert!(seen.try_recv().is_err());
    let mut neighbour = Worker::start(vec![]).await;
    let (other_gate, _) = new_gate();
    let (other_peer, _other_events, other_task) = neighbour.peer(other_gate);
    assert!(matches!(
        outcome(submit(&other_peer, "echo")).await,
        Outcome::Success(_)
    ));
    tokio::time::timeout(
        Duration::from_millis(250),
        tokio::time::sleep(Duration::from_millis(10)),
    )
    .await
    .unwrap();
    gate.release.add_permits(80);
    let mut counts = std::collections::BTreeSet::new();
    for call in calls {
        let result = outcome(call).await;
        let Outcome::Success(success) = result else {
            panic!("queued work failed: {result:?}")
        };
        counts.insert(success.result["count"].as_u64().unwrap());
    }
    assert_eq!(counts, (1..=80).collect());
    assert_eq!(gate.active.load(Ordering::SeqCst), 0);
    assert!(matches!(
        outcome(submit(&peer, "echo")).await,
        Outcome::Success(_)
    ));
    peer.close();
    other_peer.close();
    task.join().await.unwrap();
    other_task.join().await.unwrap();
    println!("R2 DRILL 64 admitted / 16 queued / BUSY / shared state / neighbour: PASS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source App build required; use scripts/drill.sh"]
async fn kernel_peer_saturated_cancellation_releases_slots_without_replaying_effects() {
    let mut worker = Worker::start(vec![]).await;
    let (gate, mut seen) = new_gate();
    let (peer, _events, task) = worker.peer(gate.clone());
    let mut calls = (0..80).map(|_| submit(&peer, "shared")).collect::<Vec<_>>();
    for _ in 0..64 {
        assert_eq!(started(&mut seen).await, "state.get");
    }
    drop(calls.remove(0));
    assert_eq!(
        started(&mut seen).await,
        "state.get",
        "cancellation must free execution admission for a queued call"
    );
    assert!(gate.active.load(Ordering::SeqCst) <= 64);
    gate.release.add_permits(80);
    let mut lost = 0;
    for call in calls {
        match outcome(call).await {
            Outcome::Success(_) => {}
            Outcome::Failure(failure) => {
                assert_eq!(failure.error.code, "APP_INSTANCE_RESTARTED");
                assert!(failure.error.message.contains("culprit: host-1"));
                assert_eq!(failure.error.retryable, Some(false));
                lost += 1;
            }
        }
    }
    // The first 64 requests and exactly 16 newly admitted waiters; a shared
    // instance replacement must not silently repeat any non-idempotent call.
    for _ in 0..15 {
        assert_eq!(started(&mut seen).await, "state.get");
    }
    assert!(seen.try_recv().is_err());
    assert_eq!(gate.active.load(Ordering::SeqCst), 0);
    assert!(matches!(
        outcome(submit(&peer, "echo")).await,
        Outcome::Success(_)
    ));
    peer.close();
    task.join().await.unwrap();
    println!("R2 DRILL saturated cancellation releases all resources; {lost} attributed instance-loss results; no non-idempotent replay: PASS");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "source App build required; use scripts/drill.sh"]
async fn kernel_peer_real_worker_stream_cancellation_and_single_manifest_replay() {
    let mut worker = Worker::start(vec!["wait"]).await;
    let (gate, mut seen) = new_gate();
    let (peer, _events, task) = worker.peer(gate.clone());
    let stream = submit(&peer, "slow_stream");
    let mut methods = std::collections::BTreeSet::new();
    for _ in 0..2 {
        methods.insert(started(&mut seen).await);
    }
    assert_eq!(methods, ["http.read".into(), "state.get".into()].into());
    assert_eq!(gate.streams.load(Ordering::SeqCst), 1);
    drop(stream);
    tokio::time::timeout(BUDGET, async {
        while gate.streams.load(Ordering::SeqCst) != 0 || gate.active.load(Ordering::SeqCst) != 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let replay = submit(&peer, "wait");
    assert_eq!(started(&mut seen).await, "state.get");
    let non_idempotent = submit(&peer, "shared");
    assert_eq!(started(&mut seen).await, "state.get");
    assert_eq!(code(outcome(submit(&peer, "trap")).await), "APP_TRAPPED");
    let Outcome::Failure(failure) = outcome(non_idempotent).await else {
        panic!("instance loss required")
    };
    assert_eq!(failure.error.code, "APP_INSTANCE_RESTARTED");
    assert!(failure.error.message.contains("host-"));
    assert_eq!(failure.error.retryable, Some(false));
    assert_eq!(started(&mut seen).await, "state.get");
    assert!(
        seen.try_recv().is_err(),
        "non-idempotent call must never replay"
    );
    assert_eq!(code(outcome(submit(&peer, "trap")).await), "APP_TRAPPED");
    assert_eq!(code(outcome(replay).await), "APP_INSTANCE_RESTARTED");
    assert!(
        seen.try_recv().is_err(),
        "manifest-declared replay happens at most once"
    );
    assert!(matches!(
        outcome(submit(&peer, "echo")).await,
        Outcome::Success(_)
    ));
    peer.close();
    task.join().await.unwrap();
    println!("R2 DRILL stream/slot release, trap attribution, only one manifest replay: PASS");
}
