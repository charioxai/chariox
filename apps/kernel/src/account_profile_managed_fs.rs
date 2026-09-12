//! No-follow operations on a managed provider directory pinned by a file handle.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

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
        metadata_at(&self.directory, &component(name)?).map(|metadata| metadata.is_some())
    }

    pub(super) fn identity(&self, name: &str) -> Result<(u64, u64), DaemonError> {
        let directory = open_child_directory(&self.directory, name)?;
        let metadata = directory
            .metadata()
            .map_err(registry_io("inspect managed account profile"))?;
        Ok((metadata.dev(), metadata.ino()))
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
        remove_tree_at(&self.directory, &component(name)?, None)
    }

    pub(super) fn remove_matching(
        &self,
        name: &str,
        expected_identity: (u64, u64),
    ) -> Result<(), DaemonError> {
        remove_tree_at(&self.directory, &component(name)?, Some(expected_identity))
    }

    pub(super) fn sync(&self) -> Result<(), DaemonError> {
        self.directory
            .sync_all()
            .map_err(registry_io("sync account profile directory"))
    }
}

fn open_child_directory(parent: &File, name: &str) -> Result<File, DaemonError> {
    let name = component(name)?;
    open_child_directory_cstr(parent, &name)
}

fn open_child_directory_cstr(parent: &File, name: &CStr) -> Result<File, DaemonError> {
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

fn metadata_at(parent: &File, name: &CStr) -> Result<Option<libc::stat>, DaemonError> {
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Ok(Some(unsafe { metadata.assume_init() }));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(None)
    } else {
        Err(registry_io("inspect managed account profile")(error))
    }
}

fn remove_tree_at(
    parent: &File,
    name: &CStr,
    expected_identity: Option<(u64, u64)>,
) -> Result<(), DaemonError> {
    let directory = open_child_directory_cstr(parent, name)?;
    let pinned = directory
        .metadata()
        .map_err(registry_io("inspect managed account profile"))?;
    if expected_identity.is_some_and(|expected| expected != (pinned.dev(), pinned.ino())) {
        return Err(registry_error(
            "delete managed account profile",
            "managed account root identity changed",
        ));
    }
    let duplicate = unsafe { libc::dup(directory.as_raw_fd()) };
    if duplicate < 0 {
        return Err(registry_io("delete managed account profile")(
            std::io::Error::last_os_error(),
        ));
    }
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        unsafe { libc::close(duplicate) };
        return Err(registry_io("delete managed account profile")(
            std::io::Error::last_os_error(),
        ));
    }
    let stream = DirectoryStream(stream);
    loop {
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            break;
        }
        let child_name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if child_name.to_bytes() == b"." || child_name.to_bytes() == b".." {
            continue;
        }
        let Some(child) = metadata_at(&directory, child_name)? else {
            continue;
        };
        if child.st_mode & libc::S_IFMT == libc::S_IFDIR {
            remove_tree_at(&directory, child_name, None)?;
        } else if unsafe { libc::unlinkat(directory.as_raw_fd(), child_name.as_ptr(), 0) } != 0 {
            return Err(registry_io("delete managed account profile")(
                std::io::Error::last_os_error(),
            ));
        }
    }
    drop(stream);
    directory
        .sync_all()
        .map_err(registry_io("sync account profile directory"))?;
    let current = metadata_at(parent, name)?.ok_or_else(|| {
        registry_error(
            "delete managed account profile",
            "managed account path changed during deletion",
        )
    })?;
    if current.st_dev as u64 != pinned.dev() || current.st_ino as u64 != pinned.ino() {
        return Err(registry_error(
            "delete managed account profile",
            "managed account path changed during deletion",
        ));
    }
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(registry_io("delete managed account profile")(
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

struct DirectoryStream(*mut libc::DIR);

impl Drop for DirectoryStream {
    fn drop(&mut self) {
        unsafe { libc::closedir(self.0) };
    }
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
