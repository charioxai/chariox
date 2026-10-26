use super::*;
use crate::worker_process::ResourceDomain;
use std::os::unix::fs::PermissionsExt;

struct Domain(File);
impl ResourceDomain for Domain {
    fn private_data_directory(&self) -> Result<File, WorkerError> {
        self.0.try_clone().map_err(|_| WorkerError::Io)
    }
    fn verify_before_continue(&mut self, _: libc::pid_t) -> Result<(), WorkerError> {
        Ok(())
    }
    fn terminate(&mut self, _: libc::pid_t) {}
    fn reap_domain_blocking(&mut self) {}
}
struct Fixture {
    path: PathBuf,
    root: Dir,
    data: Dir,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "wasm-snapshot-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = Dir::open_private(&path).unwrap();
        let data = root.create_child(OsStr::new("data")).unwrap();
        root.create_child(OsStr::new("tmp")).unwrap();
        std::fs::write(path.join("data/value"), b"committed").unwrap();
        Self { path, root, data }
    }
    fn start(&self, generation: u64, committed: u64) -> Result<(), WorkerError> {
        let prepared = PreparedWorker {
            program: CString::new("/not-executed").unwrap(),
            arguments: vec![],
            record: PreparedLaunch::Wasm {
                generation: generation.to_string(),
                installation: "installed".into(),
                release_digest: format!("sha256:{}", "a".repeat(64)),
            },
            _objects: vec![],
            domain: Box::new(Domain(self.data.0.try_clone().unwrap())),
        };
        prepared
            .visit_private_data(|data| prepare(&self.root, &self.path, data, Some(committed)))
            .unwrap()
            .1
    }
    fn value(&self) -> Vec<u8> {
        std::fs::read(self.path.join("data/value")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}

#[test]
fn retries_and_failed_updates_restore_private_data_but_never_tmp() {
    let f = Fixture::new();
    f.start(2, 1).unwrap();
    std::fs::write(f.path.join("data/value"), b"uncommitted").unwrap();
    std::fs::write(f.path.join("tmp/scratch"), b"not checkpointed").unwrap();
    f.start(2, 1).unwrap();
    assert_eq!(f.value(), b"committed");
    assert!(f.path.join("tmp/scratch").exists());
    std::fs::write(f.path.join("data/value"), b"partial restore").unwrap();
    f.start(1, 1).unwrap();
    assert_eq!(f.value(), b"committed");
    assert!(!f.path.join(".migration").exists());
}

#[test]
fn a_committed_update_retires_the_old_checkpoint_and_snapshots_its_own_files() {
    let f = Fixture::new();
    f.start(2, 1).unwrap();
    std::fs::write(f.path.join("data/value"), b"new committed").unwrap();
    f.start(2, 2).unwrap();
    assert_eq!(f.value(), b"new committed");
    assert!(!f.path.join(".migration").exists());
    f.start(3, 2).unwrap();
    std::fs::write(f.path.join("data/value"), b"failed third").unwrap();
    f.start(2, 2).unwrap();
    assert_eq!(f.value(), b"new committed");
}

#[test]
fn an_incomplete_checkpoint_is_removed_before_a_retry_and_never_restored() {
    let f = Fixture::new();
    f.root.create_child(OsStr::new(".pending")).unwrap();
    f.start(2, 1).unwrap();
    assert_eq!(f.value(), b"committed");
    assert!(!f.path.join(".pending").exists());
    assert!(f.path.join(".migration/generation").exists());
}

#[test]
fn a_corrupt_checkpoint_cannot_clear_current_data() {
    let f = Fixture::new();
    f.start(2, 1).unwrap();
    std::fs::remove_file(f.path.join(".migration/data/value")).unwrap();
    std::os::unix::fs::symlink("/outside", f.path.join(".migration/data/value")).unwrap();
    assert!(f.start(1, 1).is_err());
    assert_eq!(f.value(), b"committed");
    assert!(f.path.join(".migration").exists());
}
