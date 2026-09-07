use super::{is_atomic_temporary, try_lock_file, Dir, FsError};
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = fs::canonicalize(std::env::temp_dir()).unwrap();
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = base.join(format!(
            "chariox-private-fs-{}-{time}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn directory(&self) -> Dir {
        Dir::open_private(&self.0).unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn private_file_reopens_without_truncating_and_duplicate_create_preserves_bytes() {
    let scratch = Scratch::new();
    let dir = scratch.directory();
    let name = OsStr::new("upload.part");
    let mut file = dir.create_private_file(name).unwrap();
    file.write_all(b"accepted chunk").unwrap();
    file.sync_all().unwrap();
    assert!(
        matches!(dir.create_private_file(name), Err(FsError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists)
    );
    let mut reopened = dir.open_private_file(name).unwrap();
    let mut bytes = Vec::new();
    reopened.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"accepted chunk");
    reopened.seek(SeekFrom::End(0)).unwrap();
    reopened.write_all(b" and next").unwrap();
    assert_eq!(
        fs::metadata(scratch.0.join(name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(try_lock_file(&file).unwrap());
    assert!(!try_lock_file(&reopened).unwrap());
    dir.remove_file(name).unwrap();
    dir.remove_file(name).unwrap();
}

#[test]
fn metadata_replacement_is_complete_and_removes_temporary_entries() {
    let scratch = Scratch::new();
    let dir = scratch.directory();
    let name = OsStr::new("state.json");
    dir.atomic_replace(name, b"first state").unwrap();
    let old_reader = dir.open_private_file(name).unwrap();
    dir.atomic_replace(name, b"complete second state").unwrap();
    let mut previous = String::new();
    (&old_reader).read_to_string(&mut previous).unwrap();
    assert_eq!(
        previous, "first state",
        "An existing reader retains the old complete inode"
    );
    assert_eq!(
        fs::read(scratch.0.join(name)).unwrap(),
        b"complete second state"
    );
    assert_eq!(dir.entries(10).unwrap(), [name]);
}

#[test]
fn traversal_symlinks_hardlinks_and_nonprivate_files_are_rejected() {
    let scratch = Scratch::new();
    let dir = scratch.directory();
    for name in ["../escape", "parent/file", "parent\\file", ""] {
        assert!(matches!(
            dir.create_private_file(OsStr::new(name)),
            Err(FsError::UnsafeEntry)
        ));
    }
    let file = dir.create_private_file(OsStr::new("source")).unwrap();
    (&file).write_all(b"keep").unwrap();
    symlink(scratch.0.join("source"), scratch.0.join("link")).unwrap();
    assert!(dir.open_private_file(OsStr::new("link")).is_err());
    assert!(dir.atomic_replace(OsStr::new("link"), b"bad").is_err());
    assert_eq!(fs::read(scratch.0.join("source")).unwrap(), b"keep");
    fs::hard_link(scratch.0.join("source"), scratch.0.join("second-link")).unwrap();
    assert!(matches!(
        dir.open_private_file(OsStr::new("source")),
        Err(FsError::UnsafeEntry)
    ));
    fs::remove_file(scratch.0.join("second-link")).unwrap();
    fs::set_permissions(scratch.0.join("source"), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        dir.open_private_file(OsStr::new("source")),
        Err(FsError::UnsafeEntry)
    ));
}

#[test]
fn atomic_recovery_names_do_not_claim_unrelated_files() {
    for name in [".replace-42-1-abcd", ".replace-1-0-0"] {
        assert!(is_atomic_temporary(OsStr::new(name)));
    }
    for name in [
        ".replace-state",
        ".replace-1-2-",
        ".replace-1-2-abcd-extra",
        ".stage-1-2-abcd",
        "state.json",
    ] {
        assert!(!is_atomic_temporary(OsStr::new(name)));
    }
}

#[test]
fn existing_child_discovery_requires_publication_sync_and_the_original_parent_policy() {
    let scratch = Scratch::new();
    let name = OsStr::new("uploads");
    assert!(Dir::open_private_child_if_present(&scratch.0, name)
        .unwrap()
        .is_none());
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
    let parent = scratch.directory();
    // Simulate a visible child left by a creator whose parent publication
    // failed. No existing-store handle may escape the same failure on retry.
    drop(parent.create_child(name).unwrap());
    let mut reached_sync = false;
    let failed = Dir::open_existing_child_checked(&scratch.0, name, || {
        reached_sync = true;
        Err(FsError::Io(std::io::Error::other(
            "injected parent sync failure",
        )))
    });
    assert!(reached_sync);
    assert!(matches!(failed, Err(FsError::Io(_))));
    assert!(scratch.0.join(name).is_dir());
    let reopened = Dir::open_private_child_if_present(&scratch.0, name)
        .unwrap()
        .unwrap();
    assert!(reopened.try_lock().unwrap());
    drop(reopened);
    fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        Dir::open_private_child_if_present(&scratch.0, name),
        Err(FsError::UnsafeEntry)
    ));
    assert!(matches!(
        Dir::open_or_create_private_child(&scratch.0, name),
        Err(FsError::UnsafeEntry)
    ));
    assert_eq!(
        fs::metadata(&scratch.0).unwrap().permissions().mode() & 0o777,
        0o777
    );
}
