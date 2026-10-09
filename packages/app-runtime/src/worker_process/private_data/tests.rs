use super::*;
use crate::worker_process::{record::LaunchRecord, ResourceDomain};
use std::{
    ffi::CString,
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

struct Fixture {
    path: PathBuf,
    data: Option<PrivateData>,
    dropped: Arc<AtomicBool>,
}
struct Domain(Arc<AtomicBool>, File);
impl ResourceDomain for Domain {
    fn verify_before_continue(&mut self, _: libc::pid_t) -> std::result::Result<(), WorkerError> {
        Ok(())
    }
    fn terminate(&mut self, _: libc::pid_t) {}
    fn reap_domain_blocking(&mut self) {}
    fn private_data_directory(&self) -> std::result::Result<File, WorkerError> {
        self.1.try_clone().map_err(|_| WorkerError::Preparation)
    }
}
impl Drop for Domain {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl Fixture {
    fn new() -> Self {
        let mut template = CString::new("/tmp/chariox-private-data-XXXXXX")
            .unwrap()
            .into_bytes_with_nul();
        assert!(!unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) }.is_null());
        let path = fs::canonicalize(std::str::from_utf8(&template[..template.len() - 1]).unwrap())
            .unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let prepared = PreparedWorker {
            program: CString::new("/not-executed").unwrap(),
            arguments: vec![],
            record: LaunchRecord {
                generation: "1".into(),
                installation: "installed".into(),
                release_digest: "a".repeat(64),
                roots: [
                    "/package".into(),
                    "/data".into(),
                    "/tmp".into(),
                    "/runtime".into(),
                ],
                bootstrap: "".into(),
                nofile: 128,
                cpu_seconds: 30,
                heap_mib: 64,
                v8_threads: 1,
                max_file_bytes: 1048576,
            },
            _objects: vec![],
            domain: Box::new(Domain(dropped.clone(), File::open(&path).unwrap())),
        };
        let data = PrivateData {
            root: Arc::new(Dir(File::open(&path).unwrap())),
            _preparation: Arc::new(Mutex::new(prepared)),
            installation: "installed".into(),
            generation: 1,
            release_digest: "a".repeat(64),
        };
        Self {
            path,
            data: Some(data),
            dropped,
        }
    }
    fn data(&self) -> &PrivateData {
        self.data.as_ref().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.data.take();
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn staging_keeps_previous_file_visible_until_atomic_publication_and_sync() {
    let f = Fixture::new();
    fs::create_dir(f.path.join("nested")).unwrap();
    fs::write(f.path.join("nested/value"), b"old").unwrap();
    let staged = f
        .data()
        .prepare_replace("nested/value", b"new bytes")
        .unwrap();
    assert_eq!(fs::read(f.path.join("nested/value")).unwrap(), b"old");
    staged.publish().unwrap();
    assert_eq!(fs::read(f.path.join("nested/value")).unwrap(), b"new bytes");
    assert_eq!(
        fs::metadata(f.path.join("nested/value"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(fs::read_dir(f.path.join("nested")).unwrap().count(), 1);
    f.data()
        .prepare_replace("empty", b"")
        .unwrap()
        .publish()
        .unwrap();
    assert_eq!(fs::metadata(f.path.join("empty")).unwrap().len(), 0);
}

#[test]
fn paths_symlink_parents_and_special_destinations_cannot_redirect_a_write() {
    let f = Fixture::new();
    let outside = Fixture::new();
    fs::write(outside.path.join("valuable"), b"unchanged").unwrap();
    symlink(&outside.path, f.path.join("alias")).unwrap();
    symlink(outside.path.join("valuable"), f.path.join("link")).unwrap();
    fs::create_dir(f.path.join("directory")).unwrap();
    for path in [
        "",
        ".",
        "..",
        "../valuable",
        "/valuable",
        "nested/../valuable",
        "a//b",
        "a/./b",
        "C:/x",
        "a\\b",
        "a\0b",
        "alias/valuable",
        "link",
        "directory",
    ] {
        assert!(f.data().prepare_replace(path, b"bad").is_err(), "{path:?}");
    }
    assert!(f
        .data()
        .prepare_replace("large", &vec![0; MAX_REPLACE_BYTES + 1])
        .is_err());
    assert_eq!(
        fs::read(outside.path.join("valuable")).unwrap(),
        b"unchanged"
    );
}

#[test]
fn cancelled_stage_cleans_its_inode_and_retains_actual_preparation_until_drop() {
    let mut f = Fixture::new();
    let staged = f.data().prepare_replace("value", b"new").unwrap();
    f.data.take();
    assert!(!f.dropped.load(Ordering::SeqCst));
    assert_eq!(fs::read_dir(&f.path).unwrap().count(), 1);
    drop(staged);
    assert!(f.dropped.load(Ordering::SeqCst));
    assert_eq!(fs::read_dir(&f.path).unwrap().count(), 0);
}

#[test]
fn substituted_temporary_entry_is_rejected_and_cleanup_does_not_follow_it() {
    let f = Fixture::new();
    let outside = Fixture::new();
    fs::write(outside.path.join("valuable"), b"safe").unwrap();
    let staged = f.data().prepare_replace("value", b"new").unwrap();
    let temporary = f.path.join(&staged.temporary);
    fs::remove_file(&temporary).unwrap();
    symlink(outside.path.join("valuable"), &temporary).unwrap();
    assert_eq!(staged.publish(), Err(PrivateDataError::Identity));
    assert!(fs::symlink_metadata(temporary)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(!f.path.join("value").exists());
    assert_eq!(fs::read(outside.path.join("valuable")).unwrap(), b"safe");
}

#[test]
fn failure_after_rename_reports_uncertainty_and_preserves_published_file() {
    let f = Fixture::new();
    fs::write(f.path.join("value"), b"old").unwrap();
    let staged = f.data().prepare_replace("value", b"new").unwrap();
    assert_eq!(
        staged
            .publish_checked(|| Err(std::io::Error::other("lost directory-sync acknowledgement"))),
        Err(PrivateDataError::OutcomeUncertain)
    );
    assert_eq!(fs::read(f.path.join("value")).unwrap(), b"new");
    assert_eq!(fs::read_dir(&f.path).unwrap().count(), 1);
}

#[test]
fn reads_follow_neither_symlinks_nor_hard_links_and_stay_bounded() {
    let f = Fixture::new();
    let outside = Fixture::new();
    fs::write(outside.path.join("valuable"), b"secret").unwrap();
    symlink(&outside.path, f.path.join("alias")).unwrap();
    symlink(outside.path.join("valuable"), f.path.join("link")).unwrap();
    fs::hard_link(outside.path.join("valuable"), f.path.join("hard")).unwrap();
    fs::create_dir(f.path.join("nested")).unwrap();
    fs::write(f.path.join("nested/notes.md"), b"# Notes").unwrap();
    assert_eq!(
        f.data().read_file("nested/notes.md", 16).unwrap(),
        b"# Notes"
    );
    assert_eq!(
        f.data().read_file("nested/notes.md", 3),
        Err(PrivateDataError::Invalid)
    );
    for path in [
        "alias/valuable",
        "link",
        "hard",
        "nested",
        "../valuable",
        "missing",
    ] {
        assert!(f.data().read_file(path, 64).is_err(), "{path:?}");
    }
}

#[test]
fn raw_app_writes_racing_an_sdk_replacement_never_tear_it() {
    // V1-INT-04: the App writes the same file with node:fs while the SDK
    // replaces it. Raw writes keep ordinary semantics, but the SDK never
    // writes into an existing inode: the one it replaces never receives its
    // bytes. Every replacement publishes, and the file ends as one writer's
    // complete contents with no staging left behind.
    use std::os::unix::fs::FileExt;
    const LEN: usize = 256 * 1024;
    const ROUNDS: usize = 200;
    let f = Fixture::new();
    let path = f.path.join("doc");
    fs::write(&path, vec![b'a'; LEN]).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let writer = std::thread::spawn({
        let (path, done) = (path.clone(), done.clone());
        move || {
            while !done.load(Ordering::SeqCst) {
                fs::write(&path, vec![b'a'; LEN]).unwrap();
            }
        }
    });
    for _ in 0..ROUNDS {
        // Only replacements create inodes, and the racing writer writes only
        // `a`, so the inode about to be replaced holds no `b` from here on.
        let replaced = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        replaced.write_all_at(&vec![b'a'; LEN], 0).unwrap();
        f.data()
            .prepare_replace("doc", &vec![b'b'; LEN])
            .unwrap()
            .publish()
            .unwrap();
        let mut held = vec![0; LEN];
        let read = replaced.read_at(&mut held, 0).unwrap();
        assert!(
            held[..read].iter().all(|b| *b == b'a'),
            "the replacement wrote into the inode it replaced"
        );
    }
    done.store(true, Ordering::SeqCst);
    writer.join().unwrap();
    let last = fs::read(&path).unwrap();
    assert_eq!(last.len(), LEN);
    assert!(last.iter().all(|b| *b == b'a') || last.iter().all(|b| *b == b'b'));
    assert_eq!(fs::read_dir(&f.path).unwrap().count(), 1);
}

#[test]
fn space_refusals_are_storage_full_and_other_failures_keep_their_kind() {
    // ENOSPC is a full fixed-size volume (Linux ext4, macOS APFS); EDQUOT a
    // filesystem quota. Both are the App's quota, never a generic I/O failure.
    for errno in [libc::ENOSPC, libc::EDQUOT] {
        let error = || std::io::Error::from_raw_os_error(errno);
        assert_eq!(io(error()), PrivateDataError::StorageFull);
        assert_eq!(
            fs(private_fs::FsError::Io(error())),
            PrivateDataError::StorageFull
        );
    }
    for errno in [libc::EIO, libc::EROFS, libc::EFBIG, libc::ENOENT] {
        assert_eq!(
            io(std::io::Error::from_raw_os_error(errno)),
            PrivateDataError::Io
        );
    }
    assert_eq!(io(std::io::Error::other("no errno")), PrivateDataError::Io);
    assert_eq!(
        fs(private_fs::FsError::UnsafeEntry),
        PrivateDataError::Identity
    );
}

// V-SDK-02: hostile paths from App code. The App owns its data directory and
// can rename, delete and link inside it at any time; every SDK write must stay
// inside the installation root.

#[test]
fn a_parent_swapped_for_an_outside_symlink_mid_write_never_escapes() {
    let f = Fixture::new();
    let outside = Fixture::new();
    fs::create_dir(f.path.join("nested")).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let swapper = std::thread::spawn({
        let (root, target, done) = (f.path.clone(), outside.path.clone(), done.clone());
        move || {
            while !done.load(Ordering::SeqCst) {
                let _ = fs::rename(root.join("nested"), root.join("nested.real"));
                let _ = symlink(&target, root.join("nested"));
                let _ = fs::remove_file(root.join("nested"));
                let _ = fs::rename(root.join("nested.real"), root.join("nested"));
            }
        }
    });
    let mut published = 0;
    for _ in 0..500 {
        if let Ok(staged) = f.data().prepare_replace("nested/value", b"inside") {
            if staged.publish().is_ok() {
                published += 1;
            }
        }
    }
    done.store(true, Ordering::SeqCst);
    swapper.join().unwrap();
    assert!(published > 0, "some writes found the real directory");
    assert_eq!(
        fs::read_dir(&outside.path).unwrap().count(),
        0,
        "nothing landed outside"
    );
    // The swapper always puts the real directory back before it stops.
    assert_eq!(fs::read(f.path.join("nested/value")).unwrap(), b"inside");
}

#[test]
fn unicode_and_case_aliases_of_an_outside_symlink_are_refused() {
    let f = Fixture::new();
    let outside = Fixture::new();
    fs::write(outside.path.join("secret"), b"unchanged").unwrap();
    // NFD "café" and "Link" point outside; the App writes NFC "café" and
    // "link". A normalization- or case-insensitive filesystem (APFS) resolves
    // them to the symlinks, which must be refused; elsewhere they are new
    // files inside the root.
    symlink(outside.path.join("secret"), f.path.join("cafe\u{301}")).unwrap();
    symlink(outside.path.join("secret"), f.path.join("Link")).unwrap();
    for name in ["caf\u{e9}", "link"] {
        let aliased =
            fs::symlink_metadata(f.path.join(name)).is_ok_and(|m| m.file_type().is_symlink());
        let written = f
            .data()
            .prepare_replace(name, b"overwrite")
            .and_then(|staged| staged.publish());
        // APFS is normalization-insensitive even on case-sensitive volumes.
        #[cfg(target_os = "macos")]
        if name == "caf\u{e9}" {
            assert!(aliased, "NFC café must resolve to the NFD symlink on APFS");
        }
        if aliased {
            assert!(
                written.is_err(),
                "{name:?} resolves to a symlink and must be refused"
            );
            assert!(f.data().read_file(name, 64).is_err());
        } else {
            assert_eq!(written, Ok(()));
            assert_eq!(f.data().read_file(name, 64).unwrap(), b"overwrite");
        }
    }
    assert_eq!(fs::read(outside.path.join("secret")).unwrap(), b"unchanged");
}

#[test]
fn a_deleted_or_replaced_parent_keeps_the_write_inside_the_root() {
    let f = Fixture::new();
    let outside = Fixture::new();
    // Deleted parent: the staged file went with it, so publication fails and
    // nothing is recreated.
    fs::create_dir(f.path.join("gone")).unwrap();
    let staged = f.data().prepare_replace("gone/value", b"lost").unwrap();
    fs::remove_dir_all(f.path.join("gone")).unwrap();
    assert!(staged.publish().is_err());
    assert!(!f.path.join("gone").exists());
    // Renamed parent replaced by an outside symlink: publication stays in the
    // held directory, now at its new name, and never follows the symlink.
    fs::create_dir(f.path.join("moved")).unwrap();
    let staged = f.data().prepare_replace("moved/value", b"held").unwrap();
    fs::rename(f.path.join("moved"), f.path.join("moved.old")).unwrap();
    symlink(&outside.path, f.path.join("moved")).unwrap();
    assert_eq!(staged.publish(), Ok(()));
    assert_eq!(
        fs::read_dir(&outside.path).unwrap().count(),
        0,
        "nothing landed outside"
    );
    assert_eq!(fs::read(f.path.join("moved.old/value")).unwrap(), b"held");
    // A later write through the symlinked parent is refused.
    assert!(f.data().prepare_replace("moved/value", b"x").is_err());
}

#[test]
fn pre_spawn_storage_visit_keeps_the_domain_pinned_and_refuses_a_retained_descriptor() {
    let mut f = Fixture::new();
    let data = f.data.take().unwrap();
    let preparation = data._preparation.clone();
    drop(data);
    let prepared = Arc::try_unwrap(preparation)
        .ok()
        .expect("sole owner")
        .into_inner()
        .ok()
        .expect("healthy preparation");
    let (prepared, bytes) = prepared
        .visit_private_data(|data| {
            assert_eq!(data.installation_id(), "installed");
            assert_eq!(data.generation(), 1);
            data.prepare_replace("before-start", b"pinned")
                .unwrap()
                .publish()
                .unwrap();
            data.read_file("before-start", 64).unwrap()
        })
        .unwrap();
    assert_eq!(bytes, b"pinned");
    assert_eq!(fs::read(f.path.join("before-start")).unwrap(), b"pinned");
    assert!(!f.dropped.load(Ordering::SeqCst));
    // Returning a descriptor would outlive preparation ownership. It must
    // refuse rather than start App code with storage held by another caller.
    assert!(matches!(
        prepared.visit_private_data(|data| data.clone()),
        Err(WorkerError::Preparation)
    ));
    assert!(f.dropped.load(Ordering::SeqCst));
}
