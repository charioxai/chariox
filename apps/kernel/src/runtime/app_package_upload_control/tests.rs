use super::*;
use chariox_app_runtime::package_upload::UploadPhase;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Semaphore;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "chariox-upload-control-{}-{}-{}",
                std::process::id(),
                crate::session::unix_epoch_ms(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        Self(root)
    }

    fn database(&self) -> PathBuf {
        self.0.join("state.db")
    }

    fn service(&self) -> AppPackageUploadControl {
        AppPackageUploadControl::new(self.database())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn begin() -> UploadCommand {
    UploadCommand::Begin {
        request_id: "client-retry-id".into(),
        expected_size: 4,
        sha256: digest(b"test"),
    }
}

#[test]
fn lazy_service_resumes_owner_bound_retries_and_retains_abort_status() {
    let root = Scratch::new();
    let service = root.service();
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    let first = service.execute_at("alice", begin(), 10).unwrap();
    assert_eq!(first.expires_at_ms, 10 + UPLOAD_TTL_MS);
    assert_eq!(service.execute_at("alice", begin(), 20).unwrap(), first);
    assert_eq!(
        service.clone().execute_at("alice", begin(), 20).unwrap(),
        first
    );
    assert_eq!(
        root.service().execute_at("alice", begin(), 20),
        Err(UploadControlError::Busy),
        "An independent service cannot steal the live upload ledger"
    );
    assert_eq!(
        service.execute_at(
            "bob",
            UploadCommand::Status {
                handle: first.handle.clone()
            },
            20
        ),
        Err(UploadControlError::NotFound)
    );
    assert_eq!(
        service.execute_at(
            "bob",
            UploadCommand::Abort {
                handle: first.handle.clone()
            },
            20
        ),
        Err(UploadControlError::NotFound)
    );
    let partial = service
        .execute_at(
            "alice",
            UploadCommand::Chunk {
                handle: first.handle.clone(),
                offset: 0,
                data_base64: STANDARD.encode(b"te"),
                chunk_sha256: digest(b"te"),
            },
            20,
        )
        .unwrap();
    assert_eq!(partial.accepted_bytes, 2);
    drop(service);
    let reopened = root.service();
    assert_eq!(reopened.execute_at("alice", begin(), 30).unwrap(), partial);
    let aborted = reopened
        .execute_at(
            "alice",
            UploadCommand::Abort {
                handle: first.handle.clone(),
            },
            40,
        )
        .unwrap();
    assert_eq!(aborted.phase, UploadPhase::Aborted);
    assert_eq!(aborted.expires_at_ms, first.expires_at_ms);
    assert_eq!(reopened.execute_at("alice", begin(), 50).unwrap(), aborted);
    let next = reopened
        .execute_at("alice", begin(), first.expires_at_ms)
        .unwrap();
    assert_ne!(next.handle, first.handle);
}

#[test]
fn decoding_is_bounded_before_storage_and_checks_the_decoded_boundary_too() {
    let unavailable = AppPackageUploadControl::new(PathBuf::from("relative-state.db"));
    let request = |data_base64| UploadCommand::Chunk {
        handle: format!("upload_{:064x}", 0),
        offset: 0,
        data_base64,
        chunk_sha256: digest(b"test"),
    };
    assert_eq!(
        unavailable.execute_at(
            "alice",
            request("A".repeat(MAX_ENCODED_UPLOAD_CHUNK_BYTES + 1)),
            1
        ),
        Err(UploadControlError::LimitExceeded)
    );
    // Without final padding, this fits the encoded cap but decodes one byte
    // above 512 KiB. A cap on encoded text alone would admit too much data.
    assert_eq!(
        unavailable.execute_at(
            "alice",
            request("A".repeat(MAX_ENCODED_UPLOAD_CHUNK_BYTES)),
            1
        ),
        Err(UploadControlError::LimitExceeded)
    );
    for invalid in [String::new(), "not base64!".into(), "dGVzdA".into()] {
        assert_eq!(
            unavailable.execute_at("alice", request(invalid), 1),
            Err(UploadControlError::InvalidRequest)
        );
    }
    let bytes = vec![7; MAX_UPLOAD_CHUNK_BYTES];
    let root = Scratch::new();
    let service = root.service();
    let status = service
        .execute_at(
            "alice",
            UploadCommand::Begin {
                request_id: "maximum-chunk".into(),
                expected_size: bytes.len() as u64,
                sha256: digest(&bytes),
            },
            1,
        )
        .unwrap();
    let status = service
        .execute_at(
            "alice",
            UploadCommand::Chunk {
                handle: status.handle,
                offset: 0,
                data_base64: STANDARD.encode(&bytes),
                chunk_sha256: digest(&bytes),
            },
            2,
        )
        .unwrap();
    assert_eq!(status.accepted_bytes, MAX_UPLOAD_CHUNK_BYTES as u64);
}

#[test]
fn storage_failures_are_safe_and_lazy_initialization_can_retry() {
    let root = Scratch::new();
    let missing_parent = root.0.join("missing");
    let service = AppPackageUploadControl::new(missing_parent.join("state.db"));
    assert_eq!(
        service.execute_at("alice", begin(), 1),
        Err(UploadControlError::StorageUnavailable)
    );
    assert!(
        !missing_parent.exists(),
        "The adapter never creates parent paths"
    );
    fs::create_dir(&missing_parent).unwrap();
    assert!(service.execute_at("alice", begin(), 2).is_ok());

    let linked_parent = root.0.join("linked");
    symlink(&missing_parent, &linked_parent).unwrap();
    let linked = AppPackageUploadControl::new(linked_parent.join("other.db"));
    assert_eq!(
        linked.execute_at("alice", begin(), 2),
        Err(UploadControlError::StorageUnavailable)
    );
    for error in [
        UploadError::Io(std::io::Error::other("private /host/path must not escape")),
        UploadError::UnsafeEntry,
        UploadError::CorruptState,
    ] {
        assert_eq!(upload_error(error), UploadControlError::StorageUnavailable);
    }
}

#[tokio::test]
async fn execute_assigns_server_time_and_releases_shared_admission_after_completion() {
    let root = Scratch::new();
    let service = root.service();
    let admission = Arc::new(Semaphore::new(1));
    let permit = admission.clone().try_acquire_owned().unwrap();
    let before = crate::session::unix_epoch_ms();
    let status = service
        .execute("alice".into(), begin(), permit)
        .await
        .unwrap();
    let after = crate::session::unix_epoch_ms();
    assert!(status.expires_at_ms >= before + UPLOAD_TTL_MS);
    assert!(status.expires_at_ms <= after + UPLOAD_TTL_MS);
    assert_eq!(admission.available_permits(), 1);
}

#[tokio::test]
async fn cancelled_awaiter_cannot_release_admission_while_blocking_work_is_live() {
    let admission = Arc::new(Semaphore::new(1));
    let permit = admission.clone().try_acquire_owned().unwrap();
    let (started, did_start) = tokio::sync::oneshot::channel();
    let (release, until_released) = std::sync::mpsc::channel();
    let task = tokio::spawn(admitted_blocking(permit, move || {
        started.send(()).unwrap();
        until_released.recv().unwrap();
        Ok(())
    }));
    did_start.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(admission.available_permits(), 0);
    assert!(admission.clone().try_acquire_owned().is_err());
    release.send(()).unwrap();
    let permit = tokio::time::timeout(std::time::Duration::from_secs(2), admission.acquire_owned())
        .await
        .expect("The worker must release admission after completion")
        .unwrap();
    drop(permit);
}
