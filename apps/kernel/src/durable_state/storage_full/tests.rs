use super::*;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let path = std::env::temp_dir().join(format!(
        "chariox-storage-full-{name}-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir(&path).unwrap();
    Scratch(path)
}

/// A store whose writer connection cannot grow the database past a few more
/// pages: SQLite then fails page allocation with `SQLITE_FULL`, as it does on a
/// full disk. Freeing rows from another connection gives it space again.
fn store_with_page_limit(scratch: &Scratch) -> DurableKernelStateStore {
    let mut store = DurableKernelStateStore::open(scratch.0.join("kernel.db")).unwrap();
    limit_writer_pages(&mut store);
    store
}

/// Replaces the store's writer with one whose connection may add only 64
/// more pages to the database.
pub(crate) fn limit_writer_pages(store: &mut DurableKernelStateStore) {
    let (sender, receiver) = mpsc::sync_channel(16);
    let health = Arc::new(DurableWriterHealth::default());
    let observed = health.clone();
    let connection = Connection::open(&store.path).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    let pages: i64 = connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    connection
        .pragma_update(None, "max_page_count", pages + 64)
        .unwrap();
    let worker = std::thread::spawn(move || {
        run_durable_writer(connection, receiver, observed, Duration::ZERO);
    });
    store.writer = Arc::new(DurableStateWriter {
        sender: Mutex::new(Some(sender)),
        worker: Mutex::new(Some(worker)),
        health,
    });
}

/// Fills the page-limited database until even a small row finds no page.
pub(crate) fn fill(store: &DurableKernelStateStore) {
    for bytes in [8 * 1024, 1024, 64] {
        let payload = serde_json::json!({ "filler": "x".repeat(bytes) });
        let mut accepted = 0;
        while store.append_event("filler", None, payload.clone()).is_ok() {
            accepted += 1;
            assert!(
                accepted < 10_000,
                "the page limit never filled the database"
            );
        }
    }
}

/// Frees the filler rows from another connection, as an owner frees disk space.
pub(crate) fn free(store: &DurableKernelStateStore) {
    let other = Connection::open(store.path()).unwrap();
    other.busy_timeout(Duration::from_secs(5)).unwrap();
    other
        .execute("DELETE FROM durable_state_events WHERE kind = 'filler'", [])
        .unwrap();
}

pub(crate) fn wait_for(store: &DurableKernelStateStore, condition: DurableWriterCondition) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while store.writer_condition() != condition && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(store.writer_condition(), condition);
}

#[test]
fn a_full_disk_fails_only_writes_and_the_writer_resumes_once_a_probe_commits() {
    let scratch = scratch("resume");
    let store = store_with_page_limit(&scratch);
    let payload = serde_json::json!({ "filler": "x".repeat(8 * 1024) });
    let mut accepted = 0;
    let full = loop {
        match store.append_event("filler", None, payload.clone()) {
            Ok(_) => accepted += 1,
            Err(error) => break error,
        }
        assert!(accepted < 1_000, "the page limit never filled the database");
    };
    assert!(accepted > 0);
    assert!(full.to_string().contains("full"), "{full}");

    // Not fenced: runtime admission and reads go on, and only writes fail.
    wait_for(&store, DurableWriterCondition::StorageFull);
    store.require_writer_healthy().unwrap();
    assert_eq!(store.load_events_by_kind("filler").unwrap().len(), accepted);
    assert!(store.append_event("filler", None, payload.clone()).is_err());
    assert_eq!(
        store.take_writer_condition_change(),
        Some(DurableWriterCondition::StorageFull)
    );
    assert_eq!(store.take_writer_condition_change(), None);

    // Probes keep failing while there is no space. Then the owner frees space.
    std::thread::sleep(PROBE_BASE_DELAY * 3);
    assert_eq!(
        store.writer_condition(),
        DurableWriterCondition::StorageFull
    );
    let other = Connection::open(store.path()).unwrap();
    other.busy_timeout(Duration::from_secs(5)).unwrap();
    other
        .execute("DELETE FROM durable_state_events WHERE kind = 'filler'", [])
        .unwrap();
    drop(other);

    wait_for(&store, DurableWriterCondition::Writable);
    assert_eq!(
        store.take_writer_condition_change(),
        Some(DurableWriterCondition::Writable)
    );
    store.append_event("after", None, payload).unwrap();
    assert_eq!(store.load_events_by_kind("after").unwrap().len(), 1);
}

#[test]
fn a_busy_writer_still_probes_between_requests() {
    let scratch = scratch("busy");
    let store = store_with_page_limit(&scratch);
    let payload = serde_json::json!({ "filler": "x".repeat(8 * 1024) });
    while store.append_event("filler", None, payload.clone()).is_ok() {}
    wait_for(&store, DurableWriterCondition::StorageFull);
    let other = Connection::open(store.path()).unwrap();
    other.busy_timeout(Duration::from_secs(5)).unwrap();
    other
        .execute("DELETE FROM durable_state_events WHERE kind = 'filler'", [])
        .unwrap();
    drop(other);
    // A steady stream of small writes never leaves the writer idle long
    // enough to time out waiting for a request.
    let deadline = Instant::now() + Duration::from_secs(10);
    while store.writer_condition() != DurableWriterCondition::Writable && Instant::now() < deadline
    {
        let _ = store.append_event("tick", None, serde_json::json!({}));
        std::thread::sleep(PROBE_BASE_DELAY / 10);
    }
    assert_eq!(store.writer_condition(), DurableWriterCondition::Writable);
}

#[test]
fn only_a_full_disk_is_a_known_failed_commit() {
    let full =
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL), None);
    let io = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR_FSYNC),
        None,
    );
    let shm = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR_SHMSIZE),
        None,
    );
    assert!(is_storage_full(&full));
    assert!(!is_storage_full(&io));
    assert!(!is_storage_full(&shm));
    assert!(!take_observed());
    assert!(!observe(&io));
    assert!(!take_observed());
    assert!(observe(&full));
    assert!(take_observed());
    assert!(!take_observed());
}

#[test]
fn a_fence_is_a_stop_the_owner_hears_about_but_a_shutdown_is_not() {
    let scratch = scratch("fence");
    let store = DurableKernelStateStore::open(scratch.0.join("kernel.db")).unwrap();
    assert_eq!(store.writer_condition(), DurableWriterCondition::Writable);
    assert_eq!(store.take_writer_condition_change(), None);
    store.fence_writer().unwrap();
    assert_eq!(store.writer_condition(), DurableWriterCondition::Stopped);
    assert_eq!(
        store.take_writer_condition_change(),
        Some(DurableWriterCondition::Stopped)
    );

    let other = DurableKernelStateStore::open(scratch.0.join("other.db")).unwrap();
    other.writer.sender.lock().unwrap().take();
    if let Some(worker) = other.writer.worker.lock().unwrap().take() {
        worker.join().unwrap();
    }
    assert!(other.require_writer_healthy().is_err());
    assert_eq!(other.writer_condition(), DurableWriterCondition::Writable);
}

#[test]
fn probes_back_off_to_a_cap() {
    assert_eq!(probe_delay(1), PROBE_BASE_DELAY * 2);
    assert_eq!(probe_delay(2), PROBE_BASE_DELAY * 4);
    assert_eq!(probe_delay(20), PROBE_MAX_DELAY);
    assert_eq!(probe_delay(u32::MAX), PROBE_MAX_DELAY);
}
