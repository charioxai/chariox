#![cfg(any(target_os = "macos", target_os = "linux"))]

use chariox_app_runtime::package_upload::{
    PackageUploadStore, UploadCheckpoint, UploadError, UploadLimits, UploadPhase,
    MAX_UPLOAD_ARCHIVE_BYTES, MAX_UPLOAD_CHUNK_BYTES,
};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Root {
    parent: PathBuf,
    path: PathBuf,
}

impl Root {
    fn new() -> Self {
        let parent = std::env::temp_dir().join(format!(
            "chariox-upload-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&parent).unwrap();
        let parent = fs::canonicalize(parent).unwrap();
        let path = parent.join("uploads");
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self { parent, path }
    }
    fn open(&self) -> PackageUploadStore {
        PackageUploadStore::open(&self.path, UploadLimits::default(), 1).unwrap()
    }
    fn archive(&self, handle: &str) -> PathBuf {
        self.path.join(format!("{handle}.cxapp"))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.parent).unwrap();
    }
}

fn request_id() -> String {
    format!("request-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn fault() -> UploadError {
    UploadError::Io(std::io::Error::other("injected interruption"))
}

#[test]
fn chunks_resume_after_reopen_and_finalize_an_anchored_read_only_file() {
    let root = Root::new();
    let store = root.open();
    let bytes = vec![42_u8; MAX_UPLOAD_CHUNK_BYTES + 17];
    let upload = store
        .begin(
            "owner",
            &request_id(),
            bytes.len() as u64,
            &digest(&bytes),
            100,
            1,
        )
        .unwrap();
    let first = &bytes[..MAX_UPLOAD_CHUNK_BYTES];
    store
        .chunk("owner", &upload.handle, 0, first, &digest(first), 2)
        .unwrap();
    drop(store);
    let store = root.open();
    let retry = store
        .chunk("owner", &upload.handle, 0, first, &digest(first), 3)
        .unwrap();
    assert_eq!(retry.accepted_bytes, MAX_UPLOAD_CHUNK_BYTES as u64);
    let last = &bytes[MAX_UPLOAD_CHUNK_BYTES..];
    store
        .chunk(
            "owner",
            &upload.handle,
            retry.accepted_bytes,
            last,
            &digest(last),
            4,
        )
        .unwrap();
    let artifact = store.finalize("owner", &upload.handle, 5).unwrap();
    assert_eq!(artifact.status().phase, UploadPhase::Finalized);
    let mut read = artifact.file();
    let mut actual = Vec::new();
    read.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, bytes);
    assert!(read.write_all(b"forbidden").is_err());
    assert_eq!(
        artifact.file().metadata().unwrap().permissions().mode() & 0o777,
        0o400
    );
    assert!(matches!(
        store.finalize("owner", &upload.handle, 6),
        Err(UploadError::Busy)
    ));
    assert!(matches!(
        store.abort("owner", &upload.handle, 6),
        Err(UploadError::Busy)
    ));
    drop(artifact);
    assert!(matches!(
        store.chunk("owner", &upload.handle, 0, first, &digest(first), 6),
        Err(UploadError::Conflict)
    ));
    drop(store);
    // Simulate interruption after phase commit but before permission sealing.
    fs::set_permissions(
        root.archive(&upload.handle),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let store = root.open();
    let artifact = store.finalize("owner", &upload.handle, 7).unwrap();
    assert_eq!(
        artifact.file().metadata().unwrap().permissions().mode() & 0o777,
        0o400
    );
}

#[test]
fn ownership_and_opaque_handles_apply_to_every_upload_operation() {
    let root = Root::new();
    let store = root.open();
    let upload = store
        .begin("alice", &request_id(), 4, &digest(b"test"), 100, 1)
        .unwrap();
    for handle in [&upload.handle, "../../outside", "upload_not-a-handle"] {
        assert!(matches!(
            store.status("bob", handle, 2),
            Err(UploadError::NotFound)
        ));
        assert!(matches!(
            store.chunk("bob", handle, 0, b"test", &digest(b"test"), 2),
            Err(UploadError::NotFound)
        ));
        assert!(matches!(
            store.finalize("bob", handle, 2),
            Err(UploadError::NotFound)
        ));
        assert!(matches!(
            store.abort("bob", handle, 2),
            Err(UploadError::NotFound)
        ));
    }
    assert_eq!(
        store
            .status("alice", &upload.handle, 2)
            .unwrap()
            .accepted_bytes,
        0
    );
    assert!(store
        .begin("", &request_id(), 1, &digest(b"x"), 100, 1)
        .is_err());
    assert!(store.status("", &upload.handle, 2).is_err());
    assert!(store
        .chunk("", &upload.handle, 0, b"test", &digest(b"test"), 2)
        .is_err());
    assert!(store.finalize("", &upload.handle, 2).is_err());
    assert!(store.abort("", &upload.handle, 2).is_err());
}

#[test]
fn counts_reserved_bytes_and_expiry_are_bounded_for_each_owner_and_store() {
    let root = Root::new();
    let limits = UploadLimits {
        max_uploads: 2,
        max_uploads_per_owner: 1,
        max_reserved_bytes: 10,
        max_reserved_bytes_per_owner: 8,
        max_ttl_ms: 100,
    };
    let store = PackageUploadStore::open(&root.path, limits, 1).unwrap();
    let first = store
        .begin("alice", &request_id(), 8, &digest(b"12345678"), 10, 1)
        .unwrap();
    assert!(matches!(
        store.begin("alice", &request_id(), 1, &digest(b"x"), 10, 1),
        Err(UploadError::Limit)
    ));
    assert!(matches!(
        store.begin("bob", &request_id(), 3, &digest(b"xxx"), 10, 1),
        Err(UploadError::Limit)
    ));
    let second = store
        .begin("bob", &request_id(), 2, &digest(b"xx"), 10, 1)
        .unwrap();
    assert_ne!(first.handle, second.handle);
    assert!(matches!(
        store.begin("carol", &request_id(), 1, &digest(b"x"), 10, 1),
        Err(UploadError::Limit)
    ));
    assert!(store
        .begin("alice", &request_id(), 1, &digest(b"x"), 102, 1)
        .is_err());
    assert!(store
        .begin("alice", &request_id(), 1, &digest(b"x"), 1, 1)
        .is_err());
    store.abort("alice", &first.handle, 2).unwrap();
    assert!(!root.archive(&first.handle).exists());
    store.cleanup_expired(10).unwrap();
    assert!(!root.archive(&second.handle).exists());
    assert!(matches!(
        store.status("bob", &second.handle, 10),
        Err(UploadError::NotFound)
    ));
    store
        .begin("carol", &request_id(), 8, &digest(b"12345678"), 20, 10)
        .unwrap();
}

#[test]
fn invalid_sizes_hashes_offsets_and_conflicting_retries_never_advance_offset() {
    let root = Root::new();
    let store = root.open();
    for size in [0, MAX_UPLOAD_ARCHIVE_BYTES + 1, u64::MAX] {
        assert!(store
            .begin("owner", &request_id(), size, &digest(b"x"), 100, 1)
            .is_err());
    }
    assert!(store
        .begin("owner", &request_id(), 1, "not-a-digest", 100, 1)
        .is_err());
    let upload = store
        .begin("owner", &request_id(), 8, &digest(b"12345678"), 100, 1)
        .unwrap();
    assert!(matches!(
        store.finalize("owner", &upload.handle, 2),
        Err(UploadError::Incomplete)
    ));
    store
        .chunk("owner", &upload.handle, 0, b"1234", &digest(b"1234"), 2)
        .unwrap();
    for (offset, bytes) in [
        (0, b"xxxx".as_slice()),
        (3, b"456".as_slice()),
        (5, b"6".as_slice()),
        (u64::MAX, b"1".as_slice()),
    ] {
        assert!(matches!(
            store.chunk("owner", &upload.handle, offset, bytes, &digest(bytes), 3),
            Err(UploadError::Conflict)
        ));
    }
    assert!(matches!(
        store.chunk("owner", &upload.handle, 4, b"5678", &digest(b"wrong"), 3),
        Err(UploadError::DigestMismatch)
    ));
    assert!(store
        .chunk("owner", &upload.handle, 4, b"", &digest(b""), 3)
        .is_err());
    let oversized = vec![0; MAX_UPLOAD_CHUNK_BYTES + 1];
    assert!(matches!(
        store.chunk(
            "owner",
            &upload.handle,
            4,
            &oversized,
            &digest(&oversized),
            3
        ),
        Err(UploadError::Limit)
    ));
    assert_eq!(
        store
            .status("owner", &upload.handle, 3)
            .unwrap()
            .accepted_bytes,
        4
    );
    assert_eq!(fs::read(root.archive(&upload.handle)).unwrap(), b"1234");
    store
        .chunk("owner", &upload.handle, 4, b"xxxx", &digest(b"xxxx"), 4)
        .unwrap();
    assert!(matches!(
        store.finalize("owner", &upload.handle, 5),
        Err(UploadError::DigestMismatch)
    ));
}

#[test]
fn interrupted_chunk_truncates_only_uncommitted_suffix_on_reopen() {
    for interruption in [
        UploadCheckpoint::ArchiveSynced,
        UploadCheckpoint::BeforeStateCommit,
    ] {
        let root = Root::new();
        let store = root.open();
        let upload = store
            .begin("owner", &request_id(), 8, &digest(b"12345678"), 100, 1)
            .unwrap();
        store
            .chunk("owner", &upload.handle, 0, b"1234", &digest(b"1234"), 2)
            .unwrap();
        assert!(store
            .chunk_with_checkpoint(
                "owner",
                &upload.handle,
                4,
                b"5678",
                &digest(b"5678"),
                3,
                |point| if point == interruption {
                    Err(fault())
                } else {
                    Ok(())
                }
            )
            .is_err());
        assert_eq!(fs::metadata(root.archive(&upload.handle)).unwrap().len(), 8);
        drop(store);
        let store = root.open();
        assert_eq!(fs::read(root.archive(&upload.handle)).unwrap(), b"1234");
        assert_eq!(
            store
                .status("owner", &upload.handle, 4)
                .unwrap()
                .accepted_bytes,
            4
        );
        store
            .chunk("owner", &upload.handle, 4, b"5678", &digest(b"5678"), 5)
            .unwrap();
        assert!(store.finalize("owner", &upload.handle, 6).is_ok());
    }
}

#[test]
fn lost_ack_after_state_commit_reloads_offset_and_accepts_identical_retry() {
    let root = Root::new();
    let store = root.open();
    let upload = store
        .begin("owner", &request_id(), 4, &digest(b"test"), 100, 1)
        .unwrap();
    assert!(store
        .chunk_with_checkpoint(
            "owner",
            &upload.handle,
            0,
            b"test",
            &digest(b"test"),
            2,
            |point| if point == UploadCheckpoint::StateCommitted {
                Err(fault())
            } else {
                Ok(())
            }
        )
        .is_err());
    assert_eq!(
        store
            .status("owner", &upload.handle, 3)
            .unwrap()
            .accepted_bytes,
        4
    );
    store
        .chunk("owner", &upload.handle, 0, b"test", &digest(b"test"), 3)
        .unwrap();
    drop(store);
    let store = root.open();
    assert!(store.finalize("owner", &upload.handle, 4).is_ok());
}

#[test]
fn file_leases_keep_reservations_and_root_ownership_until_verification_finishes() {
    let root = Root::new();
    let limits = UploadLimits {
        max_uploads: 1,
        max_uploads_per_owner: 1,
        max_reserved_bytes: 4,
        max_reserved_bytes_per_owner: 4,
        ..UploadLimits::default()
    };
    let store = PackageUploadStore::open(&root.path, limits, 1).unwrap();
    let upload = store
        .begin("owner", &request_id(), 4, &digest(b"test"), 10, 1)
        .unwrap();
    store
        .chunk("owner", &upload.handle, 0, b"test", &digest(b"test"), 2)
        .unwrap();
    let artifact = store.finalize("owner", &upload.handle, 3).unwrap();
    store.cleanup_expired(10).unwrap();
    assert!(root.archive(&upload.handle).exists());
    assert!(matches!(
        store.begin("other", &request_id(), 4, &digest(b"next"), 20, 10),
        Err(UploadError::Limit)
    ));
    drop(store);
    assert!(matches!(
        PackageUploadStore::open(&root.path, limits, 10),
        Err(UploadError::Busy)
    ));
    drop(artifact);
    let store = PackageUploadStore::open(&root.path, limits, 10).unwrap();
    assert!(!root.archive(&upload.handle).exists());
    store
        .begin("other", &request_id(), 4, &digest(b"next"), 20, 10)
        .unwrap();
}

#[test]
fn concurrent_writes_return_busy_instead_of_queueing_unbounded_work() {
    let root = Root::new();
    let store = root.open();
    let upload = store
        .begin("owner", &request_id(), 4, &digest(b"test"), 100, 1)
        .unwrap();
    let (arrived, ready) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let worker = store.clone();
    let handle = upload.handle.clone();
    let task = std::thread::spawn(move || {
        worker.chunk_with_checkpoint("owner", &handle, 0, b"test", &digest(b"test"), 2, |point| {
            if point == UploadCheckpoint::ArchiveSynced {
                arrived.send(()).unwrap();
                resume.recv().unwrap();
            }
            Ok(())
        })
    });
    ready
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let status = store.status("owner", &upload.handle, 2);
    let chunk = store.chunk("owner", &upload.handle, 0, b"test", &digest(b"test"), 2);
    release.send(()).unwrap();
    assert!(task.join().unwrap().is_ok());
    assert!(matches!(status, Err(UploadError::Busy)));
    assert!(matches!(chunk, Err(UploadError::Busy)));
}

#[test]
fn descriptor_anchoring_survives_root_path_replacement_and_rejects_file_links() {
    let root = Root::new();
    let store = root.open();
    let moved = root.parent.join("anchored");
    fs::rename(&root.path, &moved).unwrap();
    fs::create_dir(&root.path).unwrap();
    let upload = store
        .begin("owner", &request_id(), 4, &digest(b"test"), 100, 1)
        .unwrap();
    assert!(!root.archive(&upload.handle).exists());
    let actual = moved.join(format!("{}.cxapp", upload.handle));
    assert!(actual.exists());
    let sentinel = root.parent.join("sentinel");
    fs::write(&sentinel, b"preserve me").unwrap();
    fs::remove_file(&actual).unwrap();
    symlink(&sentinel, &actual).unwrap();
    assert!(store
        .chunk("owner", &upload.handle, 0, b"test", &digest(b"test"), 2)
        .is_err());
    fs::remove_file(&actual).unwrap();
    fs::hard_link(&sentinel, &actual).unwrap();
    assert!(store
        .chunk("owner", &upload.handle, 0, b"test", &digest(b"test"), 2)
        .is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"preserve me");
    let linked = root.parent.join("linked");
    symlink(&moved, &linked).unwrap();
    assert!(PackageUploadStore::open(&linked, UploadLimits::default(), 1).is_err());
}

#[test]
fn restart_cleans_own_orphans_but_rejects_lost_accepted_bytes() {
    let root = Root::new();
    let store = root.open();
    let upload = store
        .begin("owner", &request_id(), 4, &digest(b"test"), 100, 1)
        .unwrap();
    store
        .chunk("owner", &upload.handle, 0, b"test", &digest(b"test"), 2)
        .unwrap();
    drop(store);
    let orphan = root.path.join(format!("upload_{}.cxapp", "a".repeat(64)));
    fs::write(&orphan, b"orphan").unwrap();
    let temporary = root.path.join(".replace-123-1-a");
    fs::write(&temporary, b"interrupted state").unwrap();
    let store = root.open();
    assert!(!orphan.exists());
    assert!(!temporary.exists());
    drop(store);
    OpenOptions::new()
        .write(true)
        .open(root.archive(&upload.handle))
        .unwrap()
        .set_len(2)
        .unwrap();
    assert!(matches!(
        PackageUploadStore::open(&root.path, UploadLimits::default(), 1),
        Err(UploadError::CorruptState)
    ));
}

#[test]
fn begin_request_id_survives_lost_reply_reopen_and_payload_conflicts() {
    let root = Root::new();
    let store = root.open();
    let first = store
        .begin("alice", "same-request", 4, &digest(b"test"), 100, 1)
        .unwrap();
    store
        .chunk("alice", &first.handle, 0, b"te", &digest(b"te"), 2)
        .unwrap();
    drop(store); // Treat the original begin response as lost in transport.
    let store = root.open();
    let retry = store
        .begin("alice", "same-request", 4, &digest(b"test"), 200, 3)
        .unwrap();
    assert_eq!(retry.handle, first.handle);
    assert_eq!(retry.accepted_bytes, 2);
    assert_eq!(retry.expires_at_ms, 100); // A retry does not renew the upload.
    assert!(matches!(
        store.begin("alice", "same-request", 5, &digest(b"test"), 200, 3),
        Err(UploadError::Conflict)
    ));
    assert!(matches!(
        store.begin("alice", "same-request", 4, &digest(b"next"), 200, 3),
        Err(UploadError::Conflict)
    ));
    let bob = store
        .begin("bob", "same-request", 4, &digest(b"test"), 200, 3)
        .unwrap();
    assert_ne!(bob.handle, first.handle);
    assert!(matches!(
        store.status("bob", &first.handle, 3),
        Err(UploadError::NotFound)
    ));
    assert_eq!(store.status("alice", &first.handle, 3).unwrap(), retry);
    for id in ["".to_string(), "\n".to_string(), "x".repeat(129)] {
        assert!(matches!(
            store.begin("alice", &id, 4, &digest(b"test"), 100, 3),
            Err(UploadError::Invalid("request ID"))
        ));
    }
}

#[test]
fn aborted_begin_receipt_prevents_resurrection_and_expires_without_unbounded_metadata() {
    let root = Root::new();
    let limits = UploadLimits {
        max_uploads: 1,
        max_uploads_per_owner: 1,
        max_reserved_bytes: 4,
        max_reserved_bytes_per_owner: 4,
        ..UploadLimits::default()
    };
    let store = PackageUploadStore::open(&root.path, limits, 1).unwrap();
    let first = store
        .begin("alice", "cancelled", 4, &digest(b"test"), 10, 1)
        .unwrap();
    store
        .chunk("alice", &first.handle, 0, b"test", &digest(b"test"), 2)
        .unwrap();
    store.abort("alice", &first.handle, 3).unwrap();
    store.abort("alice", &first.handle, 3).unwrap();
    assert!(!root.archive(&first.handle).exists());
    drop(store);
    let store = PackageUploadStore::open(&root.path, limits, 4).unwrap();
    let receipt = store
        .begin("alice", "cancelled", 4, &digest(b"test"), 20, 4)
        .unwrap();
    assert_eq!(receipt.handle, first.handle);
    assert_eq!(receipt.phase, UploadPhase::Aborted);
    assert_eq!(receipt.expires_at_ms, 10);
    assert!(matches!(
        store.finalize("alice", &first.handle, 4),
        Err(UploadError::Conflict)
    ));
    assert!(matches!(
        store.chunk("alice", &first.handle, 0, b"test", &digest(b"test"), 4),
        Err(UploadError::Conflict)
    ));
    assert!(matches!(
        store.begin("alice", "new-request", 4, &digest(b"test"), 20, 4),
        Err(UploadError::Limit)
    ));
    assert!(matches!(
        store.begin("alice", "cancelled", 4, &digest(b"next"), 20, 4),
        Err(UploadError::Conflict)
    ));
    let next = store
        .begin("alice", "cancelled", 4, &digest(b"test"), 20, 10)
        .unwrap();
    assert_ne!(next.handle, first.handle);
    assert_eq!(next.phase, UploadPhase::Receiving);
    assert_eq!(next.accepted_bytes, 0);
}

#[test]
fn abort_receipt_keeps_metadata_bound_but_releases_disk_reservation() {
    let root = Root::new();
    let limits = UploadLimits {
        max_uploads: 2,
        max_uploads_per_owner: 2,
        max_reserved_bytes: 4,
        max_reserved_bytes_per_owner: 4,
        ..UploadLimits::default()
    };
    let store = PackageUploadStore::open(&root.path, limits, 1).unwrap();
    let first = store
        .begin("owner", "first", 4, &digest(b"test"), 100, 1)
        .unwrap();
    store
        .chunk("owner", &first.handle, 0, b"test", &digest(b"test"), 2)
        .unwrap();
    let lease = store.finalize("owner", &first.handle, 3).unwrap();
    assert!(matches!(
        store.abort("owner", &first.handle, 3),
        Err(UploadError::Busy)
    ));
    assert!(matches!(
        store.begin("owner", "second", 4, &digest(b"next"), 100, 3),
        Err(UploadError::Limit)
    ));
    drop(lease);
    store.abort("owner", &first.handle, 4).unwrap();
    assert!(!root.archive(&first.handle).exists());
    store
        .begin("owner", "second", 4, &digest(b"next"), 100, 4)
        .unwrap();
}

#[test]
fn rename_before_directory_sync_is_recovered_before_any_offset_is_acknowledged() {
    let root = Root::new();
    let store = root.open();
    let request = request_id();
    let upload = store
        .begin("owner", &request, 4, &digest(b"test"), 100, 1)
        .unwrap();
    let mut points = Vec::new();
    assert!(store
        .chunk_with_checkpoint(
            "owner",
            &upload.handle,
            0,
            b"test",
            &digest(b"test"),
            2,
            |point| {
                points.push(point);
                if point == UploadCheckpoint::StateRenamed {
                    // This callback is inside atomic_replace, after renameat
                    // and before its parent fsync, not after durable commit.
                    let visible: serde_json::Value =
                        serde_json::from_slice(&fs::read(root.path.join("uploads.json")).unwrap())
                            .unwrap();
                    assert_eq!(visible["uploads"][&upload.handle]["accepted_bytes"], 4);
                    return Err(fault());
                }
                Ok(())
            },
        )
        .is_err());
    assert_eq!(
        points,
        [
            UploadCheckpoint::ArchiveSynced,
            UploadCheckpoint::BeforeStateCommit,
            UploadCheckpoint::StateRenamed,
            UploadCheckpoint::BeforeRecoverySync,
            UploadCheckpoint::RecoverySynced,
        ]
    );
    assert_eq!(
        store
            .status("owner", &upload.handle, 3)
            .unwrap()
            .accepted_bytes,
        4
    );
    assert_eq!(
        store
            .begin("owner", &request, 4, &digest(b"test"), 100, 3)
            .unwrap()
            .accepted_bytes,
        4
    );
    store
        .chunk("owner", &upload.handle, 0, b"test", &digest(b"test"), 3)
        .unwrap();
    drop(store);
    let reopened = root.open();
    assert_eq!(
        reopened
            .status("owner", &upload.handle, 4)
            .unwrap()
            .accepted_bytes,
        4
    );
    assert_eq!(fs::read(root.archive(&upload.handle)).unwrap(), b"test");
}

#[test]
fn failed_recovery_sync_blocks_retries_and_cleanup_until_durable_reopen() {
    // An unsynced rename may survive a crash or may revert. Neither outcome may
    // cause an acknowledged chunk to be lost; the failed attempt stays unacked.
    for rename_survives in [true, false] {
        let root = Root::new();
        let store = root.open();
        let request = request_id();
        let upload = store
            .begin("owner", &request, 8, &digest(b"12345678"), 100, 1)
            .unwrap();
        store
            .chunk("owner", &upload.handle, 0, b"1234", &digest(b"1234"), 2)
            .unwrap();
        let old_state = fs::read(root.path.join("uploads.json")).unwrap();
        let mut saw_rename = false;
        let mut saw_recovery_attempt = false;
        assert!(store
            .chunk_with_checkpoint(
                "owner",
                &upload.handle,
                4,
                b"5678",
                &digest(b"5678"),
                3,
                |point| match point {
                    UploadCheckpoint::StateRenamed => {
                        saw_rename = true;
                        Err(fault())
                    }
                    UploadCheckpoint::BeforeRecoverySync => {
                        saw_recovery_attempt = true;
                        Err(fault())
                    }
                    UploadCheckpoint::RecoverySynced => panic!("Recovery sync was denied"),
                    _ => Ok(()),
                },
            )
            .is_err());
        assert!(saw_rename && saw_recovery_attempt);
        assert!(matches!(
            store.status("owner", &upload.handle, 4),
            Err(UploadError::Unavailable)
        ));
        assert!(matches!(
            store.begin("owner", &request, 8, &digest(b"12345678"), 100, 4),
            Err(UploadError::Unavailable)
        ));
        assert!(matches!(
            store.chunk("owner", &upload.handle, 4, b"5678", &digest(b"5678"), 4),
            Err(UploadError::Unavailable)
        ));
        assert!(matches!(
            store.abort("owner", &upload.handle, 4),
            Err(UploadError::Unavailable)
        ));
        assert!(matches!(
            store.finalize("owner", &upload.handle, 4),
            Err(UploadError::Unavailable)
        ));
        assert!(matches!(
            store.cleanup_expired(200),
            Err(UploadError::Unavailable)
        ));
        assert_eq!(fs::read(root.archive(&upload.handle)).unwrap(), b"12345678");
        drop(store);
        if !rename_survives {
            // Deterministically model the older metadata surviving a crash.
            let mut metadata = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(root.path.join("uploads.json"))
                .unwrap();
            metadata.write_all(&old_state).unwrap();
            metadata.sync_all().unwrap();
        }
        let reopened = root.open();
        let expected = if rename_survives { 8 } else { 4 };
        assert_eq!(
            reopened
                .status("owner", &upload.handle, 5)
                .unwrap()
                .accepted_bytes,
            expected
        );
        assert_eq!(
            fs::metadata(root.archive(&upload.handle)).unwrap().len(),
            expected
        );
        assert!(fs::read(root.archive(&upload.handle))
            .unwrap()
            .starts_with(b"1234"));
    }
}
