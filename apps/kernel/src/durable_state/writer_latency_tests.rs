//! A durable commit costs one fsync: no batch window, no held runtime worker.
use super::*;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    // tmpfs where available, so the measured cost is the writer, not the disk.
    let root = Path::new("/dev/shm")
        .is_dir()
        .then(|| PathBuf::from("/dev/shm"))
        .unwrap_or_else(std::env::temp_dir);
    let path = root.join(format!(
        "chariox-writer-latency-{name}-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir(&path).unwrap();
    Scratch(path)
}

#[test]
fn a_lone_durable_write_commits_without_a_batch_window() {
    let scratch = scratch("lone");
    let store = DurableKernelStateStore::open(scratch.0.join("kernel.db")).unwrap();
    let mut elapsed = (0..10)
        .map(|index| {
            let started = Instant::now();
            store
                .append_event("lone", None, serde_json::json!({ "index": index }))
                .unwrap();
            started.elapsed()
        })
        .collect::<Vec<_>>();
    elapsed.sort();
    assert!(
        elapsed[elapsed.len() / 2] < Duration::from_millis(4),
        "a lone write waited for company: {elapsed:?}"
    );
    assert_eq!(store.writer_health_snapshot().max_batch_records, 1);
}

#[test]
fn a_slow_durable_commit_does_not_hold_its_runtime_worker() {
    let scratch = scratch("worker");
    let mut store = DurableKernelStateStore::open(scratch.0.join("kernel.db")).unwrap();
    // A writer that stalls like a commit behind a slow fsync until released.
    let (sender, receiver) = mpsc::sync_channel(16);
    let (release, stalled) = mpsc::channel::<()>();
    let health = Arc::new(DurableWriterHealth::default());
    let observed = health.clone();
    let path = store.path.clone();
    let worker = std::thread::spawn(move || {
        let connection = Connection::open(path).unwrap();
        let _ = stalled.recv_timeout(Duration::from_secs(5));
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
    runtime.block_on(async {
        let (appending, appended) = tokio::sync::oneshot::channel();
        let writer = tokio::spawn(async move {
            let _ = appending.send(());
            store.append_event("slow", None, serde_json::json!({}))
        });
        appended.await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let other = tokio::spawn(async { "ran" });
        let other = tokio::time::timeout(Duration::from_secs(1), other).await;
        release.send(()).unwrap();
        assert_eq!(
            other
                .expect("other tasks must run while a commit waits")
                .unwrap(),
            "ran"
        );
        writer.await.unwrap().unwrap();
    });
}
