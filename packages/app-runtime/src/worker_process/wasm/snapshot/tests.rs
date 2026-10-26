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
        self.start_with_migration(generation, committed, true)
    }
    fn start_with_migration(
        &self,
        generation: u64,
        committed: u64,
        migrating: bool,
    ) -> Result<(), WorkerError> {
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
            .visit_private_data(|data| prepare(&self.root, data, Some(committed), migrating))
            .unwrap()
            .1
    }
    fn value(&self) -> Vec<u8> {
        std::fs::read(self.path.join("data/value")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        tree::clear(&self.root).unwrap();
        std::fs::remove_dir(&self.path).unwrap();
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
    let path = CString::new(
        f.path
            .join(".migration/data/value")
            .as_os_str()
            .as_encoded_bytes(),
    )
    .unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(f.start(1, 1).is_err());
    assert_eq!(f.value(), b"committed");
    assert!(f.path.join(".migration").exists());
}

#[test]
fn an_update_without_migrations_does_not_checkpoint_private_files() {
    let f = Fixture::new();
    std::os::unix::fs::symlink("value", f.path.join("data/link")).unwrap();
    f.start_with_migration(2, 1, false).unwrap();
    assert!(!f.path.join(".migration").exists());
}

#[test]
fn migration_rewinds_links_large_files_and_more_than_ten_thousand_entries() {
    use std::os::unix::{
        ffi::OsStringExt,
        fs::{symlink, MetadataExt},
    };
    let f = Fixture::new();
    let data = f.path.join("data");
    symlink("value", data.join("link")).unwrap();
    symlink("/outside-must-not-be-followed", data.join("outside")).unwrap();
    std::fs::hard_link(data.join("value"), data.join("hard")).unwrap();
    let name = std::ffi::OsString::from_vec(vec![b'n', b'\\', 0xff]);
    std::fs::write(data.join(&name), b"raw name").unwrap();
    std::fs::create_dir(data.join("empty")).unwrap();
    let large = File::create(data.join("large")).unwrap();
    large.set_len(513 * 1024 * 1024).unwrap();
    for n in 0..10_001 {
        File::create(data.join(format!("f{n}"))).unwrap();
    }
    f.start(2, 1).unwrap();
    std::fs::write(data.join("value"), b"changed through hard link").unwrap();
    std::fs::remove_file(data.join("link")).unwrap();
    std::fs::write(data.join("link"), b"changed type").unwrap();
    std::fs::remove_dir(data.join("empty")).unwrap();
    std::fs::write(data.join("large"), b"truncated").unwrap();
    std::fs::write(data.join(&name), b"changed").unwrap();
    f.start_with_migration(1, 1, false).unwrap();
    assert_eq!(f.value(), b"committed");
    assert_eq!(std::fs::read(data.join("hard")).unwrap(), b"committed");
    assert_ne!(
        std::fs::metadata(data.join("value")).unwrap().ino(),
        std::fs::metadata(data.join("hard")).unwrap().ino()
    );
    assert_eq!(
        std::fs::read_link(data.join("link")).unwrap(),
        Path::new("value")
    );
    assert_eq!(
        std::fs::read_link(data.join("outside")).unwrap(),
        Path::new("/outside-must-not-be-followed")
    );
    assert_eq!(std::fs::read(data.join(&name)).unwrap(), b"raw name");
    assert!(data.join("empty").is_dir());
    assert_eq!(
        std::fs::metadata(data.join("large")).unwrap().len(),
        513 * 1024 * 1024
    );
    assert_eq!(std::fs::read_dir(&data).unwrap().count(), 10_008);
    assert!(!f.path.join(".migration").exists());
}

#[test]
fn deep_trees_rewind_and_retire_without_retaining_ancestor_descriptors() {
    let f = Fixture::new();
    let mut leaf = Dir(f.data.0.try_clone().unwrap());
    for _ in 0..1100 {
        leaf = leaf.create_child(OsStr::new("nested")).unwrap();
    }
    leaf.write_new(OsStr::new("leaf"), b"committed").unwrap();
    f.start(2, 1).unwrap();
    leaf.remove_file(OsStr::new("leaf")).unwrap();
    leaf.write_new(OsStr::new("leaf"), b"failed").unwrap();
    drop(leaf);
    f.start_with_migration(1, 1, false).unwrap();
    let mut leaf = Dir(f.data.0.try_clone().unwrap());
    for _ in 0..1100 {
        leaf = leaf.child(OsStr::new("nested")).unwrap();
    }
    let mut contents = String::new();
    leaf.read_file(OsStr::new("leaf"), false)
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(contents, "committed");
    assert!(!f.path.join(".migration").exists());
}
