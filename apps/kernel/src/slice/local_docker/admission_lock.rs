//! Both resource reservations use the same host-wide lock names as older kernels.
//! Provisioning changes permissions in place; it never swaps a live lock inode.
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) fn path(resource: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/chariox-docker-{resource}-admission.lock"))
}

pub(super) fn open(path: &Path) -> io::Result<File> {
    open_for_owner(path, 0)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!(
        "{message}; provision the host-wide locks as root with deploy/local-linux/provision-docker-admission-locks.py (do not replace a lock while kernels run)"
    ))
}

fn open_for_owner(path: &Path, owner: u32) -> io::Result<File> {
    // /tmp is a root-owned symlink to /private/tmp on macOS. Anchor operations
    // to its validated directory descriptor rather than following a leaf link.
    let parent = path
        .parent()
        .ok_or_else(|| invalid("admission lock has no parent"))?;
    let parent = std::fs::canonicalize(parent)?;
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(parent)?;
    let metadata = directory.metadata()?;
    let writable = metadata.mode() & 0o022 != 0;
    if metadata.uid() != owner || (writable && metadata.mode() & 0o1000 == 0) {
        return Err(invalid(
            "admission lock parent must be root-owned and sticky when writable",
        ));
    }
    let name = path
        .file_name()
        .ok_or_else(|| invalid("admission lock has no filename"))?;
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(name.as_bytes())
        .map_err(|_| invalid("invalid admission lock filename"))?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(invalid(&format!(
            "failed to open admission lock: {}",
            io::Error::last_os_error()
        )));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != owner
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
        || metadata.len() != 0
    {
        return Err(invalid("admission lock must be an empty root-owned regular file with one link and no group/other writes"));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs2::FileExt;
    use std::os::unix::fs::{symlink, PermissionsExt};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir()
                .join(format!("chariox-admission-{:032x}", rand::random::<u128>()));
            std::fs::create_dir(&p).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self(p)
        }
        fn lock(&self) -> PathBuf {
            let p = self.0.join("memory.lock");
            File::create(&p).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
            p
        }
        fn open(&self, p: &Path) -> io::Result<File> {
            open_for_owner(p, unsafe { libc::geteuid() })
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn read_only_lock_contends_on_same_inode_and_recovers() {
        let f = Fixture::new();
        let p = f.lock();
        let first = f.open(&p).unwrap();
        first.lock_exclusive().unwrap();
        let second = f.open(&p).unwrap();
        assert_eq!(
            first.metadata().unwrap().ino(),
            second.metadata().unwrap().ino()
        );
        assert!(second.try_lock_exclusive().is_err());
        let flags = unsafe { libc::fcntl(second.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(flags & libc::O_ACCMODE, libc::O_RDONLY);
        drop(first);
        second.try_lock_exclusive().unwrap();
    }

    #[test]
    fn rejects_replaceable_parent_and_symlink_or_hardlink_leaf() {
        let f = Fixture::new();
        let p = f.lock();
        std::fs::set_permissions(&f.0, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(f.open(&p).is_err());
        std::fs::set_permissions(&f.0, std::fs::Permissions::from_mode(0o1777)).unwrap();
        assert!(f.open(&p).is_ok());
        let link = f.0.join("link");
        symlink(&p, &link).unwrap();
        assert!(f.open(&link).is_err());
        std::fs::remove_file(link).unwrap();
        let link = f.0.join("hardlink");
        std::fs::hard_link(&p, &link).unwrap();
        assert!(f.open(&p).is_err());
    }

    #[test]
    fn rejects_foreign_owner_writable_or_nonempty_leaf() {
        let f = Fixture::new();
        let p = f.lock();
        assert!(open_for_owner(&p, unsafe { libc::geteuid() }.wrapping_add(1)).is_err());
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(f.open(&p).is_err());
        std::fs::write(&p, b"unexpected").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(f.open(&p).is_err());
    }
}
