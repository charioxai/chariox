use super::*;
use crate::durable_state::{
    app_installation_staging::{AppVerifiedInstallationError, AppVerifiedInstallationMutation},
    app_publishers::AppPublisherMutation,
};
use crate::runtime::app_package_upload_control::UploadCommand;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chariox_app_package::{pack, Manifest, TrustedPublisher};
use chariox_app_runtime::publisher_trust::TrustDecision;
use ed25519_dalek::SigningKey;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "chariox-app-preparation-{:016x}",
                rand::random::<u64>()
            ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }
    fn store(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.0.join("kernel.db")).unwrap()
    }
    fn releases(&self) -> Vec<PathBuf> {
        fs::read_dir(&self.0)
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.unwrap();
                entry
                    .file_name()
                    .to_str()
                    .unwrap()
                    .starts_with("app-releases-")
                    .then(|| entry.path())
            })
            .collect()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fn writable(path: &Path) {
            if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
                for entry in fs::read_dir(path).unwrap() {
                    writable(&entry.unwrap().path());
                }
            }
        }
        writable(&self.0);
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn decision(id: &str) -> TrustDecision {
    TrustDecision {
        decision_id: id.into(),
        authority_ref: "kernel-test-decision".into(),
    }
}
fn publisher() -> TrustedPublisher {
    TrustedPublisher {
        publisher_id: "com.example".into(),
        key_id: "developer-1".into(),
        public_key: SigningKey::from_bytes(&[27; 32]).verifying_key(),
    }
}
fn enroll(store: &DurableKernelStateStore, owner: &str) {
    store
        .mutate_app_publisher(
            owner,
            AppPublisherMutation::Enroll {
                publisher: publisher(),
                expected_revision: 0,
                decision: decision("enroll"),
                now_ms: 1,
            },
        )
        .unwrap();
}
fn archive(signing_byte: u8) -> Vec<u8> {
    let manifest: Manifest = serde_json::from_value(json!({
        "schema":"chariox.app.v1", "appId":"com.example.prepared", "version":"1.0.0",
        "publisher":{"id":"com.example","keyId":"developer-1","name":"Developer"},
        "sdkVersion":chariox_app_package::SUPPORTED_SDK_VERSION, "appContractVersion":1,
        "minKernelProtocol":crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
        "resourcePolicy":"chariox.app.resources.v1",
        "runtime":{"engine":"node","entry":"runtime/main.js"},
        "ui":{"entry":"ui/index.html"}, "capabilities":{}
    }))
    .unwrap();
    let files = BTreeMap::from([
        (
            "runtime/main.js".into(),
            b"export default function register() {}".to_vec(),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><title>prepared</title>".to_vec(),
        ),
    ]);
    pack(
        &manifest,
        &files,
        &SigningKey::from_bytes(&[signing_byte; 32]),
        &Limits::default(),
    )
    .unwrap()
}
fn upload(service: &AppPackageUploadControl, owner: &str, bytes: &[u8], complete: bool) -> String {
    let admission = Arc::new(Semaphore::new(8));
    runtime().block_on(async {
        let begun = service
            .execute(
                owner.into(),
                UploadCommand::Begin {
                    request_id: digest(bytes),
                    expected_size: bytes.len() as u64,
                    sha256: digest(bytes),
                },
                admission.clone().try_acquire_owned().unwrap(),
            )
            .await
            .unwrap();
        if complete {
            service
                .execute(
                    owner.into(),
                    UploadCommand::Chunk {
                        handle: begun.handle.clone(),
                        offset: 0,
                        data_base64: STANDARD.encode(bytes),
                        chunk_sha256: digest(bytes),
                    },
                    admission.try_acquire_owned().unwrap(),
                )
                .await
                .unwrap();
        }
        begun.handle
    })
}
fn service(store: &DurableKernelStateStore) -> (AppPackageUploadControl, AppPackagePreparation) {
    let uploads = AppPackageUploadControl::new(store.path().to_owned());
    (
        uploads.clone(),
        AppPackagePreparation::new(store.clone(), uploads),
    )
}
fn prepare(
    service: &AppPackagePreparation,
    owner: &str,
    handle: &str,
) -> Result<PreparedAppPackage> {
    runtime().block_on(service.prepare(
        owner.into(),
        handle.into(),
        Arc::new(Semaphore::new(8)).try_acquire_owned().unwrap(),
    ))
}

#[test]
fn preparation_publishes_exact_verified_bytes_without_installing_and_retries_reuse() {
    let root = Scratch::new();
    let store = root.store();
    enroll(&store, "alice");
    let (uploads, service) = service(&store);
    let bytes = archive(27);
    let handle = upload(&uploads, "alice", &bytes, true);
    let first = prepare(&service, "alice", &handle).unwrap();
    assert!(!first.reused());
    assert_eq!(
        first
            .candidate("alice")
            .unwrap()
            .release_metadata()
            .package_digest,
        digest(&bytes)
    );
    assert_eq!(
        first.directory("alice").unwrap().metadata().unwrap().mode() & 0o777,
        0o500
    );
    assert_eq!(
        first.candidate("bob").err(),
        Some(PreparationError::UploadNotFound)
    );
    assert_eq!(
        first.directory("bob").err(),
        Some(PreparationError::UploadNotFound)
    );
    assert!(store
        .list_app_installations("alice", None, 10)
        .unwrap()
        .installations
        .is_empty());
    let second = prepare(&service, "alice", &handle).unwrap();
    assert!(second.reused());
    let a = first.directory("alice").unwrap().metadata().unwrap();
    let b = second.directory("alice").unwrap().metadata().unwrap();
    assert_eq!((a.dev(), a.ino()), (b.dev(), b.ino()));
    let release_path = root
        .releases()
        .pop()
        .unwrap()
        .join(digest(&bytes).strip_prefix("sha256:").unwrap());
    assert_eq!(
        fs::read(release_path.join("envelope.cxapp")).unwrap(),
        bytes
    );
    drop(first);
    drop(second);
    drop(service);
    drop(uploads);
    let (_, reopened) = self::service(&store);
    assert!(prepare(&reopened, "alice", &handle).unwrap().reused());
}

#[test]
fn wrong_owner_partial_unenrolled_and_bad_signature_publish_no_release() {
    let root = Scratch::new();
    let store = root.store();
    let (uploads, service) = service(&store);
    let valid = archive(27);
    let handle = upload(&uploads, "alice", &valid, false);
    assert_eq!(
        prepare(&service, "bob", &handle).err(),
        Some(PreparationError::UploadNotFound)
    );
    assert_eq!(
        prepare(&service, "alice", &handle).err(),
        Some(PreparationError::UploadIncomplete)
    );
    upload(&uploads, "alice", &valid, true);
    assert_eq!(
        prepare(&service, "alice", &handle).err(),
        Some(PreparationError::PublisherNotEnrolled)
    );
    enroll(&store, "alice");
    let bad = upload(&uploads, "alice", &archive(29), true);
    assert_eq!(
        prepare(&service, "alice", &bad).err(),
        Some(PreparationError::PackageRejected(
            ErrorCode::InvalidSignature
        ))
    );
    assert!(root.releases().is_empty());
    assert!(store
        .list_app_installations("alice", None, 10)
        .unwrap()
        .installations
        .is_empty());
}

#[test]
fn interrupted_publication_retry_and_revocation_before_writer_stage_are_safe() {
    let root = Scratch::new();
    let store = root.store();
    enroll(&store, "alice");
    let (uploads, service) = service(&store);
    let handle = upload(&uploads, "alice", &archive(27), true);
    assert_eq!(
        service
            .prepare_at("alice", &handle, crate::session::unix_epoch_ms(), |point| {
                if point == PreparationCheckpoint::Published {
                    Err(PreparationError::PublicationInterrupted)
                } else {
                    Ok(())
                }
            })
            .err(),
        Some(PreparationError::PublicationInterrupted)
    );
    let recovered = prepare(&service, "alice", &handle).unwrap();
    assert!(recovered.reused());
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "developer-1".into(),
                expected_revision: 1,
                decision: decision("revoke"),
                now_ms: 3,
            },
        )
        .unwrap();
    assert!(matches!(
        store.mutate_verified_app_installation(
            "alice",
            AppVerifiedInstallationMutation::CreateAndStage {
                installation_id: "installation".into(),
                candidate: recovered.candidate("alice").unwrap().clone(),
                now_ms: 4,
            }
        ),
        Err(AppVerifiedInstallationError::Verified(
            VerifiedStageError::Trust(PublisherTrustError::Revoked)
        ))
    ));
    assert_eq!(
        prepare(&service, "alice", &handle).err(),
        Some(PreparationError::PublisherRevoked)
    );
    assert!(store
        .list_app_installations("alice", None, 10)
        .unwrap()
        .installations
        .is_empty());
}

#[test]
fn cancellation_retains_both_admission_permits_until_blocking_work_finishes() {
    let runtime = runtime();
    let preparation = Arc::new(Semaphore::new(1));
    let admission = Arc::new(Semaphore::new(1));
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let task = runtime.spawn(admitted_blocking(
        preparation.clone(),
        admission.clone().try_acquire_owned().unwrap(),
        move || {
            started_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            done_tx.send(()).unwrap();
            Ok(())
        },
    ));
    runtime.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while started_rx.try_recv().is_err() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    });
    task.abort();
    runtime.block_on(async {
        assert!(task.await.unwrap_err().is_cancelled());
    });
    assert_eq!(admission.available_permits(), 0);
    assert_eq!(preparation.available_permits(), 0);
    let replacement = Arc::new(Semaphore::new(1));
    assert_eq!(
        runtime.block_on(admitted_blocking(
            preparation.clone(),
            replacement.clone().try_acquire_owned().unwrap(),
            || Ok(())
        )),
        Err(PreparationError::Busy)
    );
    assert_eq!(replacement.available_permits(), 1);
    release_tx.send(()).unwrap();
    done_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    // Runtime shutdown joins blocking tasks, including their retained permits.
    drop(runtime);
    assert_eq!(admission.available_permits(), 1);
    assert_eq!(preparation.available_permits(), 1);
}
