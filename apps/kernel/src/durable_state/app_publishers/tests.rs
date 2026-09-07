use super::*;

struct Database(std::path::PathBuf);
impl Database {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-app-publishers-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.0.join("kernel.db")).unwrap()
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn enroll(decision_id: &str, revision: u64) -> AppPublisherMutation {
    AppPublisherMutation::Enroll {
        publisher: TrustedPublisher {
            publisher_id: "local.developer".into(),
            key_id: "development".into(),
            public_key: ed25519_dalek::SigningKey::from_bytes(&[17; 32]).verifying_key(),
        },
        expected_revision: revision,
        decision: TrustDecision {
            decision_id: decision_id.into(),
            authority_ref: "kernel-decision-fixture".into(),
        },
        now_ms: 10,
    }
}

fn revoke(decision_id: &str, revision: u64) -> AppPublisherMutation {
    AppPublisherMutation::Revoke {
        publisher_id: "local.developer".into(),
        key_id: "development".into(),
        expected_revision: revision,
        decision: TrustDecision {
            decision_id: decision_id.into(),
            authority_ref: "kernel-decision-fixture".into(),
        },
        now_ms: 20,
    }
}

fn check_snapshot(
    store: &DurableKernelStateStore,
    snapshot: &TrustedPublisherSnapshot,
) -> Result<(), PublisherTrustError> {
    let mut reader = store.connection.lock().unwrap();
    let transaction = reader.transaction().unwrap();
    snapshot.require_current(&transaction, "alice")
}

#[test]
fn publisher_trust_uses_writer_and_recovers_revocation_without_regranting_old_decisions() {
    let database = Database::new();
    let store = database.open();
    let reader = store.connection.lock().unwrap();
    // Holding the separate read-only connection cannot prevent a writer commit.
    let enrolled = store
        .mutate_app_publisher("alice", enroll("enroll", 0))
        .unwrap();
    assert_eq!(enrolled.revision, 1);
    drop(reader);
    let before = store
        .trusted_app_publisher("alice", "local.developer", "development")
        .unwrap();
    assert!(store.list_app_publishers("bob").unwrap().is_empty());
    assert!(matches!(
        store.trusted_app_publisher("bob", "local.developer", "development"),
        Err(AppPublisherError::Trust(PublisherTrustError::NotFound))
    ));
    assert!(matches!(
        store.mutate_app_publisher("bob", revoke("revoke", 1)),
        Err(AppPublisherError::Trust(PublisherTrustError::NotFound))
    ));
    let revoked = store
        .mutate_app_publisher("alice", revoke("revoke", 1))
        .unwrap();
    assert_eq!(revoked.revision, 2);
    assert!(!revoked.enrolled);
    drop(store);

    let store = database.open();
    assert_eq!(
        store
            .mutate_app_publisher("alice", enroll("enroll", 0))
            .unwrap(),
        enrolled
    );
    assert!(matches!(
        store.trusted_app_publisher("alice", "local.developer", "development"),
        Err(AppPublisherError::Trust(PublisherTrustError::Revoked))
    ));
    assert!(matches!(
        check_snapshot(&store, &before),
        Err(PublisherTrustError::Revoked)
    ));
    assert!(matches!(
        store.mutate_app_publisher("alice", enroll("revoke", 2)),
        Err(AppPublisherError::Trust(PublisherTrustError::Conflict))
    ));
    let current = store
        .mutate_app_publisher("alice", enroll("new-decision", 2))
        .unwrap();
    assert_eq!(current.revision, 3);
    assert!(matches!(
        check_snapshot(&store, &before),
        Err(PublisherTrustError::Conflict)
    ));
    assert_eq!(store.list_app_publishers("alice").unwrap().len(), 1);
}

#[test]
fn publisher_conflict_preserves_adjacent_ordinary_writer_events() {
    let database = Database::new();
    let store = database.open();
    store
        .mutate_app_publisher("alice", enroll("enroll", 0))
        .unwrap();
    let (before_tx, before_rx) = mpsc::channel();
    let (conflict_tx, conflict_rx) = mpsc::channel();
    let (after_tx, after_rx) = mpsc::channel();
    let event = |id: &str, response| {
        super::super::DurableWriterRequest::Ordinary(super::super::DurableWriteRequest {
            operation: super::super::DurableWriteOperation::Event {
                event_id: id.into(),
                kind: "app.publisher.test".into(),
                subject_id: None,
                timestamp_ms: 1,
                payload_json: "{}".into(),
            },
            response,
        })
    };
    store.writer.enqueue(event("before", before_tx)).unwrap();
    store
        .writer
        .enqueue(DurableWriterRequest::AppPublisher(Box::new(
            AppPublisherRequest {
                owner_id: "alice".into(),
                mutation: revoke("stale", 0),
                response: conflict_tx,
            },
        )))
        .unwrap();
    store.writer.enqueue(event("after", after_tx)).unwrap();
    assert!(before_rx.recv().unwrap().is_ok());
    assert!(matches!(
        conflict_rx.recv().unwrap(),
        Err(PublisherTrustError::Conflict)
    ));
    assert!(after_rx.recv().unwrap().is_ok());
    assert_eq!(
        store
            .load_events_by_kind("app.publisher.test")
            .unwrap()
            .len(),
        2
    );
}
