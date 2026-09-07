use chariox_app_package::TrustedPublisher;
use chariox_app_runtime::publisher_trust::{
    PublisherTrustError as Error, PublisherTrustRegistry as Registry, TrustDecision,
    TrustedPublisherSnapshot, MAX_DECISIONS_PER_OWNER, MAX_KEYS_PER_OWNER,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rusqlite::{params, Connection, TransactionBehavior};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-publisher-trust-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> Connection {
        let mut connection = Connection::open(self.0.join("kernel.sqlite")).unwrap();
        Registry::new(&mut connection).initialize().unwrap();
        connection
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn publisher(seed: u8) -> TrustedPublisher {
    TrustedPublisher {
        publisher_id: "local.developer".into(),
        key_id: "dev-key-1".into(),
        public_key: SigningKey::from_bytes(&[seed; 32]).verifying_key(),
    }
}
fn decision(id: &str) -> TrustDecision {
    TrustDecision {
        decision_id: id.into(),
        authority_ref: "kernel-interaction:confirmed".into(),
    }
}
fn memory() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    Registry::new(&mut connection).initialize().unwrap();
    connection
}

fn check_snapshot(
    snapshot: &TrustedPublisherSnapshot,
    connection: &mut Connection,
    owner: &str,
) -> Result<(), Error> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    snapshot.require_current(&transaction, owner)
}

#[test]
fn enrolled_key_reopens_with_exact_owner_and_manifest_identity() {
    let root = Scratch::new();
    let mut connection = root.open();
    let public = publisher(1);
    let receipt = Registry::new(&mut connection)
        .enroll("alice", &public, 0, &decision("accept"), 10)
        .unwrap();
    assert_eq!(receipt.revision, 1);
    assert!(receipt.enrolled);
    drop(connection);
    let mut connection = root.open();
    let registry = Registry::new(&mut connection);
    let snapshot = registry
        .trusted_publisher("alice", &public.publisher_id, &public.key_id)
        .unwrap();
    assert_eq!(snapshot.publisher().public_key, public.public_key);
    assert_eq!(snapshot.publisher().publisher_id, public.publisher_id);
    assert_eq!(snapshot.revision(), 1);
    assert!(matches!(
        registry.get("bob", &public.publisher_id, &public.key_id),
        Err(Error::NotFound)
    ));
    assert!(registry.list("bob").unwrap().is_empty());
    assert!(matches!(
        registry.get("alice", "local.other", &public.key_id),
        Err(Error::NotFound)
    ));
    drop(registry);
    assert!(matches!(
        check_snapshot(&snapshot, &mut connection, "bob"),
        Err(Error::NotFound)
    ));
    check_snapshot(&snapshot, &mut connection, "alice").unwrap();
    // Identical decision IDs and publisher names in another owner are separate.
    Registry::new(&mut connection)
        .enroll("bob", &publisher(2), 0, &decision("accept"), 20)
        .unwrap();
    check_snapshot(&snapshot, &mut connection, "alice").unwrap();
}

#[test]
fn old_decision_replay_never_regrants_and_reenrollment_fences_old_snapshots() {
    let root = Scratch::new();
    let mut first = root.open();
    let mut second = root.open();
    let public = publisher(1);
    let accept = decision("accept");
    let first_receipt = Registry::new(&mut first)
        .enroll("alice", &public, 0, &accept, 10)
        .unwrap();
    let snapshot = Registry::new(&mut first)
        .trusted_publisher("alice", &public.publisher_id, &public.key_id)
        .unwrap();
    let revocation = Registry::new(&mut second)
        .revoke(
            "alice",
            &public.publisher_id,
            &public.key_id,
            1,
            &decision("revoke"),
            20,
        )
        .unwrap();
    assert_eq!(revocation.revision, 2);
    assert!(!revocation.enrolled);
    let retry = Registry::new(&mut first)
        .enroll("alice", &public, 0, &accept, 99)
        .unwrap();
    assert_eq!(
        retry, first_receipt,
        "retry returns the historical decision without mutation"
    );
    assert!(matches!(
        check_snapshot(&snapshot, &mut first, "alice"),
        Err(Error::Revoked)
    ));
    assert!(
        !Registry::new(&mut first)
            .get("alice", &public.publisher_id, &public.key_id)
            .unwrap()
            .enrolled
    );
    assert!(matches!(
        Registry::new(&mut first).enroll("alice", &public, 1, &decision("stale"), 30),
        Err(Error::Conflict)
    ));
    let enrolled = Registry::new(&mut first)
        .enroll("alice", &public, 2, &decision("reenroll"), 30)
        .unwrap();
    assert_eq!(enrolled.revision, 3);
    assert!(matches!(
        check_snapshot(&snapshot, &mut first, "alice"),
        Err(Error::Conflict)
    ));
    Registry::new(&mut first)
        .revoke(
            "alice",
            &public.publisher_id,
            &public.key_id,
            1,
            &decision("revoke"),
            100,
        )
        .unwrap();
    assert!(
        Registry::new(&mut first)
            .get("alice", &public.publisher_id, &public.key_id)
            .unwrap()
            .enrolled,
        "an old revocation retry also cannot mutate a later explicit decision"
    );
}

#[test]
fn immutable_binding_and_conflicting_decision_reuse_are_rejected() {
    let mut connection = memory();
    let public = publisher(1);
    let accept = decision("accept");
    Registry::new(&mut connection)
        .enroll("alice", &public, 0, &accept, 10)
        .unwrap();
    assert!(matches!(
        Registry::new(&mut connection).enroll("alice", &publisher(2), 1, &decision("replace"), 20),
        Err(Error::Conflict)
    ));
    let mut renamed = public.clone();
    renamed.key_id = "dev-key-2".into();
    assert!(matches!(
        Registry::new(&mut connection).enroll("alice", &renamed, 0, &accept, 20),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        Registry::new(&mut connection).enroll("alice", &public, 1, &accept, 20),
        Err(Error::Conflict)
    ));
    let mut changed = accept.clone();
    changed.authority_ref = "another-interaction".into();
    assert!(matches!(
        Registry::new(&mut connection).enroll("alice", &public, 0, &changed, 20),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        Registry::new(&mut connection).revoke(
            "alice",
            &public.publisher_id,
            &public.key_id,
            1,
            &accept,
            20
        ),
        Err(Error::Conflict)
    ));
    Registry::new(&mut connection)
        .revoke(
            "alice",
            &public.publisher_id,
            &public.key_id,
            1,
            &decision("revoke"),
            20,
        )
        .unwrap();
    assert!(matches!(
        Registry::new(&mut connection).enroll(
            "alice",
            &publisher(2),
            2,
            &decision("replace-after-revoke"),
            30
        ),
        Err(Error::Conflict)
    ));
    let mut rotated = publisher(2);
    rotated.key_id = "dev-key-2".into();
    Registry::new(&mut connection)
        .enroll("alice", &rotated, 0, &decision("new-key"), 30)
        .unwrap();
    assert_eq!(
        Registry::new(&mut connection).list("alice").unwrap().len(),
        2
    );
}

#[test]
fn key_and_receipt_mutation_roll_back_together_on_actual_sql_failure() {
    let mut connection = memory();
    let public = publisher(1);
    Registry::new(&mut connection)
        .enroll("alice", &public, 0, &decision("accept"), 10)
        .unwrap();
    connection
        .execute_batch(
            "CREATE TEMP TRIGGER deny_trust_update BEFORE UPDATE ON app_publisher_keys
        BEGIN SELECT RAISE(ABORT, 'injected persistence failure'); END;",
        )
        .unwrap();
    assert!(matches!(
        Registry::new(&mut connection).revoke(
            "alice",
            &public.publisher_id,
            &public.key_id,
            1,
            &decision("revoke"),
            20
        ),
        Err(Error::Database(_))
    ));
    let entry = Registry::new(&mut connection)
        .get("alice", &public.publisher_id, &public.key_id)
        .unwrap();
    assert_eq!(entry.revision, 1);
    assert!(entry.enrolled);
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM app_publisher_decisions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    connection
        .execute_batch("DROP TRIGGER deny_trust_update;")
        .unwrap();
    Registry::new(&mut connection)
        .revoke(
            "alice",
            &public.publisher_id,
            &public.key_id,
            1,
            &decision("revoke"),
            20,
        )
        .unwrap();
    connection
        .execute_batch(
            "CREATE TEMP TRIGGER deny_trust_insert BEFORE INSERT ON app_publisher_keys
        BEGIN SELECT RAISE(ABORT, 'injected persistence failure'); END;",
        )
        .unwrap();
    assert!(matches!(
        Registry::new(&mut connection).enroll("bob", &public, 0, &decision("accept"), 30),
        Err(Error::Database(_))
    ));
    assert!(Registry::new(&mut connection)
        .list("bob")
        .unwrap()
        .is_empty());
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM app_publisher_decisions WHERE owner_id='bob'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn snapshot_recheck_runs_inside_the_installation_writers_transaction() {
    let root = Scratch::new();
    let mut first = root.open();
    let mut second = root.open();
    second.busy_timeout(std::time::Duration::ZERO).unwrap();
    let public = publisher(1);
    Registry::new(&mut first)
        .enroll("alice", &public, 0, &decision("accept"), 10)
        .unwrap();
    let snapshot = Registry::new(&mut first)
        .trusted_publisher("alice", &public.publisher_id, &public.key_id)
        .unwrap();
    first
        .execute_batch("CREATE TABLE installation_commit_fixture (revision INTEGER NOT NULL);")
        .unwrap();
    let transaction = first
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    snapshot.require_current(&transaction, "alice").unwrap();
    assert!(matches!(
        Registry::new(&mut second).revoke(
            "alice",
            &public.publisher_id,
            &public.key_id,
            1,
            &decision("revoke"),
            20
        ),
        Err(Error::Database(_))
    ));
    transaction
        .execute(
            "INSERT INTO installation_commit_fixture(revision) VALUES(?1)",
            params![snapshot.revision() as i64],
        )
        .unwrap();
    transaction.commit().unwrap();
    Registry::new(&mut second)
        .revoke(
            "alice",
            &public.publisher_id,
            &public.key_id,
            1,
            &decision("revoke"),
            20,
        )
        .unwrap();
    let transaction = first
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(matches!(
        snapshot.require_current(&transaction, "alice"),
        Err(Error::Revoked)
    ));
    transaction.rollback().unwrap();
    assert_eq!(
        first
            .query_row(
                "SELECT count(*) FROM installation_commit_fixture",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn decision_bounds_preserve_capacity_to_revoke_every_active_key() {
    let mut connection = memory();
    let public = publisher(1);
    let mut revision = 0;
    for index in 0..MAX_DECISIONS_PER_OWNER - 1 {
        revision = Registry::new(&mut connection)
            .enroll(
                "alice",
                &public,
                revision,
                &decision(&format!("accept-{index}")),
                index as u64,
            )
            .unwrap()
            .revision;
    }
    assert!(matches!(
        Registry::new(&mut connection).enroll(
            "alice",
            &public,
            revision,
            &decision("too-many"),
            1000
        ),
        Err(Error::Limit)
    ));
    let revoked = Registry::new(&mut connection)
        .revoke(
            "alice",
            &public.publisher_id,
            &public.key_id,
            revision,
            &decision("reserved-revoke"),
            1000,
        )
        .unwrap();
    assert!(!revoked.enrolled);
    assert!(matches!(
        Registry::new(&mut connection).enroll(
            "alice",
            &public,
            revoked.revision,
            &decision("after-limit"),
            1001
        ),
        Err(Error::Limit)
    ));
    Registry::new(&mut connection)
        .enroll("alice", &public, 0, &decision("accept-0"), 2000)
        .unwrap();
    assert!(
        !Registry::new(&mut connection)
            .get("alice", &public.publisher_id, &public.key_id)
            .unwrap()
            .enrolled
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM app_publisher_decisions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        MAX_DECISIONS_PER_OWNER as i64
    );
}

#[test]
fn bounded_key_tombstones_never_release_an_immutable_binding() {
    let mut connection = memory();
    for index in 0..MAX_KEYS_PER_OWNER {
        let mut public = publisher(1);
        public.key_id = format!("key-{index}");
        Registry::new(&mut connection)
            .enroll(
                "alice",
                &public,
                0,
                &decision(&format!("accept-{index}")),
                10,
            )
            .unwrap();
        Registry::new(&mut connection)
            .revoke(
                "alice",
                &public.publisher_id,
                &public.key_id,
                1,
                &decision(&format!("revoke-{index}")),
                20,
            )
            .unwrap();
    }
    assert_eq!(
        Registry::new(&mut connection).list("alice").unwrap().len(),
        MAX_KEYS_PER_OWNER
    );
    assert!(matches!(
        Registry::new(&mut connection).enroll("alice", &publisher(2), 0, &decision("extra"), 30),
        Err(Error::Limit)
    ));
    Registry::new(&mut connection)
        .enroll("bob", &publisher(2), 0, &decision("extra"), 30)
        .unwrap();
}

#[test]
fn invalid_keys_identifiers_and_revision_exhaustion_never_become_trust() {
    let mut connection = memory();
    let public = publisher(1);
    let mut weak = public.clone();
    weak.public_key = VerifyingKey::from_bytes(&[0; 32]).unwrap();
    assert!(weak.public_key.is_weak());
    assert!(matches!(
        Registry::new(&mut connection).enroll("alice", &weak, 0, &decision("weak"), 1),
        Err(Error::Invalid)
    ));
    let mut bad_name = public.clone();
    bad_name.publisher_id = "_invalid".into();
    assert!(matches!(
        Registry::new(&mut connection).enroll("alice", &bad_name, 0, &decision("bad"), 1),
        Err(Error::Invalid)
    ));
    assert!(matches!(
        Registry::new(&mut connection).enroll("alice", &public, u64::MAX, &decision("overflow"), 1),
        Err(Error::Invalid)
    ));
    assert!(Registry::new(&mut connection)
        .list("alice")
        .unwrap()
        .is_empty());
    Registry::new(&mut connection)
        .enroll("alice", &public, 0, &decision("accept"), 1)
        .unwrap();
    connection
        .execute("UPDATE app_publisher_keys SET revision=?1", [i64::MAX])
        .unwrap();
    assert!(matches!(
        Registry::new(&mut connection).revoke(
            "alice",
            &public.publisher_id,
            &public.key_id,
            i64::MAX as u64,
            &decision("exhausted"),
            2
        ),
        Err(Error::Limit)
    ));
    assert_eq!(
        Registry::new(&mut connection)
            .get("alice", &public.publisher_id, &public.key_id)
            .unwrap()
            .revision,
        i64::MAX as u64
    );
    connection
        .execute("UPDATE app_publisher_keys SET public_key=zeroblob(32)", [])
        .unwrap();
    assert!(matches!(
        Registry::new(&mut connection).trusted_publisher(
            "alice",
            &public.publisher_id,
            &public.key_id
        ),
        Err(Error::Corrupt)
    ));
}
