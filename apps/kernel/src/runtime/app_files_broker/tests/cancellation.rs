use super::*;
use crate::durable_state::app_files::TestPublicationFault;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tokio::{sync::oneshot, time::timeout};

#[test]
fn cancelled_queued_file_keeps_admission_until_writer_drains_and_never_publishes() {
    let fixture = Fixture::new(Mode::Ready);
    fixture.runtime.block_on(async {
        let (started, start) = oneshot::channel();
        let started = Mutex::new(Some(started));
        let checks = AtomicUsize::new(0);
        let mut service = fixture.service();
        service.budget_observer = Some(Arc::new(move || {
            // Actual first request: decoder, blocking stage, staged budget,
            // writer enqueue, then writer apply just before real SQLite BEGIN.
            if checks.fetch_add(1, Ordering::SeqCst) == 4 {
                let _ = started.lock().unwrap().take().unwrap().send(());
            }
        }));
        let (queued, queue) = oneshot::channel();
        let queued = Mutex::new(Some(queued));
        let enqueues = AtomicUsize::new(0);
        let fault = Arc::new(TestPublicationFault {
            queued: Some(Arc::new(move || {
                if enqueues.fetch_add(1, Ordering::SeqCst) == 1 {
                    let _ = queued.lock().unwrap().take().unwrap().send(());
                }
            })),
            ..Default::default()
        });
        service.publication_fault = Some(fault.clone());
        let mut peer = TestPeer::start(service);
        let mut blocker = rusqlite::Connection::open(fixture.store.path()).unwrap();
        let held = blocker
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        peer.replace("first", b"first").await;
        timeout(WAIT, start).await.unwrap().unwrap();
        peer.replace("cancelled", b"must not publish").await;
        timeout(WAIT, queue).await.unwrap().unwrap();
        assert_eq!(fixture.admission.available_permits(), 6);
        assert_eq!(fixture.observed.pending_file_replacements().unwrap(), 2);
        peer.cancel("cancelled").await;
        let (id, error) = peer.response().await;
        assert_eq!(id, "cancelled");
        assert_eq!(error.unwrap_err().code, "CANCELLED");
        assert_eq!(
            fixture.admission.available_permits(),
            6,
            "IPC cancellation cannot release the still queued blocking operation"
        );
        assert_eq!(fixture.observed.private_file().unwrap(), None);
        drop(held);
        let (id, result) = peer.response().await;
        assert_eq!(id, "first");
        assert_eq!(result.unwrap(), serde_json::json!({"bytesWritten":5}));
        peer.close().await;
        assert_eq!(fault.publications.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.admission.available_permits(), 8);
        assert_eq!(
            fixture.observed.private_file().unwrap(),
            Some(b"first".to_vec())
        );
        assert_eq!(fixture.observed.pending_file_replacements().unwrap(), 0);
    });
}

// Exercise the App's actual cleanup against a real prepared broker replacement.
mod export_cleanup {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/apps/excalidraw/backend/src/export_cleanup.rs"
    ));
}

#[test]
fn cancelled_export_removes_staging_while_prepared_replacement_drains() {
    let fixture = Fixture::new(Mode::Ready);
    fixture.runtime.block_on(async {
        let (started, start) = oneshot::channel();
        let started = Mutex::new(Some(started));
        let checks = AtomicUsize::new(0);
        let mut service = fixture.service();
        service.budget_observer = Some(Arc::new(move || {
            if checks.fetch_add(1, Ordering::SeqCst) == 4 {
                let _ = started.lock().unwrap().take().unwrap().send(());
            }
        }));
        let (queued, queue) = oneshot::channel();
        let queued = Mutex::new(Some(queued));
        let enqueues = AtomicUsize::new(0);
        let fault = Arc::new(TestPublicationFault {
            queued: Some(Arc::new(move || {
                if enqueues.fetch_add(1, Ordering::SeqCst) == 1 {
                    let _ = queued.lock().unwrap().take().unwrap().send(());
                }
            })),
            ..Default::default()
        });
        service.publication_fault = Some(fault.clone());
        let mut peer = TestPeer::start(service);
        let directory = fixture.observed.export_staging_directory();
        std::fs::create_dir(&directory).unwrap();
        let destination = directory.join("copy.excalidraw");
        let mut blocker = rusqlite::Connection::open(fixture.store.path()).unwrap();
        let held = blocker.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        peer.replace("first", b"first").await;
        timeout(WAIT, start).await.unwrap().unwrap();
        peer.send("export", "files.atomic_replace", serde_json::json!({
            "path":".export-cancelled/copy.excalidraw", "contentsBase64":STANDARD.encode(b"cancelled export")
        })).await;
        timeout(WAIT, queue).await.unwrap().unwrap();
        let entries = std::fs::read_dir(&directory).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].to_string_lossy().starts_with(".chariox-replace-"));
        peer.cancel("export").await;
        let (id, result) = peer.response().await;
        assert_eq!(id, "export");
        assert_eq!(result.unwrap_err().code, "CANCELLED");
        // The component's Drop runs before the outstanding mutation releases its temp file.
        export_cleanup::remove(&destination).unwrap();
        assert!(!directory.exists());
        assert_eq!(fixture.admission.available_permits(), 6);
        drop(held);
        assert_eq!(peer.response().await.0, "first");
        peer.close().await;
        assert!(!directory.exists(), "the drained mutation must not recreate staging");
        assert_eq!(fault.publications.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.admission.available_permits(), 8);
        assert_eq!(fixture.observed.pending_file_replacements().unwrap(), 0);
        assert_eq!(fixture.observed.private_file().unwrap(), Some(b"first".to_vec()));
    });
}
