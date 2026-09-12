//! No-follow operations on a managed provider directory pinned by a file handle.

use std::ffi::CString;
use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use crate::error::DaemonError;

use super::{registry_error, registry_io};

pub(super) struct ManagedProviderParent {
    directory: File,
}

impl ManagedProviderParent {
    pub(super) fn open(
        registry_path: &Path,
        owner_component: &str,
        provider: &str,
    ) -> Result<Self, DaemonError> {
        let registry_parent = File::open(registry_path.parent().unwrap_or_else(|| Path::new(".")))
            .map_err(registry_io("open managed account parent"))?;
        let base = open_child_directory(&registry_parent, "provider-accounts")?;
        let owner = open_child_directory(&base, owner_component)?;
        let directory = open_child_directory(&owner, provider)?;
        Ok(Self { directory })
    }

    pub(super) fn exists(&self, name: &str) -> Result<bool, DaemonError> {
        match fs::symlink_metadata(self.path(name)) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(registry_io("inspect managed account profile")(error)),
        }
    }

    pub(super) fn require_directory(&self, name: &str) -> Result<(), DaemonError> {
        let metadata = fs::symlink_metadata(self.path(name))
            .map_err(registry_io("inspect managed account profile"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(registry_error(
                "inspect managed account profile",
                "managed account profile path must be a regular directory",
            ));
        }
        Ok(())
    }

    pub(super) fn rename(&self, from: &str, to: &str) -> Result<(), DaemonError> {
        let from = component(from)?;
        let to = component(to)?;
        // Both names resolve relative to the same pinned, no-follow parent.
        let result = unsafe {
            libc::renameat(
                self.directory.as_raw_fd(),
                from.as_ptr(),
                self.directory.as_raw_fd(),
                to.as_ptr(),
            )
        };
        if result != 0 {
            return Err(registry_io("move managed account profile")(
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    pub(super) fn remove(&self, name: &str) -> Result<(), DaemonError> {
        self.require_directory(name)?;
        // The fd path pins the parent even if its original pathname changes.
        fs::remove_dir_all(self.path(name)).map_err(registry_io("delete managed account profile"))
    }

    pub(super) fn sync(&self) -> Result<(), DaemonError> {
        self.directory
            .sync_all()
            .map_err(registry_io("sync account profile directory"))
    }

    fn path(&self, name: &str) -> PathBuf {
        #[cfg(target_os = "linux")]
        let fd_root = "/proc/self/fd";
        #[cfg(not(target_os = "linux"))]
        let fd_root = "/dev/fd";
        Path::new(fd_root)
            .join(self.directory.as_raw_fd().to_string())
            .join(name)
    }
}

fn open_child_directory(parent: &File, name: &str) -> Result<File, DaemonError> {
    let name = component(name)?;
    // O_NOFOLLOW rejects a swapped symlink at each untrusted path component.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(registry_io("open managed account parent")(
            std::io::Error::last_os_error(),
        ));
    }
    Ok(unsafe { <File as std::os::fd::FromRawFd>::from_raw_fd(fd) })
}

fn component(name: &str) -> Result<CString, DaemonError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(registry_error(
            "access managed account profile",
            "invalid managed account path component",
        ));
    }
    CString::new(name).map_err(|_| {
        registry_error(
            "access managed account profile",
            "invalid managed account path component",
        )
    })
}
