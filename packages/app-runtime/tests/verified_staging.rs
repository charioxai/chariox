use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    installation::{
        ActiveGeneration, CapabilityApproval, CapabilityDecision, InstallationError,
        InstallationRegistry as Installations, StageToken, UpdatePhase,
        VerifiedInstallCandidate as Candidate, VerifiedStageError as Error,
        RETAINED_UPDATE_RECORDS,
    },
    publisher_trust::{
        PublisherTrustError, PublisherTrustRegistry as Publishers, TrustDecision,
        TrustedPublisherSnapshot,
    },
};
use ed25519_dalek::SigningKey;
use rusqlite::{Connection, TransactionBehavior};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Database(PathBuf);
impl Database {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-verified-stage-{}-{}-{}",
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
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .unwrap();
        Publishers::new(&mut connection).initialize().unwrap();
        Installations::new(&mut connection).initialize().unwrap();
        connection
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[derive(Clone)]
struct Package {
    manifest: Manifest,
    files: BTreeMap<String, Vec<u8>>,
    key: SigningKey,
}
impl Package {
    fn new() -> Self {
        Self { manifest: serde_json::from_value(json!({
            "schema":"chariox.app.v1", "appId":"com.example.installed", "version":"1.0.0",
            "publisher":{"id":"com.example","keyId":"developer-1","name":"Developer"},
            "sdkVersion":"0.3.0", "appContractVersion":1, "minKernelProtocol":500,
            "resourcePolicy":"chariox.app.resources.v1", "runtime":{"engine":"node","entry":"runtime/main.js"},
            "ui":{"entry":"ui/index.html"}, "tools":"schemas/tools.json", "capabilities":{}
        })).unwrap(), key: SigningKey::from_bytes(&[27;32]), files: BTreeMap::from([
            ("runtime/main.js".into(), b"export default function register() {}".to_vec()),
            ("ui/index.html".into(), b"<!doctype html><title>Installed fixture</title>".to_vec()),
            ("assets/logo.svg".into(), b"<svg></svg>".to_vec()),
            ("schemas/tools.json".into(), br#"{"tools":[{"name":"echo","description":"Echo input","inputSchema":{"type":"object","additionalProperties":false,"properties":{"text":{"type":"string"}}}}]}"#.to_vec()),
        ]) }
    }
    fn publisher(&self) -> TrustedPublisher {
        TrustedPublisher {
            publisher_id: self.manifest.publisher.id.clone(),
            key_id: self.manifest.publisher.key_id.clone(),
            public_key: self.key.verifying_key(),
        }
    }
    fn bytes(&self) -> Vec<u8> {
        pack(&self.manifest, &self.files, &self.key, &Limits::default()).unwrap()
    }
    fn candidate(&self, trust: &TrustedPublisherSnapshot) -> Candidate {
        let bytes = self.bytes();
        let policy = VerificationPolicy::new(500, vec![self.publisher()]);
        Candidate::from_verified(&verify(&bytes, &policy).unwrap(), trust).unwrap()
    }
}
fn decision(id: &str) -> TrustDecision {
    TrustDecision {
        decision_id: id.into(),
        authority_ref: "kernel-confirmation".into(),
    }
}
fn enroll(
    connection: &mut Connection,
    owner: &str,
    package: &Package,
    revision: u64,
    id: &str,
) -> TrustedPublisherSnapshot {
    let public = package.publisher();
    let mut publishers = Publishers::new(connection);
    publishers
        .enroll(owner, &public, revision, &decision(id), 1)
        .unwrap();
    publishers
        .trusted_publisher(owner, &public.publisher_id, &public.key_id)
        .unwrap()
}
fn prepare(connection: &mut Connection, token: &StageToken) {
    let mut registry = Installations::new(connection);
    registry
        .decide(
            token,
            CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: "capability-decision".into(),
                    authority_ref: "kernel-confirmation".into(),
                },
            },
            2,
        )
        .unwrap();
    registry.quiesce(token, 3).unwrap();
    registry.mark_prepared(token, 4).unwrap();
}
fn activate(connection: &mut Connection, token: &StageToken) -> ActiveGeneration {
    prepare(connection, token);
    let trust = Publishers::new(connection)
        .trusted_publisher("alice", "com.example", "developer-1")
        .unwrap();
    Installations::new(connection)
        .commit_verified(token, "alice", &trust, 5)
        .unwrap()
}
fn count(connection: &Connection, table: &str) -> i64 {
    assert!([
        "app_installations",
        "app_installation_updates",
        "app_installation_stage_trust"
    ]
    .contains(&table));
    connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn verified_signer_key_cannot_be_substituted_behind_matching_publisher_ids() {
    let database = Database::new();
    let mut connection = database.open();
    let package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    let mut substituted = package.clone();
    substituted.key = SigningKey::from_bytes(&[29; 32]);
    let archive = substituted.bytes();
    let policy = VerificationPolicy::new(500, vec![substituted.publisher()]);
    let verified = verify(&archive, &policy).unwrap();
    assert_eq!(
        verified.signer().public_key,
        substituted.key.verifying_key()
    );
    assert!(matches!(
        Candidate::from_verified(&verified, &trust),
        Err(Error::SignerMismatch)
    ));
    assert_eq!(count(&connection, "app_installations"), 0);
}

#[test]
fn fingerprints_derive_from_verified_semantics_and_all_signed_view_assets() {
    let database = Database::new();
    let mut connection = database.open();
    let package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    let base = package.candidate(&trust).release_metadata().clone();
    assert_eq!(base.schema_version, 0);
    assert_eq!(
        base.package_digest,
        format!("sha256:{:x}", Sha256::digest(package.bytes()))
    );
    let mut grant = package.clone();
    grant
        .manifest
        .capabilities
        .clipboard
        .push(chariox_app_package::ClipboardAccess::Write);
    let changed = grant.candidate(&trust).release_metadata().clone();
    assert_ne!(base.capabilities_digest, changed.capabilities_digest);
    assert_eq!(base.catalog_digest, changed.catalog_digest);
    assert_eq!(base.view_digest, changed.view_digest);
    let mut same_declarations = package.clone();
    same_declarations.files.insert("schemas/tools.json".into(), br#"{"tools":[
      {"inputSchema":{"additionalProperties":false,"properties":{"text":{"type":"string"}},"type":"object"},"description":"Echo input","name":"echo"}
    ]}"#.to_vec());
    let changed = same_declarations
        .candidate(&trust)
        .release_metadata()
        .clone();
    assert_eq!(
        base.catalog_digest, changed.catalog_digest,
        "RFC8785 ignores JSON object order/formatting"
    );
    assert_eq!(base.capabilities_digest, changed.capabilities_digest);
    let mut catalog = package.clone();
    catalog.files.insert("schemas/tools.json".into(), br#"{"tools":[{"name":"echo","description":"New description","inputSchema":{"type":"object","additionalProperties":false,"properties":{"text":{"type":"string"}}}}]}"#.to_vec());
    let changed = catalog.candidate(&trust).release_metadata().clone();
    assert_ne!(base.catalog_digest, changed.catalog_digest);
    assert_eq!(base.capabilities_digest, changed.capabilities_digest);
    let mut assets = package.clone();
    assets
        .files
        .insert("assets/logo.svg".into(), b"<svg><path/></svg>".to_vec());
    let changed = assets.candidate(&trust).release_metadata().clone();
    assert_ne!(
        base.view_digest, changed.view_digest,
        "shared assets outside ui/ are included"
    );
    assert_eq!(base.catalog_digest, changed.catalog_digest);
    let mut migrated = package.clone();
    migrated.manifest.migrations = Some(
        serde_json::from_value(json!({"directory":"migrations", "targetVersion":1,
        "steps":[{"from":0,"to":1,"entry":"migrations/0001.mjs"}]}))
        .unwrap(),
    );
    migrated.files.insert(
        "migrations/0001.mjs".into(),
        b"export default async function migrate() {}".to_vec(),
    );
    assert_eq!(
        migrated.candidate(&trust).release_metadata().schema_version,
        1
    );
}

#[test]
fn stage_and_signer_binding_commit_together_and_reopen_without_approval() {
    let database = Database::new();
    let mut connection = database.open();
    let package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    let candidate = package.candidate(&trust);
    let record = Installations::new(&mut connection)
        .create_and_stage_verified("installation", "alice", &candidate, 10)
        .unwrap();
    assert_eq!(record.phase, UpdatePhase::Staged);
    assert_eq!(record.decision, CapabilityDecision::Pending);
    let binding = Installations::new(&mut connection)
        .staged_trust("alice", &record.token)
        .unwrap();
    assert_eq!(binding.key_id(), "developer-1");
    assert_eq!(binding.trust_revision(), 1);
    assert_eq!(
        binding.package_digest(),
        candidate.release_metadata().package_digest
    );
    assert_eq!(
        binding.public_key_fingerprint(),
        format!(
            "sha256:{:x}",
            Sha256::digest(package.key.verifying_key().as_bytes())
        )
    );
    drop(connection);
    let mut connection = database.open();
    assert_eq!(
        Installations::new(&mut connection)
            .staged_trust("alice", &record.token)
            .unwrap(),
        binding
    );
    assert!(matches!(
        Installations::new(&mut connection).staged_trust("bob", &record.token),
        Err(Error::Installation(InstallationError::NotFound))
    ));
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    binding
        .require_current(&transaction, "alice", &trust)
        .unwrap();
    transaction.rollback().unwrap();
}

#[test]
fn revocation_or_revision_change_before_staging_leaves_no_partial_installation() {
    let database = Database::new();
    let mut first = database.open();
    let mut second = database.open();
    let package = Package::new();
    let trust = enroll(&mut first, "alice", &package, 0, "enroll");
    let candidate = package.candidate(&trust);
    Publishers::new(&mut second)
        .revoke(
            "alice",
            "com.example",
            "developer-1",
            1,
            &decision("revoke"),
            20,
        )
        .unwrap();
    assert!(matches!(
        Installations::new(&mut first).create_and_stage_verified(
            "installation",
            "alice",
            &candidate,
            30
        ),
        Err(Error::Trust(PublisherTrustError::Revoked))
    ));
    for table in [
        "app_installations",
        "app_installation_updates",
        "app_installation_stage_trust",
    ] {
        assert_eq!(count(&first, table), 0);
    }
    let fresh = enroll(&mut second, "alice", &package, 2, "reenroll");
    assert!(matches!(
        Installations::new(&mut first).create_and_stage_verified(
            "installation",
            "alice",
            &candidate,
            40
        ),
        Err(Error::Trust(PublisherTrustError::Conflict))
    ));
    let record = Installations::new(&mut first)
        .create_and_stage_verified("installation", "alice", &package.candidate(&fresh), 50)
        .unwrap();
    assert_eq!(record.token.generation, 1);
}

#[test]
fn existing_installation_owner_and_generation_are_checked_inside_verified_stage() {
    let database = Database::new();
    let mut connection = database.open();
    let package = Package::new();
    let alice = enroll(&mut connection, "alice", &package, 0, "enroll");
    let bob = enroll(&mut connection, "bob", &package, 0, "enroll");
    let record = Installations::new(&mut connection)
        .create_and_stage_verified("installation", "alice", &package.candidate(&alice), 1)
        .unwrap();
    activate(&mut connection, &record.token);
    assert!(matches!(
        Installations::new(&mut connection).stage_verified(
            "installation",
            "bob",
            1,
            &package.candidate(&bob),
            10
        ),
        Err(Error::Installation(InstallationError::NotFound))
    ));
    assert!(matches!(
        Installations::new(&mut connection).stage_verified(
            "installation",
            "alice",
            0,
            &package.candidate(&alice),
            10
        ),
        Err(Error::Installation(InstallationError::Conflict))
    ));
    let updated = Installations::new(&mut connection)
        .stage_verified("installation", "alice", 1, &package.candidate(&alice), 10)
        .unwrap();
    assert_eq!(updated.token.generation, 2);
    assert_eq!(
        Installations::new(&mut connection)
            .get("installation")
            .unwrap()
            .active
            .unwrap()
            .generation,
        1
    );
}

#[test]
fn actual_binding_insert_failure_rolls_back_stage_and_generation_allocation() {
    let database = Database::new();
    let mut connection = database.open();
    let package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    let candidate = package.candidate(&trust);
    let fail = |connection: &Connection| {
        connection.execute_batch("CREATE TEMP TRIGGER deny_stage_binding BEFORE INSERT ON app_installation_stage_trust
        BEGIN SELECT RAISE(ABORT,'injected trust binding failure'); END;").unwrap()
    };
    fail(&connection);
    assert!(matches!(
        Installations::new(&mut connection).create_and_stage_verified(
            "installation",
            "alice",
            &candidate,
            1
        ),
        Err(Error::Installation(InstallationError::Database(_)))
    ));
    for table in [
        "app_installations",
        "app_installation_updates",
        "app_installation_stage_trust",
    ] {
        assert_eq!(count(&connection, table), 0);
    }
    connection
        .execute_batch("DROP TRIGGER deny_stage_binding;")
        .unwrap();
    let first = Installations::new(&mut connection)
        .create_and_stage_verified("installation", "alice", &candidate, 1)
        .unwrap();
    assert_eq!(first.token.generation, 1);
    activate(&mut connection, &first.token);
    fail(&connection);
    assert!(Installations::new(&mut connection)
        .stage_verified("installation", "alice", 1, &candidate, 10)
        .is_err());
    assert!(Installations::new(&mut connection)
        .get("installation")
        .unwrap()
        .pending_generation
        .is_none());
    connection
        .execute_batch("DROP TRIGGER deny_stage_binding;")
        .unwrap();
    let second = Installations::new(&mut connection)
        .stage_verified("installation", "alice", 1, &candidate, 10)
        .unwrap();
    assert_eq!(second.token.generation, 2);
    assert_eq!(count(&connection, "app_installation_stage_trust"), 2);
}

#[test]
fn retained_stage_binding_fences_revocation_reenrollment_and_aborted_candidates() {
    let database = Database::new();
    let mut connection = database.open();
    let package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    let record = Installations::new(&mut connection)
        .create_and_stage_verified("installation", "alice", &package.candidate(&trust), 1)
        .unwrap();
    let binding = Installations::new(&mut connection)
        .staged_trust("alice", &record.token)
        .unwrap();
    Publishers::new(&mut connection)
        .revoke(
            "alice",
            "com.example",
            "developer-1",
            1,
            &decision("revoke"),
            20,
        )
        .unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(matches!(
        binding.require_current(&transaction, "alice", &trust),
        Err(Error::Trust(PublisherTrustError::Revoked))
    ));
    transaction.rollback().unwrap();
    let fresh = enroll(&mut connection, "alice", &package, 2, "reenroll");
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(matches!(
        binding.require_current(&transaction, "alice", &fresh),
        Err(Error::Trust(PublisherTrustError::Conflict))
    ));
    transaction.rollback().unwrap();
    Installations::new(&mut connection)
        .abort(&record.token, "trust changed", 30)
        .unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(matches!(
        binding.require_current(&transaction, "alice", &fresh),
        Err(Error::Installation(InstallationError::InvalidTransition))
    ));
}

#[test]
fn trust_bindings_remain_bounded_without_forgetting_an_old_active_generation() {
    let database = Database::new();
    let mut connection = database.open();
    let package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    let candidate = package.candidate(&trust);
    let first = Installations::new(&mut connection)
        .create_and_stage_verified("installation", "alice", &candidate, 1)
        .unwrap();
    activate(&mut connection, &first.token);
    let binding = Installations::new(&mut connection)
        .staged_trust("alice", &first.token)
        .unwrap();
    for index in 0..RETAINED_UPDATE_RECORDS + 2 {
        let record = Installations::new(&mut connection)
            .stage_verified("installation", "alice", 1, &candidate, index as u64 + 10)
            .unwrap();
        Installations::new(&mut connection)
            .abort(&record.token, "fixture abort", index as u64 + 20)
            .unwrap();
    }
    assert_eq!(
        count(&connection, "app_installation_updates"),
        RETAINED_UPDATE_RECORDS as i64
    );
    assert_eq!(
        count(&connection, "app_installation_stage_trust"),
        (RETAINED_UPDATE_RECORDS + 1) as i64
    );
    assert_eq!(
        Installations::new(&mut connection)
            .staged_trust("alice", &first.token)
            .unwrap(),
        binding
    );
}

#[test]
fn activation_rechecks_revocation_and_reenrollment_without_changing_active_data() {
    let database = Database::new();
    let mut connection = database.open();
    let mut concurrent_writer = database.open();
    let mut package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    let first = Installations::new(&mut connection)
        .create_and_stage_verified("installation", "alice", &package.candidate(&trust), 1)
        .unwrap();
    let active = activate(&mut connection, &first.token);
    package.manifest.version = "2.0.0".into();
    let second = Installations::new(&mut connection)
        .stage_verified("installation", "alice", 1, &package.candidate(&trust), 10)
        .unwrap();
    prepare(&mut connection, &second.token);
    let before = Installations::new(&mut connection)
        .get("installation")
        .unwrap();
    Publishers::new(&mut concurrent_writer)
        .revoke(
            "alice",
            "com.example",
            "developer-1",
            1,
            &decision("revoke"),
            20,
        )
        .unwrap();
    assert!(matches!(
        Installations::new(&mut connection).commit_verified(&second.token, "alice", &trust, 21),
        Err(Error::Trust(PublisherTrustError::Revoked))
    ));
    let fresh = enroll(&mut concurrent_writer, "alice", &package, 2, "reenroll");
    for snapshot in [&trust, &fresh] {
        assert!(matches!(
            Installations::new(&mut connection).commit_verified(
                &second.token,
                "alice",
                snapshot,
                22
            ),
            Err(Error::Trust(PublisherTrustError::Conflict))
        ));
    }
    assert_eq!(
        Installations::new(&mut connection)
            .get("installation")
            .unwrap(),
        before
    );
    assert_eq!(before.active.as_ref(), Some(&active));
    assert_eq!(before.pending_generation, Some(second.token.generation));
    assert!(before.admission_paused);
    assert_eq!(
        Installations::new(&mut connection)
            .journal("installation")
            .unwrap()[0]
            .phase,
        UpdatePhase::Prepared
    );
    Installations::new(&mut connection)
        .abort(&second.token, "trust revision changed", 23)
        .unwrap();
    let third = Installations::new(&mut connection)
        .stage_verified("installation", "alice", 1, &package.candidate(&fresh), 24)
        .unwrap();
    let active = activate(&mut connection, &third.token);
    assert_eq!(active.generation, 3);
    assert_eq!(active.release.version, "2.0.0");
    drop(connection);
    let mut connection = database.open();
    let installed = Installations::new(&mut connection)
        .get("installation")
        .unwrap();
    assert_eq!(installed.active.as_ref(), Some(&active));
    assert_eq!(installed.pending_generation, None);
    assert!(!installed.admission_paused);
}

#[test]
fn activation_requires_verified_binding_owner_approval_and_preparation() {
    let database = Database::new();
    let mut connection = database.open();
    let package = Package::new();
    let alice = enroll(&mut connection, "alice", &package, 0, "enroll");
    let bob = enroll(&mut connection, "bob", &package, 0, "enroll");
    let candidate = package.candidate(&alice);
    let stage = Installations::new(&mut connection)
        .create_and_stage_verified("installation", "alice", &candidate, 1)
        .unwrap();
    assert!(matches!(
        Installations::new(&mut connection).commit_verified(&stage.token, "bob", &bob, 2),
        Err(Error::Installation(InstallationError::NotFound))
    ));
    assert!(matches!(
        Installations::new(&mut connection).commit_verified(&stage.token, "alice", &alice, 2),
        Err(Error::Installation(InstallationError::InvalidTransition))
    ));
    assert!(matches!(
        Installations::new(&mut connection).quiesce(&stage.token, 3),
        Err(InstallationError::ApprovalRequired)
    ));
    assert!(Installations::new(&mut connection)
        .get("installation")
        .unwrap()
        .active
        .is_none());
    prepare(&mut connection, &stage.token);
    let mut invalid_approval = Installations::new(&mut connection)
        .journal("installation")
        .unwrap()
        .remove(0);
    let approved = invalid_approval.clone();
    invalid_approval.decision = CapabilityDecision::Pending;
    // Simulate an inconsistent persisted preparation receipt to directly prove
    // the shared activation mutation still checks approval independently.
    connection.execute("UPDATE app_installation_updates SET record_json = ?1 WHERE installation_id = 'installation'",
        [serde_json::to_string(&invalid_approval).unwrap()]).unwrap();
    assert!(matches!(
        Installations::new(&mut connection).commit_verified(&stage.token, "alice", &alice, 4),
        Err(Error::Installation(InstallationError::ApprovalRequired))
    ));
    connection.execute("UPDATE app_installation_updates SET record_json = ?1 WHERE installation_id = 'installation'",
        [serde_json::to_string(&approved).unwrap()]).unwrap();
    Installations::new(&mut connection)
        .commit_verified(&stage.token, "alice", &alice, 5)
        .unwrap();
    let plain = Installations::new(&mut connection)
        .stage("installation", 1, candidate.release_metadata().clone(), 6)
        .unwrap();
    prepare(&mut connection, &plain.token);
    assert!(matches!(
        Installations::new(&mut connection).commit_verified(&plain.token, "alice", &alice, 7),
        Err(Error::Installation(InstallationError::NotFound))
    ));
}

#[test]
fn actual_activation_journal_failure_rolls_back_active_pointer_and_preserves_pending() {
    let database = Database::new();
    let mut connection = database.open();
    let package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    let candidate = package.candidate(&trust);
    let first = Installations::new(&mut connection)
        .create_and_stage_verified("installation", "alice", &candidate, 1)
        .unwrap();
    activate(&mut connection, &first.token);
    let second = Installations::new(&mut connection)
        .stage_verified("installation", "alice", 1, &candidate, 10)
        .unwrap();
    prepare(&mut connection, &second.token);
    let before = Installations::new(&mut connection)
        .get("installation")
        .unwrap();
    connection.execute_batch("CREATE TEMP TRIGGER deny_activation_journal BEFORE UPDATE ON app_installation_updates
        WHEN NEW.phase = 'committed' BEGIN SELECT RAISE(ABORT,'injected post-pointer journal failure'); END;").unwrap();
    assert!(matches!(
        Installations::new(&mut connection).commit_verified(&second.token, "alice", &trust, 20),
        Err(Error::Installation(InstallationError::Database(_)))
    ));
    assert_eq!(
        Installations::new(&mut connection)
            .get("installation")
            .unwrap(),
        before
    );
    assert_eq!(
        Installations::new(&mut connection)
            .journal("installation")
            .unwrap()[0]
            .phase,
        UpdatePhase::Prepared
    );
    // A successful active UPDATE occurred before the trigger; the failed
    // transaction must undo it on disk, not only in the in-memory return value.
    drop(connection);
    let mut connection = database.open();
    assert_eq!(
        Installations::new(&mut connection)
            .get("installation")
            .unwrap(),
        before
    );
    let active = Installations::new(&mut connection)
        .commit_verified(&second.token, "alice", &trust, 21)
        .unwrap();
    assert_eq!(active.generation, second.token.generation);
}

#[test]
fn plain_commit_cannot_bypass_current_or_revoked_stage_trust() {
    let database = Database::new();
    let mut connection = database.open();
    let package = Package::new();
    let trust = enroll(&mut connection, "alice", &package, 0, "enroll");
    let candidate = package.candidate(&trust);
    let first = Installations::new(&mut connection)
        .create_and_stage_verified("installation", "alice", &candidate, 1)
        .unwrap();
    prepare(&mut connection, &first.token);
    let before = Installations::new(&mut connection)
        .get("installation")
        .unwrap();
    assert!(matches!(
        Installations::new(&mut connection).commit(&first.token, 5),
        Err(InstallationError::Invalid("verified activation required"))
    ));
    assert_eq!(
        Installations::new(&mut connection)
            .get("installation")
            .unwrap(),
        before
    );
    Installations::new(&mut connection)
        .commit_verified(&first.token, "alice", &trust, 6)
        .unwrap();
    let second = Installations::new(&mut connection)
        .stage_verified("installation", "alice", 1, &candidate, 10)
        .unwrap();
    prepare(&mut connection, &second.token);
    Publishers::new(&mut connection)
        .revoke(
            "alice",
            "com.example",
            "developer-1",
            1,
            &decision("revoke"),
            20,
        )
        .unwrap();
    drop(connection);
    let mut connection = database.open();
    let before = Installations::new(&mut connection)
        .get("installation")
        .unwrap();
    assert!(matches!(
        Installations::new(&mut connection).commit(&second.token, 21),
        Err(InstallationError::Invalid("verified activation required"))
    ));
    assert!(matches!(
        Installations::new(&mut connection).commit_verified(&second.token, "alice", &trust, 21),
        Err(Error::Trust(PublisherTrustError::Revoked))
    ));
    assert_eq!(
        Installations::new(&mut connection)
            .get("installation")
            .unwrap(),
        before
    );
    assert_eq!(before.generation, 1);
    assert_eq!(before.pending_generation, Some(second.token.generation));
    assert!(before.admission_paused);
}
