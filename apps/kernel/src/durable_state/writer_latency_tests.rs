//! Queue policy and worker release; absolute latency belongs to the live drill.
use super::*;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn scratch(name: &str) -> Scratch {
    let path = std::env::temp_dir().join(format!(
        "chariox-writer-latency-{name}-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir(&path).unwrap();
    Scratch(path)
}
fn request(index: usize) -> DurableWriteRequest {
    DurableWriteRequest {
        observation: None,
        operation: DurableWriteOperation::Event {
            event_id: format!("event-{index}"),
            kind: "test".into(),
            subject_id: None,
            timestamp_ms: 1,
            payload_json: "{}".into(),
        },
        response: mpsc::channel().0,
    }
}

#[test]
fn a_lone_durable_write_commits_without_a_batch_window() {
    let mut attempts = 0;
    let batch = collect_ready_durable_batch(request(0), || {
        attempts += 1;
        assert_eq!(
            attempts, 1,
            "an empty queue must end the batch, never poll for company"
        );
        None
    });
    assert_eq!(batch.len(), 1);
    assert_eq!(attempts, 1);
}

#[test]
fn a_durable_batch_collects_only_ready_writes_and_keeps_its_bound() {
    let mut ready = (1..3)
        .map(request)
        .collect::<std::collections::VecDeque<_>>();
    let batch = collect_ready_durable_batch(request(0), || ready.pop_front());
    assert_eq!(batch.len(), 3);
    let mut index = 0;
    let batch = collect_ready_durable_batch(request(0), || {
        index += 1;
        Some(request(index))
    });
    assert_eq!(batch.len(), DURABLE_WRITE_BATCH_LIMIT);
    assert_eq!(index, DURABLE_WRITE_BATCH_LIMIT - 1);
}

fn worker_progress_while_commit_is_held(force_blocked_worker: bool, typed: bool) -> bool {
    let scratch = scratch("worker");
    let mut store = DurableKernelStateStore::open(scratch.0.join("kernel.db")).unwrap();
    let (sender, receiver) = mpsc::sync_channel(16);
    let (release, stalled) = mpsc::channel::<()>();
    let (entered, committing) = mpsc::channel::<()>();
    let health = Arc::new(DurableWriterHealth::default());
    let observed = health.clone();
    let path = store.path.clone();
    let worker = std::thread::spawn(move || {
        let mut connection = Connection::open(path).unwrap();
        let first = receiver.recv().unwrap();
        entered.send(()).unwrap();
        stalled.recv().unwrap();
        match first {
            DurableWriterRequest::Ordinary(first) => {
                commit_durable_write_batch(&mut connection, vec![first], &observed)
            }
            DurableWriterRequest::AgentLifecycle(request) => {
                agent_lifecycle::execute(
                    &mut connection,
                    *request,
                    &agent_lifecycle::faults::AgentFaults::default(),
                    &Default::default(),
                );
            }
            _ => panic!("test request expected"),
        }
        run_durable_writer(connection, receiver, observed);
    });
    store.writer = Arc::new(DurableStateWriter {
        sender: Mutex::new(Some(sender)),
        worker: Mutex::new(Some(worker)),
        health,
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    // Negative control: entering a current-thread handle selects the existing
    // blocking fallback, reproducing a recv() held on the sole executor worker.
    let blocking_context = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let regression = force_blocked_worker.then(|| blocking_context.handle().clone());
    let writer = runtime.spawn(async move {
        let _entered = regression.as_ref().map(|handle| handle.enter());
        if typed {
            store
                .agent_lifecycle(agent_lifecycle::Operation::Outside(
                    agent_lifecycle::outside::Mutation::RunStates { states: Vec::new() },
                ))
                .map(|_| ())
        } else {
            store
                .append_event("slow", None, serde_json::json!({}))
                .map(|_| ())
        }
    });
    // This thread is outside Tokio: its deadline keeps advancing even when
    // the executor and its timer driver are blocked. Always release before
    // asserting, so the failing regression also cleans up.
    let entered = committing.recv_timeout(Duration::from_secs(5));
    let (progress, observed_progress) = mpsc::channel();
    let other = runtime.spawn(async move {
        let _ = progress.send(());
    });
    let ran_while_held = observed_progress
        .recv_timeout(Duration::from_secs(1))
        .is_ok();
    release.send(()).unwrap();
    runtime.block_on(writer).unwrap().unwrap();
    runtime.block_on(other).unwrap();
    entered.unwrap();
    ran_while_held
}

#[test]
fn a_slow_durable_commit_does_not_hold_its_runtime_worker() {
    assert!(
        worker_progress_while_commit_is_held(false, false),
        "other tasks must run while the commit is still held"
    );
}

#[test]
fn the_independent_watchdog_detects_a_blocked_runtime_worker() {
    assert!(
        !worker_progress_while_commit_is_held(true, false),
        "control must detect a blocked executor before releasing the commit"
    );
}

#[test]
fn a_typed_durable_commit_does_not_hold_its_runtime_worker() {
    assert!(
        worker_progress_while_commit_is_held(false, true),
        "typed writer waits must yield the async worker"
    );
    assert!(
        !worker_progress_while_commit_is_held(true, true),
        "negative control must detect the old typed recv path"
    );
}
