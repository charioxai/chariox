use super::*;
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture {
    root: PathBuf,
    owner: Owner,
    digest: String,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".chariox/dev");
        fs::create_dir_all(&parent).unwrap();
        let root = parent.join(format!(
            "app-code-source-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        mode(&root, 0o700);
        let owner = Owner {
            uid: unsafe { libc::geteuid() },
            gid: unsafe { libc::getegid() },
            cgroup_root: "/sys/fs/cgroup/fixed".into(),
            kernel_database_paths: vec![root.join("kernel.db")],
        };
        let fixture = Self {
            root,
            owner,
            digest: format!("sha256:{:x}", Sha256::digest(b"mechanical archive fixture")),
        };
        fixture.publish(&fixture.owner.kernel_database_paths[0]);
        fixture
    }
    fn publish(&self, database: &Path) -> PathBuf {
        let store = crate::release_store::root_for_database(database).unwrap();
        fs::create_dir(&store).unwrap();
        mode(&store, 0o700);
        let root = store.join(&self.digest[7..]);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("envelope.cxapp"), b"mechanical archive fixture").unwrap();
        mode(&root.join("envelope.cxapp"), 0o400);
        fs::create_dir(root.join("payload")).unwrap();
        fs::write(root.join("payload/main.js"), b"fixed").unwrap();
        mode(&root.join("payload/main.js"), 0o400);
        mode(&root.join("payload"), 0o500);
        mode(&root, 0o500);
        root
    }
    fn path(&self) -> PathBuf {
        crate::release_store::root_for_database(&self.owner.kernel_database_paths[0])
            .unwrap()
            .join(&self.digest[7..])
    }
}
fn mode(path: &Path, value: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(value)).unwrap();
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fn restore(path: &Path) {
            if fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()) {
                mode(path, 0o700);
                for entry in fs::read_dir(path).unwrap() {
                    restore(&entry.unwrap().path())
                }
            }
        }
        restore(&self.root);
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn enrolled_source_retains_exact_readonly_payload_and_shared_release_lock() {
    let fixture = Fixture::new();
    let (root, payload) = release(&fixture.owner, &fixture.digest).unwrap();
    assert_eq!(
        payload.0.metadata().unwrap().ino(),
        fs::metadata(fixture.path().join("payload")).unwrap().ino()
    );
    let observer = File::open(fixture.path()).unwrap();
    assert_ne!(
        unsafe { libc::flock(observer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    drop(payload);
    drop(root);
    assert_eq!(
        unsafe { libc::flock(observer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    assert!(matches!(
        release(&fixture.owner, &fixture.digest),
        Err(Error::Busy)
    ));
}
#[test]
fn enrollment_rejects_ambiguous_digest_or_archive_substitution() {
    let mut fixture = Fixture::new();
    let archive = fixture.path().join("envelope.cxapp");
    mode(&archive, 0o600);
    fs::write(&archive, b"changed").unwrap();
    mode(&archive, 0o400);
    assert!(matches!(
        release(&fixture.owner, &fixture.digest),
        Err(Error::Identity)
    ));
    mode(&archive, 0o600);
    fs::write(&archive, b"mechanical archive fixture").unwrap();
    mode(&archive, 0o400);
    let second = fixture.root.join("second.db");
    fixture.publish(&second);
    fixture.owner.kernel_database_paths.push(second);
    assert!(matches!(
        release(&fixture.owner, &fixture.digest),
        Err(Error::Identity)
    ));
}
#[test]
fn source_tree_never_adopts_writable_files_links_or_enrolled_root_aliases() {
    let fixture = Fixture::new();
    let payload = fixture.path().join("payload");
    let entry = payload.join("main.js");
    mode(&entry, 0o600);
    assert!(matches!(
        release(&fixture.owner, &fixture.digest),
        Err(Error::Identity)
    ));
    mode(&entry, 0o400);
    mode(&payload, 0o700);
    fs::remove_file(&entry).unwrap();
    symlink("../envelope.cxapp", &entry).unwrap();
    mode(&payload, 0o500);
    assert!(release(&fixture.owner, &fixture.digest).is_err());
    let mut owner = fixture.owner.clone();
    let alias = fixture.root.join("alias");
    symlink(&fixture.root, &alias).unwrap();
    owner.kernel_database_paths = vec![alias.join("kernel.db")];
    assert!(release(&owner, &fixture.digest).is_err());
}

#[test]
fn mechanical_source_bounds_accept_every_default_phase_one_package() {
    let limits = chariox_app_package::Limits::default();
    assert!(ARCHIVE_BYTES >= limits.max_archive_bytes);
    assert!(FILE_BYTES >= limits.max_file_bytes as u64);
    assert!(TREE_DEPTH >= limits.max_path_depth);
    // Each archive file can create at most depth-1 new directories and one
    // file in payload. Tar metadata also counts against max_entries, so this
    // overestimates the actual extracted tree instead of undercounting it.
    assert!(
        TREE_ENTRIES
            >= limits
                .max_entries
                .checked_mul(limits.max_path_depth)
                .unwrap()
    );
}
