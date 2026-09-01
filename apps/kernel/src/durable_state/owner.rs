//! A process-lifetime fence for the kernel which restores and schedules state.
//! Database observers do not acquire this lease. SQLite transaction locks alone
//! cannot prevent two runtimes from making decisions from the same snapshot.
use std::fs::{File, OpenOptions};
use std::path::Path;

use crate::error::DaemonError;

pub(super) fn acquire(path: &Path) -> Result<File, DaemonError> {
    let fail = |error: std::io::Error| DaemonError::LocalTransport {
        operation: "durable_state.acquire_owner",
        message: error.to_string(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(fail)?;
    }
    let mut name = path.as_os_str().to_os_string();
    name.push(".owner.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(name).map_err(fail)?;
    let metadata = file.metadata().map_err(fail)?;
    let regular = metadata.is_file();
    #[cfg(windows)]
    let regular = {
        use std::os::windows::fs::MetadataExt;
        regular && metadata.file_attributes() & 0x400 == 0
    };
    if !regular {
        return Err(DaemonError::LocalTransport {
            operation: "durable_state.acquire_owner",
            message: "durable state ownership lock must be a regular file".to_string(),
        });
    }
    fs2::FileExt::try_lock_exclusive(&file).map_err(|error| DaemonError::LocalTransport {
        operation: "durable_state.acquire_owner",
        message: if error.kind() == std::io::ErrorKind::WouldBlock {
            "durable state is already owned by another kernel".to_string()
        } else {
            format!("cannot acquire durable state ownership: {error}")
        },
    })?;
    // Never unlink the lock file. Replacing its inode would let a second process
    // acquire a different lock while the original owner still holds this one.
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRoot(std::path::PathBuf);
    impl TestRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "chariox-state-owner-{:032x}",
                rand::random::<u128>()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn durable_state_owner_is_exclusive_until_last_clone_drops() {
        let root = TestRoot::new();
        let path = root.path().join("state.db");
        let owner = std::sync::Arc::new(acquire(&path).unwrap());
        let clone = owner.clone();
        assert!(acquire(&path).is_err());
        drop(owner);
        assert!(acquire(&path).is_err());
        let other = acquire(&root.path().join("other.db")).unwrap();
        drop(clone);
        assert!(acquire(&path).is_ok());
        drop(other);
    }

    #[cfg(unix)]
    #[test]
    fn durable_state_owner_rejects_symlink_lock() {
        let root = TestRoot::new();
        let target = root.path().join("target");
        std::fs::write(&target, "unchanged").unwrap();
        std::os::unix::fs::symlink(&target, root.path().join("state.db.owner.lock")).unwrap();
        assert!(acquire(&root.path().join("state.db")).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
    }
}
