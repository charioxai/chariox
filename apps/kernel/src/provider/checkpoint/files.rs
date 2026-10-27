//! Bounded native-file I/O relative to pinned, no-follow directory handles.

use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

use super::{NativeCheckpointFailure as Failure, MAX_NATIVE_CHECKPOINT_BYTES};

pub(super) struct Directory(File);

impl Directory {
    pub(super) fn open(path: &Path) -> Result<Self, Failure> {
        let path = path
            .canonicalize()
            .map_err(|_| Failure::StorageUnavailable)?;
        let mut current = Self(File::open("/").map_err(|_| Failure::StorageUnavailable)?);
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    current =
                        current.child(name.to_str().ok_or(Failure::InvalidArtifact)?, false)?;
                }
                _ => return Err(Failure::InvalidArtifact),
            }
        }
        Ok(current)
    }

    fn child(&self, name: &str, create: bool) -> Result<Self, Failure> {
        let name = component(name)?;
        if create {
            // A concurrent creator is fine; openat below still refuses symlinks.
            let result = unsafe { libc::mkdirat(self.0.as_raw_fd(), name.as_ptr(), 0o700) };
            if result != 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(Failure::StorageUnavailable);
            }
        }
        if create {
            self.0.sync_all().map_err(|_| Failure::StorageUnavailable)?;
        }
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(Failure::StorageUnavailable);
        }
        Ok(Self(unsafe { File::from_raw_fd(fd) }))
    }

    fn parent(&self, path: &str, create: bool) -> Result<(Self, CString), Failure> {
        let mut parts = path.split('/').collect::<Vec<_>>();
        let name = component(parts.pop().ok_or(Failure::InvalidArtifact)?)?;
        let mut directory = Self(
            self.0
                .try_clone()
                .map_err(|_| Failure::StorageUnavailable)?,
        );
        for part in parts {
            directory = directory.child(part, create)?;
        }
        Ok((directory, name))
    }

    pub(super) fn read(&self, path: &str) -> Result<Vec<u8>, Failure> {
        let (parent, name) = self.parent(path, false)?;
        let fd = unsafe {
            libc::openat(
                parent.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(Failure::StorageUnavailable);
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let before = file.metadata().map_err(|_| Failure::StorageUnavailable)?;
        if !before.is_file() {
            return Err(Failure::InvalidArtifact);
        }
        if before.len() > MAX_NATIVE_CHECKPOINT_BYTES as u64 {
            return Err(Failure::TooLarge);
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_NATIVE_CHECKPOINT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Failure::StorageUnavailable)?;
        if bytes.len() > MAX_NATIVE_CHECKPOINT_BYTES {
            return Err(Failure::TooLarge);
        }
        let mut verification = Vec::new();
        file.rewind().map_err(|_| Failure::StorageUnavailable)?;
        (&mut file)
            .take(MAX_NATIVE_CHECKPOINT_BYTES as u64 + 1)
            .read_to_end(&mut verification)
            .map_err(|_| Failure::StorageUnavailable)?;
        let after = file.metadata().map_err(|_| Failure::StorageUnavailable)?;
        if bytes != verification
            || before.len() != after.len()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return Err(Failure::SourceChanged);
        }
        let current_fd = unsafe {
            libc::openat(
                parent.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if current_fd < 0 {
            return Err(Failure::SourceChanged);
        }
        let current = unsafe { File::from_raw_fd(current_fd) }
            .metadata()
            .map_err(|_| Failure::SourceChanged)?;
        if current.dev() != before.dev() || current.ino() != before.ino() {
            return Err(Failure::SourceChanged);
        }
        Ok(bytes)
    }

    pub(super) fn publish(&self, path: &str, bytes: &[u8]) -> Result<(), Failure> {
        self.publish_file(path, bytes, None)
    }

    pub(super) fn extend(&self, path: &str, bytes: &[u8]) -> Result<(), Failure> {
        let previous = self.read(path)?;
        if previous.is_empty() || !previous.ends_with(b"\n") || !bytes.starts_with(&previous) {
            return Err(Failure::TargetConflict);
        }
        self.publish_file(path, bytes, Some(&previous))
    }

    /// Drops everything after `bytes`; the file must still begin with them.
    pub(super) fn rewind(&self, path: &str, bytes: &[u8]) -> Result<(), Failure> {
        let current = self.read(path)?;
        if current == bytes {
            return Ok(());
        }
        if !current.starts_with(bytes) {
            return Err(Failure::TargetConflict);
        }
        self.publish_file(path, bytes, Some(&current))
    }

    fn publish_file(
        &self,
        path: &str,
        bytes: &[u8],
        previous: Option<&[u8]>,
    ) -> Result<(), Failure> {
        let (parent, name) = self.parent(path, true)?;
        let temporary = component(&format!(
            ".chariox-checkpoint-{:032x}.part",
            rand::random::<u128>()
        ))?;
        let fd = unsafe {
            libc::openat(
                parent.0.as_raw_fd(),
                temporary.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(Failure::StorageUnavailable);
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let result = (|| {
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(|_| Failure::StorageUnavailable)?;
            if let Some(previous) = previous {
                // The source provider is stopped and Move fences new launches.
                // Recheck after staging, then atomically publish its extension so
                // a crash cannot leave half a JSONL record in the native thread.
                if self.read(path)? != previous {
                    return Err(Failure::SourceChanged);
                }
                let renamed = unsafe {
                    libc::renameat(
                        parent.0.as_raw_fd(),
                        temporary.as_ptr(),
                        parent.0.as_raw_fd(),
                        name.as_ptr(),
                    )
                };
                if renamed != 0 {
                    return Err(Failure::StorageUnavailable);
                }
                return parent.0.sync_all().map_err(|_| Failure::StorageUnavailable);
            }
            // Hard-link publication is atomic and can never replace user data.
            let result = unsafe {
                libc::linkat(
                    parent.0.as_raw_fd(),
                    temporary.as_ptr(),
                    parent.0.as_raw_fd(),
                    name.as_ptr(),
                    0,
                )
            };
            if result != 0 {
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(Failure::StorageUnavailable);
                }
                if self.read(path)? != bytes {
                    return Err(Failure::TargetConflict);
                }
            }
            parent.0.sync_all().map_err(|_| Failure::StorageUnavailable)
        })();
        let removed = unsafe { libc::unlinkat(parent.0.as_raw_fd(), temporary.as_ptr(), 0) };
        if removed != 0
            && !(previous.is_some()
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound)
        {
            return Err(Failure::StorageUnavailable);
        }
        parent
            .0
            .sync_all()
            .map_err(|_| Failure::StorageUnavailable)?;
        result
    }
}

fn component(name: &str) -> Result<CString, Failure> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(Failure::InvalidArtifact);
    }
    CString::new(name).map_err(|_| Failure::InvalidArtifact)
}
