//! Fatal writer admission and joined shutdown for uncertain workflow scheduling.
use super::*;

impl DurableKernelStateStore {
    pub(crate) fn require_writer_healthy(&self) -> Result<(), DaemonError> {
        self.writer.require_healthy()
    }

    /// Blocking fatal shutdown, not transient write cancellation. Previously
    /// executing commits may finish; return waits for the writer to stop, and no
    /// queued or subsequent request can commit afterward. Restart reloads durable
    /// workflow/prompt intents before admitting new work.
    pub(crate) fn fence_writer(&self) -> Result<(), DaemonError> {
        self.writer.health.fatal.store(true, Ordering::Release);
        self.writer
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let mut worker = self
            .writer
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(worker) = worker.take() {
            worker.join().map_err(|_| fenced_writer_error())?;
        }
        Ok(())
    }
}

pub(super) fn fenced_writer_error() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "durable_state.fenced",
        message: "durable writer requires recovery before runtime admission".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fatal_fence_discards_queued_writes_and_waits_for_actual_writer_exit() {
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(std::env::temp_dir().join(format!(
            "chariox-writer-fence-{}-{}",
            std::process::id(),
            rand_suffix()
        )));
        fs::create_dir(&scratch.0).unwrap();
        let mut store = DurableKernelStateStore::open(scratch.0.join("kernel.db")).unwrap();
        let (sender, receiver) = mpsc::sync_channel(16);
        let (release, blocked) = mpsc::channel();
        let health = Arc::new(DurableWriterHealth::default());
        let observed = health.clone();
        let path = store.path.clone();
        let worker = std::thread::spawn(move || {
            let connection = Connection::open(path).unwrap();
            // Deterministic held writer, with a timeout so an assertion failure
            // cannot leave this test's owner waiting forever during Drop.
            let _ = blocked.recv_timeout(Duration::from_secs(5));
            run_durable_writer(connection, receiver, observed, Duration::ZERO);
        });
        store.writer = Arc::new(DurableStateWriter {
            sender: Mutex::new(Some(sender)),
            worker: Mutex::new(Some(worker)),
            health,
        });
        let mut replies = Vec::new();
        for index in 0..8 {
            let (response, reply) = mpsc::channel();
            store
                .writer
                .enqueue(DurableWriterRequest::Ordinary(DurableWriteRequest {
                    operation: DurableWriteOperation::Event {
                        event_id: format!("fenced-{index}"),
                        kind: "fence-test".into(),
                        subject_id: None,
                        timestamp_ms: 1,
                        payload_json: "{}".into(),
                    },
                    response,
                }))
                .unwrap();
            replies.push(reply);
        }
        let owned = store.clone();
        let fence = std::thread::spawn(move || owned.fence_writer());
        let deadline = Instant::now() + Duration::from_secs(2);
        while store.require_writer_healthy().is_ok() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(store.require_writer_healthy().is_err());
        assert!(!fence.is_finished());
        release.send(()).unwrap();
        fence.join().unwrap().unwrap();
        for reply in replies {
            assert!(matches!(
                reply.recv_timeout(Duration::from_secs(1)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ));
        }
        assert!(store
            .append_event("later", None, serde_json::json!({}))
            .is_err());
        let database = Connection::open(store.path()).unwrap();
        assert_eq!(
            database
                .query_row("SELECT count(*) FROM durable_state_events", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
        drop(database);
        drop(store);
        // Fatal state is process-local. Restart reopens the authoritative state.
        let restarted = DurableKernelStateStore::open(scratch.0.join("kernel.db")).unwrap();
        restarted.require_writer_healthy().unwrap();
        restarted
            .append_event("after-recovery", None, serde_json::json!({}))
            .unwrap();
    }
}
