//! The worker the kernel verified is the worker it runs. Linux execs the
//! verified descriptor itself. macOS cannot exec a descriptor (`/dev/fd/N` is
//! EACCES, no `fexecve`), and execing the installed path after hashing it
//! leaves a window in which a swapped file runs instead. So the kernel streams
//! the installed worker once, hashing and copying the same bytes into a
//! read-only file inside a directory only the kernel user can enter, and runs
//! that copy. The installed path is never executed, and the copy's identity is
//! checked again before every exec.
use super::AotError;
use crate::private_fs::{Dir, FsError};
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::PathBuf,
};

const COPY_NAME: &str = "worker";
const COPY_MODE: u32 = 0o500;

/// What the kernel verified, as the filesystem reports it. Any rewrite, chmod,
/// rename or replacement of the copy changes at least one field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Fingerprint {
    device: u64,
    inode: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Fingerprint {
    fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            length: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

/// Owns the private directory; dropping it removes the copy.
struct Directory {
    parent: Dir,
    name: OsString,
    child: Dir,
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = self.child.remove_file(OsStr::new(COPY_NAME));
        let _ = self.parent.remove_directory(&self.name);
    }
}

pub(super) struct PrivateCopy {
    _directory: Directory,
    path: PathBuf,
    pinned: Fingerprint,
}

impl PrivateCopy {
    /// Streams `installed` once into a private read-only copy. Returns the copy
    /// opened read-only, its guard and the digest of the bytes written.
    pub(super) fn create(
        installed: &mut File,
        parent: &std::path::Path,
    ) -> Result<(File, Self, String), AotError> {
        // The parent must be real: Dir refuses symlinked ancestors, and macOS's
        // temporary directory sits behind the /var -> /private/var link.
        let parent_path = std::fs::canonicalize(parent)?;
        let parent = Dir::open_owned_parent(&parent_path).map_err(unusable)?;
        let name = OsString::from(format!(
            "chariox-app-worker-{:032x}",
            rand::random::<u128>()
        ));
        let child = parent.create_child(&name).map_err(unusable)?;
        let directory = Directory {
            parent,
            name: name.clone(),
            child,
        };
        let mut copy = directory
            .child
            .create_private_file(OsStr::new(COPY_NAME))
            .map_err(unusable)?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let size = installed.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
            copy.write_all(&buffer[..size])?;
        }
        copy.sync_all()?;
        if unsafe { libc::fchmod(copy.as_raw_fd(), COPY_MODE as libc::mode_t) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // No descriptor that could write the copy outlives this function.
        drop(copy);
        let executable = directory
            .child
            .read_file(OsStr::new(COPY_NAME), true)
            .map_err(unusable)?;
        let pinned = Fingerprint::of(&executable.metadata()?);
        let path = parent_path.join(&name).join(COPY_NAME);
        Ok((
            executable,
            Self {
                _directory: directory,
                path,
                pinned,
            },
            format!("sha256:{:x}", hash.finalize()),
        ))
    }

    /// The path to exec, only while it still names the verified bytes.
    pub(super) fn path(&self, executable: &File) -> Result<PathBuf, AotError> {
        let held = Fingerprint::of(&executable.metadata()?);
        let named = std::fs::symlink_metadata(&self.path)?;
        if held != self.pinned
            || Fingerprint::of(&named) != self.pinned
            || !named.is_file()
            || named.nlink() != 1
            || named.mode() & 0o7777 != COPY_MODE
            || named.uid() != unsafe { libc::geteuid() }
        {
            return Err(AotError::Integrity);
        }
        Ok(self.path.clone())
    }
}

fn unusable(error: FsError) -> AotError {
    eprintln!("App worker private copy unavailable: {error}");
    AotError::Runtime
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("pinned-{:032x}", rand::random::<u128>()));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
        fn installed(&self, bytes: &[u8]) -> File {
            let path = self.0.join("installed");
            std::fs::write(&path, bytes).unwrap();
            File::open(path).unwrap()
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_copy_holds_the_hashed_bytes_even_after_the_installed_path_is_swapped() {
        let scratch = Scratch::new();
        let mut installed = scratch.installed(b"pinned bytes");
        let (executable, copy, build) = PrivateCopy::create(&mut installed, &scratch.0).unwrap();
        std::fs::remove_file(scratch.0.join("installed")).unwrap();
        std::fs::write(scratch.0.join("installed"), b"swapped bytes").unwrap();
        let path = copy.path(&executable).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"pinned bytes");
        assert_eq!(
            build,
            format!("sha256:{:x}", Sha256::digest(b"pinned bytes"))
        );
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, COPY_MODE);
    }

    #[test]
    fn a_copy_replaced_after_the_check_is_refused_before_exec() {
        let scratch = Scratch::new();
        let mut installed = scratch.installed(b"pinned bytes");
        let (executable, copy, _) = PrivateCopy::create(&mut installed, &scratch.0).unwrap();
        let path = copy.path(&executable).unwrap();
        std::fs::rename(&path, path.with_file_name("moved")).unwrap();
        std::fs::write(&path, b"swapped bytes").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(COPY_MODE)).unwrap();
        assert!(matches!(copy.path(&executable), Err(AotError::Integrity)));
    }

    #[test]
    fn a_copy_rewritten_in_place_is_refused_before_exec() {
        let scratch = Scratch::new();
        let mut installed = scratch.installed(b"pinned bytes");
        let (executable, copy, _) = PrivateCopy::create(&mut installed, &scratch.0).unwrap();
        let path = copy.path(&executable).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(&path, b"pinned bytes").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(COPY_MODE)).unwrap();
        assert!(matches!(copy.path(&executable), Err(AotError::Integrity)));
    }

    #[test]
    fn dropping_the_copy_removes_its_directory() {
        let scratch = Scratch::new();
        let mut installed = scratch.installed(b"pinned bytes");
        let (executable, copy, _) = PrivateCopy::create(&mut installed, &scratch.0).unwrap();
        let directory = copy.path(&executable).unwrap().parent().unwrap().to_owned();
        assert!(directory.is_dir());
        drop(copy);
        assert!(!directory.exists());
    }
}
