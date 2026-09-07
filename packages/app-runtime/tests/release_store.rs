#![cfg(any(target_os = "macos", target_os = "linux"))]

use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::release_store::{
    ReleaseStore, ReleaseStoreError, StageBudget, StageCheckpoint,
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
        "sdkVersion":"0.1.0", "appContractVersion":1, "minKernelProtocol":500,
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
            store
                .stage_with_checkpoint(&verified, &archive, budget(), |checkpoint| {
                    if checkpoint == StageCheckpoint::BeforePublish {
                        barrier.wait();
                    }
                    Ok(())
                })
                .unwrap()
                .reused
        };
        let one = scope.spawn(run);
        let two = scope.spawn(run);
        assert_ne!(one.join().unwrap(), two.join().unwrap());
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
