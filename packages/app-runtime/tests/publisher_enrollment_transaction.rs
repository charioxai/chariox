use chariox_app_package::TrustedPublisher;
use chariox_app_runtime::publisher_trust::{enroll_in, PublisherTrustRegistry, TrustDecision};
use rusqlite::Connection;

fn publisher() -> TrustedPublisher {
    TrustedPublisher {
        publisher_id: "local.developer".into(),
        key_id: "development".into(),
        public_key: ed25519_dalek::SigningKey::from_bytes(&[19; 32]).verifying_key(),
    }
}
fn decision() -> TrustDecision {
    TrustDecision {
        decision_id: "human-enrollment".into(),
        authority_ref: "kernel-operation".into(),
    }
}

#[test]
fn enrollment_composes_with_operation_commit_and_rolls_back_with_it() {
    let mut connection = Connection::open_in_memory().unwrap();
    PublisherTrustRegistry::new(&mut connection)
        .initialize()
        .unwrap();
    connection
        .execute_batch("CREATE TABLE operation_receipts(id TEXT PRIMARY KEY)")
        .unwrap();
    {
        let transaction = connection.transaction().unwrap();
        enroll_in(&transaction, "owner", &publisher(), 0, &decision(), 1).unwrap();
        transaction
            .execute("INSERT INTO operation_receipts VALUES('operation')", [])
            .unwrap();
        // Simulate outer operation failure: neither enrollment nor receipt commits.
    }
    assert!(PublisherTrustRegistry::new(&mut connection)
        .list("owner")
        .unwrap()
        .is_empty());
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM operation_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let transaction = connection.transaction().unwrap();
    let receipt = enroll_in(&transaction, "owner", &publisher(), 0, &decision(), 2).unwrap();
    transaction
        .execute("INSERT INTO operation_receipts VALUES('operation')", [])
        .unwrap();
    transaction.commit().unwrap();
    assert_eq!(receipt.revision, 1);
    assert_eq!(
        PublisherTrustRegistry::new(&mut connection)
            .get("owner", "local.developer", "development")
            .unwrap()
            .revision,
        1
    );
}

#[test]
fn handled_inner_failure_cannot_commit_an_orphan_publisher_decision() {
    let mut connection = Connection::open_in_memory().unwrap();
    PublisherTrustRegistry::new(&mut connection)
        .initialize()
        .unwrap();
    connection.execute_batch("CREATE TRIGGER reject_publisher BEFORE INSERT ON app_publisher_keys BEGIN SELECT RAISE(ABORT,'fixture'); END;
        CREATE TABLE adjacent(value INTEGER)").unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(enroll_in(&transaction, "owner", &publisher(), 0, &decision(), 1).is_err());
    transaction
        .execute("INSERT INTO adjacent VALUES(1)", [])
        .unwrap();
    transaction.commit().unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM app_publisher_decisions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM adjacent", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
