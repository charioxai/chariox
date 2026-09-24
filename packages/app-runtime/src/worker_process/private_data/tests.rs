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
struct Domain(Arc<AtomicBool>);
impl ResourceDomain for Domain {
    fn verify_before_continue(&mut self, _: libc::pid_t) -> std::result::Result<(), WorkerError> {
        Ok(())
    }
    fn terminate(&mut self, _: libc::pid_t) {}
    fn reap_domain_blocking(&mut self) {}
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
            domain: Box::new(Domain(dropped.clone())),
        };
        let data = PrivateData {
            root: Arc::new(Dir(File::open(&path).unwrap())),
            preparation: Arc::new(Mutex::new(prepared)),
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
