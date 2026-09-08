#![cfg(any(target_os = "macos", target_os = "linux"))]

use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::release_store::{
    CleanupCheckpoint, ReleaseStore, ReleaseStoreError, StageBudget, StageCheckpoint,
};
use ed25519_dalek::SigningKey;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Barrier,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        // macOS's /var alias is a symlink; use the actual system temp ancestor.
        let base = fs::canonicalize(std::env::temp_dir()).unwrap();
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = base.join(format!(
            "chariox-release-store-{}-{time}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        permissions(&root, 0o700);
        Self(root)
    }
    fn child(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir(&path).unwrap();
        permissions(&path, 0o700);
        path
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fn writable(path: &Path) {
            if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
                permissions(path, 0o700);
                for entry in fs::read_dir(path).unwrap() {
                    writable(&entry.unwrap().path());
                }
            }
        }
        writable(&self.0);
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn permissions(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn package() -> (Vec<u8>, VerificationPolicy) {
    let key = SigningKey::from_bytes(&[27; 32]);
    let manifest: Manifest = serde_json::from_value(json!({
        "schema":"chariox.app.v1", "appId":"com.example.release", "version":"1.0.0",
        "publisher":{"id":"com.example","keyId":"developer-1","name":"Developer"},
        "sdkVersion":"0.5.0", "appContractVersion":1, "minKernelProtocol":500,
        "resourcePolicy":"chariox.app.resources.v1",
        "runtime":{"engine":"node","entry":"runtime/main.js"},
        "ui":{"entry":"ui/index.html"}, "capabilities":{}
    }))
    .unwrap();
    let files = BTreeMap::from([
        (
            "runtime/main.js".to_owned(),
            b"export default function register() {}".to_vec(),
        ),
        (
            "ui/index.html".to_owned(),
            b"<!doctype html><title>Release fixture</title>".to_vec(),
        ),
    ]);
    let archive = pack(&manifest, &files, &key, &Limits::default()).unwrap();
    let policy = VerificationPolicy::new(
        500,
        vec![TrustedPublisher {
            publisher_id: "com.example".to_owned(),
            key_id: "developer-1".to_owned(),
            public_key: key.verifying_key(),
        }],
    );
    (archive, policy)
}
fn budget() -> StageBudget {
    StageBudget {
        max_stage_bytes: 1024 * 1024,
        reserved_bytes: 1024 * 1024,
        host_reserve_bytes: 1024 * 1024,
    }
}

#[test]
fn ui_asset_reads_require_signed_ui_inventory_and_bounded_allocation() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let package = verify(&archive, &policy).unwrap();
    let mut files: BTreeMap<String, Vec<u8>> = package
        .files()
        .map(|(path, bytes)| (path.into(), bytes.to_vec()))
        .collect();
    files.insert("ui/assets/main.css".into(), b"body { margin: 0 }".to_vec());
    let archive = pack(
        package.manifest(),
        &files,
        &SigningKey::from_bytes(&[27; 32]),
        &Limits::default(),
    )
    .unwrap();
    let package = verify(&archive, &policy).unwrap();
    store.stage(&package, &archive, budget()).unwrap();
    let lease = store.lease_verified(&package, &archive).unwrap();
    assert_eq!(
        lease.read_ui_asset("index.html", 1024).unwrap(),
        files["ui/index.html"]
    );
    assert_eq!(
        lease.read_ui_asset("assets/main.css", 1024).unwrap(),
        files["ui/assets/main.css"]
    );
    assert!(matches!(
        lease.read_ui_asset("index.html", 1),
        Err(ReleaseStoreError::ReservationExceeded)
    ));
    for path in [
        "",
        "/index.html",
        "ui/index.html",
        "../runtime/main.js",
        "runtime/main.js",
        "%2e%2e/runtime/main.js",
        "assets/../index.html",
        "assets\\main.css",
        "missing.js",
    ] {
        assert!(
            matches!(
                lease.read_ui_asset(path, 1024),
                Err(ReleaseStoreError::UnsafeEntry)
            ),
            "{path}"
        );
    }
}

#[test]
fn ui_asset_reads_reject_content_changes_and_symlinks_after_lease_verification() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let package = verify(&archive, &policy).unwrap();
    let staged = store.stage(&package, &archive, budget()).unwrap();
    let lease = store.lease_verified(&package, &archive).unwrap();
    let ui = staged.path.join("payload/ui");
    let entry = ui.join("index.html");
    let original = package.file("ui/index.html").unwrap();
    permissions(&entry, 0o600);
    fs::write(&entry, vec![b'x'; original.len()]).unwrap();
    permissions(&entry, 0o400);
    assert!(matches!(
        lease.read_ui_asset("index.html", 1024),
        Err(ReleaseStoreError::InvalidExisting)
    ));
    permissions(&entry, 0o600);
    fs::write(&entry, original).unwrap();
    permissions(&entry, 0o400);
    assert_eq!(lease.read_ui_asset("index.html", 1024).unwrap(), original);
    permissions(&ui, 0o700);
    fs::remove_file(&entry).unwrap();
    symlink("../runtime/main.js", &entry).unwrap();
    permissions(&ui, 0o500);
    assert!(lease.read_ui_asset("index.html", 1024).is_err());
}

#[test]
fn stored_archive_reopen_retains_exact_bytes_and_shared_generation_lock() {
    use std::os::fd::AsRawFd;
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let package = verify(&archive, &policy).unwrap();
    let staged = store.stage(&package, &archive, budget()).unwrap();
    let mut stored = store
        .open_stored_archive(&package.package_digest())
        .unwrap();
    assert_eq!(stored.read_bytes().unwrap(), archive);
    assert_eq!(stored.read_bytes().unwrap(), archive);
    let observer = fs::File::open(&staged.path).unwrap();
    assert_ne!(
        unsafe { libc::flock(observer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    drop(stored);
    assert_eq!(
        unsafe { libc::flock(observer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
}

#[test]
fn stored_archive_rejects_tampering_oversize_and_link_replacement_before_verification() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let package = verify(&archive, &policy).unwrap();
    let staged = store.stage(&package, &archive, budget()).unwrap();
    let path = staged.path.join("envelope.cxapp");
    permissions(&path, 0o600);
    let mut bad = archive.clone();
    bad[0] ^= 1;
    fs::write(&path, bad).unwrap();
    permissions(&path, 0o400);
    let mut stored = store
        .open_stored_archive(&package.package_digest())
        .unwrap();
    assert!(matches!(
        stored.read_bytes(),
        Err(ReleaseStoreError::ArchiveMismatch)
    ));
    drop(stored);
    permissions(&path, 0o600);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(128 * 1024 * 1024 + 1)
        .unwrap();
    permissions(&path, 0o400);
    let mut stored = store
        .open_stored_archive(&package.package_digest())
        .unwrap();
    assert!(matches!(
        stored.read_bytes(),
        Err(ReleaseStoreError::ReservationExceeded)
    ));
    drop(stored);
    permissions(&staged.path, 0o700);
    fs::remove_file(&path).unwrap();
    symlink("payload/runtime/main.js", &path).unwrap();
    permissions(&staged.path, 0o500);
    assert!(store
        .open_stored_archive(&package.package_digest())
        .is_err());
}

#[test]
fn verified_worker_lease_retains_shared_publication_lock_until_last_consumer_drains() {
    use std::os::fd::AsRawFd;
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let package = verify(&archive, &policy).unwrap();
    let staged = store.stage(&package, &archive, budget()).unwrap();
    let first = store.lease_verified(&package, &archive).unwrap();
    let second = store.lease_verified(&package, &archive).unwrap();
    assert_eq!(first.package_digest(), package.package_digest());
    assert_eq!(first.manifest(), package.manifest());
    assert_eq!(first.declarations(), package.declarations());
    let observer = fs::File::open(&staged.path).unwrap();
    let exclusive = || unsafe { libc::flock(observer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_ne!(exclusive(), 0);
    assert!(matches!(
        store.stage(&package, &archive, budget()),
        Err(ReleaseStoreError::Busy)
    ));
    drop(first);
    assert_ne!(exclusive(), 0);
    drop(second);
    assert_eq!(exclusive(), 0);
    assert!(matches!(
        store.lease_verified(&package, &archive),
        Err(ReleaseStoreError::Busy)
    ));
}

#[test]
fn worker_lease_rejects_archive_mismatch_payload_tampering_and_symlink_substitution() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let package = verify(&archive, &policy).unwrap();
    let staged = store.stage(&package, &archive, budget()).unwrap();
    let mut wrong_archive = archive.clone();
    wrong_archive[0] ^= 1;
    assert!(matches!(
        store.lease_verified(&package, &wrong_archive),
        Err(ReleaseStoreError::ArchiveMismatch)
    ));
    let entry = staged.path.join("payload/runtime/main.js");
    permissions(&entry, 0o600);
    fs::write(&entry, b"unverified runtime entry").unwrap();
    permissions(&entry, 0o400);
    assert!(matches!(
        store.lease_verified(&package, &archive),
        Err(ReleaseStoreError::InvalidExisting)
    ));
    permissions(entry.parent().unwrap(), 0o700);
    fs::remove_file(&entry).unwrap();
    symlink("../../envelope.cxapp", &entry).unwrap();
    permissions(entry.parent().unwrap(), 0o500);
    assert!(matches!(
        store.lease_verified(&package, &archive),
        Err(ReleaseStoreError::UnsafeEntry)
    ));
}
fn names(root: &Path) -> Vec<String> {
    let mut entries: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    entries
}
fn stage_path(root: &Path) -> PathBuf {
    let name = names(root)
        .into_iter()
        .find(|name| name.starts_with(".stage-"))
        .unwrap();
    root.join(name)
}

#[test]
fn verified_release_is_readonly_and_keeps_original_archive_for_reverification() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let staged = store.stage(&verified, &archive, budget()).unwrap();
    assert!(!staged.reused);
    assert_eq!(staged.package_digest, verified.package_digest());
    assert_eq!(
        names(&directory.0),
        [verified.package_digest().strip_prefix("sha256:").unwrap()]
    );
    assert_eq!(
        fs::read(staged.path.join("envelope.cxapp")).unwrap(),
        archive
    );
    for (path, expected) in verified.files() {
        let file = staged.path.join("payload").join(path);
        assert_eq!(fs::read(&file).unwrap(), expected);
        assert_eq!(fs::metadata(file).unwrap().permissions().mode() & 0o222, 0);
    }
    assert_eq!(
        staged.directory.metadata().unwrap().permissions().mode() & 0o222,
        0
    );
    let saved = fs::read(staged.path.join("envelope.cxapp")).unwrap();
    assert_eq!(
        verify(&saved, &policy).unwrap().package_digest(),
        verified.package_digest()
    );
    let zero_budget = StageBudget {
        max_stage_bytes: 0,
        reserved_bytes: 0,
        host_reserve_bytes: 0,
    };
    assert!(
        store
            .stage(&verified, &archive, zero_budget)
            .unwrap()
            .reused
    );
}

#[test]
fn archive_mismatch_and_budget_fail_before_any_stage_exists() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    assert!(matches!(
        store.stage(&verified, b"different archive", budget()),
        Err(ReleaseStoreError::ArchiveMismatch)
    ));
    let mut insufficient = budget();
    insufficient.reserved_bytes = 1;
    assert!(matches!(
        store.stage(&verified, &archive, insufficient),
        Err(ReleaseStoreError::ReservationExceeded)
    ));
    let mut exhausted_host = budget();
    exhausted_host.host_reserve_bytes = u64::MAX;
    assert!(matches!(
        store.stage(&verified, &archive, exhausted_host),
        Err(ReleaseStoreError::HostReserve)
    ));
    assert!(names(&directory.0).is_empty());
}

#[test]
fn exact_reservation_bounds_actual_allocations_and_one_byte_short_cannot_stage() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let required = store.required_reservation(&verified, &archive).unwrap();
    assert!(required >= archive.len() as u64);
    assert!(
        required < 1024 * 1024,
        "Tiny fixture must not be charged preferred APFS I/O sizes"
    );
    let exact = StageBudget {
        reserved_bytes: required,
        max_stage_bytes: required,
        host_reserve_bytes: 1024 * 1024,
    };
    let mut insufficient = exact;
    insufficient.reserved_bytes -= 1;
    assert!(matches!(
        store.stage(&verified, &archive, insufficient),
        Err(ReleaseStoreError::ReservationExceeded)
    ));
    assert!(names(&directory.0).is_empty());
    let release = store.stage(&verified, &archive, exact).unwrap();
    fn allocated(path: &Path) -> u64 {
        let metadata = fs::symlink_metadata(path).unwrap();
        let descendants = if metadata.is_dir() {
            fs::read_dir(path)
                .unwrap()
                .map(|entry| allocated(&entry.unwrap().path()))
                .sum()
        } else {
            0
        };
        metadata.blocks() * 512 + descendants
    }
    assert!(
        allocated(&release.path) <= required,
        "Admission estimate must cover the real fixture's allocated blocks"
    );
}

#[test]
fn every_write_checkpoint_failure_cleans_its_stage_without_a_release() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    // Created, three persisted files including the archive, then BeforePublish.
    for fail_at in 0..5 {
        let mut index = 0;
        let result = store.stage_with_checkpoint(&verified, &archive, budget(), |_| {
            let current = index;
            index += 1;
            if current == fail_at {
                Err(std::io::Error::from(std::io::ErrorKind::Interrupted).into())
            } else {
                Ok(())
            }
        });
        assert_eq!(index, fail_at + 1, "checkpoint {fail_at} must be reached");
        assert!(
            matches!(result, Err(ReleaseStoreError::Io(error)) if error.kind() == std::io::ErrorKind::Interrupted)
        );
        assert!(
            names(&directory.0).is_empty(),
            "checkpoint {fail_at} leaked stage data"
        );
    }
}

#[test]
fn modified_and_extra_existing_content_is_never_reused_or_overwritten() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let staged = store.stage(&verified, &archive, budget()).unwrap();
    let file = staged.path.join("payload/runtime/main.js");
    permissions(&file, 0o600);
    fs::write(&file, b"modified package code").unwrap();
    permissions(&file, 0o400);
    assert!(matches!(
        store.stage(&verified, &archive, budget()),
        Err(ReleaseStoreError::InvalidExisting)
    ));
    assert_eq!(fs::read(&file).unwrap(), b"modified package code");
    permissions(&file, 0o600);
    fs::write(&file, verified.file("runtime/main.js").unwrap()).unwrap();
    permissions(&file, 0o400);
    permissions(&staged.path, 0o700);
    fs::write(staged.path.join("unreviewed.js"), b"extra").unwrap();
    permissions(&staged.path, 0o500);
    assert!(matches!(
        store.stage(&verified, &archive, budget()),
        Err(ReleaseStoreError::InvalidExisting)
    ));
}

#[test]
fn root_and_ancestor_symlinks_and_shared_root_permissions_are_rejected() {
    let directory = Directory::new();
    let actual = directory.child("actual");
    let link = directory.0.join("link");
    symlink(&actual, &link).unwrap();
    assert!(ReleaseStore::open(&link).is_err());
    fs::create_dir(actual.join("nested")).unwrap();
    permissions(&actual.join("nested"), 0o700);
    assert!(ReleaseStore::open(&link.join("nested")).is_err());
    permissions(&actual, 0o755);
    assert!(matches!(
        ReleaseStore::open(&actual),
        Err(ReleaseStoreError::InvalidRoot)
    ));
}

#[test]
fn hostile_staged_symlink_never_reaches_its_target_and_is_cleaned_on_failure() {
    let directory = Directory::new();
    let releases = directory.child("releases");
    let victim = directory.0.join("keep.txt");
    fs::write(&victim, b"keep unchanged").unwrap();
    let store = ReleaseStore::open(&releases).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let result = store.stage_with_checkpoint(&verified, &archive, budget(), |checkpoint| {
        if checkpoint == StageCheckpoint::BeforePublish {
            let parent = stage_path(&releases).join("payload/runtime");
            permissions(&parent, 0o700);
            fs::remove_file(parent.join("main.js")).unwrap();
            symlink(&victim, parent.join("main.js")).unwrap();
            permissions(&parent, 0o500);
        }
        Ok(())
    });
    assert!(matches!(result, Err(ReleaseStoreError::UnsafeEntry)));
    assert!(names(&releases).is_empty());
    assert_eq!(fs::read(victim).unwrap(), b"keep unchanged");
}

#[test]
fn root_rename_cannot_redirect_writes_to_a_replacement_symlink() {
    let directory = Directory::new();
    let releases = directory.child("releases");
    let unrelated = directory.child("unrelated");
    let moved = directory.0.join("moved");
    let store = ReleaseStore::open(&releases).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let staged = store
        .stage_with_checkpoint(&verified, &archive, budget(), |checkpoint| {
            if checkpoint == StageCheckpoint::Created {
                fs::rename(&releases, &moved).unwrap();
                symlink(&unrelated, &releases).unwrap();
            }
            Ok(())
        })
        .unwrap();
    assert!(staged.directory.metadata().unwrap().is_dir());
    assert!(names(&unrelated).is_empty());
    assert_eq!(names(&moved).len(), 1);
}

#[test]
fn simultaneous_same_digest_stages_publish_once_and_validate_the_winner() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let barrier = Barrier::new(2);
    std::thread::scope(|scope| {
        let run = || {
            store.stage_with_checkpoint(&verified, &archive, budget(), |checkpoint| {
                if checkpoint == StageCheckpoint::BeforePublish {
                    barrier.wait();
                }
                Ok(())
            })
        };
        let one = scope.spawn(run);
        let two = scope.spawn(run);
        let results = [one.join().unwrap(), two.join().unwrap()];
        let completed = results.map(|result| match result {
            Err(ReleaseStoreError::Busy) => store.stage(&verified, &archive, budget()).unwrap(),
            other => other.unwrap(),
        });
        assert_ne!(completed[0].reused, completed[1].reused);
    });
    assert_eq!(names(&directory.0).len(), 1);
}

#[test]
fn competing_empty_digest_directory_is_not_overwritten_during_publication() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let name = verified.package_digest().strip_prefix("sha256:").unwrap();
    let result = store.stage_with_checkpoint(&verified, &archive, budget(), |checkpoint| {
        if checkpoint == StageCheckpoint::BeforePublish {
            fs::create_dir(directory.0.join(name)).unwrap();
            permissions(&directory.0.join(name), 0o500);
        }
        Ok(())
    });
    assert!(matches!(result, Err(ReleaseStoreError::InvalidExisting)));
    assert!(names(&directory.0.join(name)).is_empty());
    assert_eq!(names(&directory.0), [name]);
}

#[test]
fn an_external_hardlink_invalidates_reuse_of_an_otherwise_matching_release() {
    let directory = Directory::new();
    let releases = directory.child("releases");
    let store = ReleaseStore::open(&releases).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let staged = store.stage(&verified, &archive, budget()).unwrap();
    let external = directory.0.join("external-link");
    fs::hard_link(staged.path.join("payload/runtime/main.js"), &external).unwrap();
    assert!(matches!(
        store.stage(&verified, &archive, budget()),
        Err(ReleaseStoreError::UnsafeEntry)
    ));
    assert_eq!(
        fs::read(external).unwrap(),
        verified.file("runtime/main.js").unwrap()
    );
}

#[test]
fn recovery_skips_live_stages_and_removes_only_recognized_abandoned_trees() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let staged = store
        .stage_with_checkpoint(&verified, &archive, budget(), |checkpoint| {
            if checkpoint == StageCheckpoint::Created {
                let report = store.collect_abandoned().unwrap();
                assert_eq!(report.active, 1);
                assert_eq!(report.removed, 0);
            }
            Ok(())
        })
        .unwrap();
    let abandoned = directory.child(".stage-1-2-abcd");
    fs::write(
        abandoned.join(".chariox-stage"),
        format!(
            "chariox.app.release-stage.v1\n{}\n",
            verified.package_digest()
        ),
    )
    .unwrap();
    fs::write(abandoned.join("partial"), b"partial package").unwrap();
    let unrelated = directory.child(".stage-3-4-abcd");
    fs::write(unrelated.join("preserve"), b"unrecognized directory").unwrap();
    let report = store.collect_abandoned().unwrap();
    assert_eq!(report.removed, 1);
    assert_eq!(report.unrecognized, 1);
    assert!(!abandoned.exists());
    assert!(unrelated.join("preserve").exists());
    assert!(staged.path.join("envelope.cxapp").exists());
}

#[test]
fn publication_keeps_root_renameable_and_excludes_reuse_until_sealed_and_synced() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let final_path = directory
        .0
        .join(verified.package_digest().strip_prefix("sha256:").unwrap());
    let mut published = false;
    let mut parent_synced = false;
    let staged = store
        .stage_with_checkpoint(&verified, &archive, budget(), |point| {
            match point {
                StageCheckpoint::BeforePublish => {
                    let stage = stage_path(&directory.0);
                    assert_eq!(fs::metadata(&stage).unwrap().mode() & 0o777, 0o700);
                    assert_eq!(
                        fs::metadata(stage.join("payload")).unwrap().mode() & 0o777,
                        0o500
                    );
                }
                StageCheckpoint::Published => {
                    published = true;
                    assert_eq!(fs::metadata(&final_path).unwrap().mode() & 0o777, 0o700);
                    assert!(matches!(
                        store.stage(&verified, &archive, budget()),
                        Err(ReleaseStoreError::Busy)
                    ));
                }
                StageCheckpoint::BeforeParentSync => {
                    assert_eq!(fs::metadata(&final_path).unwrap().mode() & 0o777, 0o500);
                    assert!(matches!(
                        store.stage(&verified, &archive, budget()),
                        Err(ReleaseStoreError::Busy)
                    ));
                }
                StageCheckpoint::ParentSynced => parent_synced = true,
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert!(published && parent_synced);
    assert_eq!(staged.directory.metadata().unwrap().mode() & 0o777, 0o500);
    // Returning a consumer descriptor must not retain the publication flock.
    assert!(store.stage(&verified, &archive, budget()).unwrap().reused);
}

#[test]
fn interrupted_rename_is_verified_repaired_and_parent_synced_before_reuse_ack() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let final_path = directory
        .0
        .join(verified.package_digest().strip_prefix("sha256:").unwrap());
    assert!(store
        .stage_with_checkpoint(&verified, &archive, budget(), |point| {
            if point == StageCheckpoint::Published {
                return Err(std::io::Error::other("interrupted after rename").into());
            }
            Ok(())
        })
        .is_err());
    assert_eq!(fs::metadata(&final_path).unwrap().mode() & 0o777, 0o700);
    drop(store);
    let store = ReleaseStore::open(&directory.0).unwrap();
    // Refuse the sync boundary: even a matching reused tree cannot acknowledge.
    assert!(store
        .stage_with_checkpoint(&verified, &archive, budget(), |point| {
            if point == StageCheckpoint::BeforeParentSync {
                return Err(std::io::Error::other("publication sync unavailable").into());
            }
            Ok(())
        })
        .is_err());
    assert_eq!(fs::metadata(&final_path).unwrap().mode() & 0o777, 0o500);
    let mut synced = false;
    let reused = store
        .stage_with_checkpoint(&verified, &archive, budget(), |point| {
            if point == StageCheckpoint::ParentSynced {
                synced = true;
            }
            Ok(())
        })
        .unwrap();
    assert!(reused.reused && synced);
    assert_eq!(
        fs::read(reused.path.join("envelope.cxapp")).unwrap(),
        archive
    );
}

#[test]
fn repair_rejects_tampered_unsealed_published_tree_without_acknowledging_it() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let final_path = directory
        .0
        .join(verified.package_digest().strip_prefix("sha256:").unwrap());
    assert!(store
        .stage_with_checkpoint(&verified, &archive, budget(), |point| {
            if point == StageCheckpoint::Published {
                return Err(std::io::Error::other("interruption").into());
            }
            Ok(())
        })
        .is_err());
    let file = final_path.join("envelope.cxapp");
    permissions(&file, 0o600);
    fs::write(&file, b"unverified replacement").unwrap();
    permissions(&file, 0o400);
    assert!(matches!(
        store.stage(&verified, &archive, budget()),
        Err(ReleaseStoreError::InvalidExisting)
    ));
    assert_eq!(fs::metadata(&final_path).unwrap().mode() & 0o777, 0o700);
}

#[test]
fn interrupted_cleanup_preserves_marker_until_all_payload_deletion_is_durable() {
    let directory = Directory::new();
    let store = ReleaseStore::open(&directory.0).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let abandoned = directory.child(".stage-7-8-abcd");
    let marker = abandoned.join(".chariox-stage");
    fs::write(
        &marker,
        format!(
            "chariox.app.release-stage.v1\n{}\n",
            verified.package_digest()
        ),
    )
    .unwrap();
    let payload = abandoned.join("payload");
    fs::create_dir(&payload).unwrap();
    permissions(&payload, 0o700);
    fs::write(payload.join("first"), b"partial package").unwrap();
    fs::write(payload.join(".chariox-stage"), b"ordinary nested filename").unwrap();
    permissions(&payload, 0o500);
    permissions(&abandoned, 0o500);
    assert!(store
        .collect_abandoned_with_checkpoint(|point| {
            if point == CleanupCheckpoint::PayloadEntryRemoved {
                return Err(std::io::Error::other("cleanup interrupted"));
            }
            Ok(())
        })
        .is_err());
    assert!(
        marker.exists(),
        "the recovery marker must outlive partial payload deletion"
    );
    drop(store);
    let store = ReleaseStore::open(&directory.0).unwrap();
    assert!(store
        .collect_abandoned_with_checkpoint(|point| {
            if point == CleanupCheckpoint::BeforeMarkerRemoval {
                assert_eq!(names(&abandoned), [".chariox-stage"]);
                return Err(std::io::Error::other(
                    "interrupted before final marker removal",
                ));
            }
            Ok(())
        })
        .is_err());
    assert!(marker.exists());
    assert_eq!(store.collect_abandoned().unwrap().removed, 1);
    assert!(!abandoned.exists());
}

#[test]
fn database_root_creation_is_private_durable_reusable_and_separates_kernels() {
    let directory = Directory::new();
    let first = ReleaseStore::open_or_create(&directory.0.join("first.db")).unwrap();
    let second = ReleaseStore::open_or_create(&directory.0.join("second.db")).unwrap();
    let (archive, policy) = package();
    let verified = verify(&archive, &policy).unwrap();
    let first_release = first.stage(&verified, &archive, budget()).unwrap();
    let second_release = second.stage(&verified, &archive, budget()).unwrap();
    assert_ne!(first_release.path, second_release.path);
    for release in [&first_release, &second_release] {
        assert_eq!(
            fs::metadata(release.path.parent().unwrap()).unwrap().mode() & 0o777,
            0o700
        );
    }
    drop(first);
    let reopened = ReleaseStore::open_or_create(&directory.0.join("first.db")).unwrap();
    let recovered = reopened.stage(&verified, &archive, budget()).unwrap();
    assert!(recovered.reused);
    assert_eq!(recovered.path, first_release.path);
    assert_eq!(
        names(&directory.0).len(),
        2,
        "Only fixed release leaves are created"
    );
}

#[test]
fn database_release_constructor_rejects_symlinks_and_unsafe_parents_without_chmod() {
    let directory = Directory::new();
    assert!(ReleaseStore::open_or_create(Path::new("relative.db")).is_err());
    let original = directory.child("original");
    let alias = directory.0.join("alias");
    symlink(&original, &alias).unwrap();
    assert!(ReleaseStore::open_or_create(&alias.join("state.db")).is_err());
    let database = original.join("state.db");
    ReleaseStore::open_or_create(&database).unwrap();
    let release_leaf = original.join(names(&original).pop().unwrap());
    fs::remove_dir(&release_leaf).unwrap();
    symlink(&directory.0, &release_leaf).unwrap();
    assert!(ReleaseStore::open_or_create(&database).is_err());
    assert!(fs::symlink_metadata(&release_leaf)
        .unwrap()
        .file_type()
        .is_symlink());
    let shared = directory.child("shared");
    permissions(&shared, 0o777);
    assert!(ReleaseStore::open_or_create(&shared.join("state.db")).is_err());
    assert_eq!(fs::metadata(&shared).unwrap().mode() & 0o777, 0o777);
    assert!(names(&shared).is_empty());
}
