//! Kernel-held filesystem objects for an owner-selected external directory.
//! The caller owns grant, signer, quota and revocation admission. These types
//! retain the admitted directory objects through I/O; no pathname is reopened
//! after admission. They are never given to a contained App worker.
use crate::private_fs::{self, Dir};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Component, Path},
    time::UNIX_EPOCH,
};

pub const MAX_FILE_BYTES: usize = 512 * 1024;
pub const MAX_DIRECTORY_ENTRIES: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderAccess {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderIdentity {
    device: u64,
    inode: u64,
    created_secs: u64,
    created_nanos: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FolderError {
    #[error("folder_path_invalid")]
    Invalid,
    #[error("folder_identity_changed_choose_again")]
    Identity,
    #[error("folder_entry_unsafe")]
    Unsafe,
    #[error("folder_limit_exceeded")]
    Limit,
    #[error("folder_io_failed")]
    Io,
    #[error("folder_write_outcome_uncertain_do_not_repeat")]
    OutcomeUncertain,
}
type Result<T> = std::result::Result<T, FolderError>;

pub struct ExternalFolder {
    root: Dir,
    identity: FolderIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderEntry {
    pub name: String,
    pub kind: FolderEntryKind,
    pub bytes: u64,
}

impl ExternalFolder {
    /// Select only an absolute, real directory. Birth identity is required so
    /// a recycled inode after restart cannot inherit the owner's permission.
    pub fn select(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            return Err(FolderError::Invalid);
        }
        let mut root = Dir::absolute_root().map_err(fs_error)?;
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    root = root.child(name).map_err(fs_error)?;
                }
                _ => return Err(FolderError::Invalid),
            }
        }
        let metadata = root.0.metadata().map_err(|_| FolderError::Io)?;
        let created = metadata
            .created()
            .map_err(|_| FolderError::Identity)?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| FolderError::Identity)?;
        if created.is_zero() {
            return Err(FolderError::Identity);
        }
        Ok(Self {
            identity: FolderIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
                created_secs: created.as_secs(),
                created_nanos: created.subsec_nanos(),
            },
            root,
        })
    }

    /// Every new operation resolves the selected path again. A moved, missing
    /// or replaced root needs another owner selection, even if still readable.
    pub fn reopen(path: &Path, expected: FolderIdentity) -> Result<Self> {
        let opened = Self::select(path).map_err(|_| FolderError::Identity)?;
        if opened.identity != expected {
            return Err(FolderError::Identity);
        }
        Ok(opened)
    }

    pub fn identity(&self) -> FolderIdentity {
        self.identity
    }

    pub fn read(&self, relative: &str) -> Result<Vec<u8>> {
        let (parent, name) = self.parent(relative)?;
        let name = private_fs::cstring(&name).map_err(fs_error)?;
        let file = private_fs::file(unsafe {
            libc::openat(
                parent.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })
        .map_err(fs_error)?;
        let metadata = file.metadata().map_err(|_| FolderError::Io)?;
        if !metadata.is_file() || metadata.nlink() != 1 || metadata.dev() != self.identity.device {
            return Err(FolderError::Unsafe);
        }
        if metadata.len() > MAX_FILE_BYTES as u64 {
            return Err(FolderError::Limit);
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| FolderError::Io)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(FolderError::Limit);
        }
        Ok(bytes)
    }

    /// Listing omits entries a worker cannot safely use. The entry bound is
    /// applied before filtering, so a directory of unsafe entries is bounded.
    #[allow(
        clippy::unnecessary_cast,
        reason = "libc device/inode types differ on macOS and Linux"
    )]
    pub fn list(&self, relative: &str) -> Result<Vec<FolderEntry>> {
        let directory = if relative.is_empty() {
            Dir(self.root.0.try_clone().map_err(|_| FolderError::Io)?)
        } else {
            let (parent, name) = self.parent(relative)?;
            let child = parent.child(&name).map_err(fs_error)?;
            self.check_device(&child)?;
            child
        };
        let mut entries = Vec::new();
        for name in directory.entries(MAX_DIRECTORY_ENTRIES).map_err(fs_error)? {
            let metadata = private_fs::entry_metadata(&directory, &name).map_err(fs_error)?;
            if metadata.st_dev as u64 != self.identity.device {
                continue;
            }
            let kind = match metadata.st_mode & libc::S_IFMT {
                libc::S_IFREG if metadata.st_nlink == 1 => FolderEntryKind::File,
                libc::S_IFDIR => FolderEntryKind::Directory,
                _ => continue,
            };
            let Some(name) = name.to_str().filter(|name| valid_component(name)) else {
                continue;
            };
            entries.push(FolderEntry {
                name: name.to_owned(),
                kind,
                bytes: u64::try_from(metadata.st_size).map_err(|_| FolderError::Unsafe)?,
            });
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    /// Stage outside the state writer. The caller must fence a current writable
    /// grant and its quota before consuming the prepared value at publication.
    pub fn prepare_replace(&self, relative: &str, bytes: &[u8]) -> Result<FolderReplace> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(FolderError::Limit);
        }
        let (parent, destination) = self.parent(relative)?;
        require_destination(&parent, &destination)?;
        let temporary = OsString::from(format!(".chariox-folder-{:032x}", rand::random::<u128>()));
        let file = parent.create_private_file(&temporary).map_err(fs_error)?;
        let mut staged = FolderReplace {
            parent,
            destination,
            temporary,
            file,
            published: false,
        };
        staged.file.write_all(bytes).map_err(|_| FolderError::Io)?;
        staged.file.sync_all().map_err(|_| FolderError::Io)?;
        Ok(staged)
    }

    fn parent(&self, relative: &str) -> Result<(Dir, OsString)> {
        if relative.len() > 4096 {
            return Err(FolderError::Invalid);
        }
        let components: Vec<_> = relative.split('/').collect();
        if components.is_empty() || components.iter().any(|part| !valid_component(part)) {
            return Err(FolderError::Invalid);
        }
        let mut parent = Dir(self.root.0.try_clone().map_err(|_| FolderError::Io)?);
        for component in &components[..components.len() - 1] {
            parent = parent.child(OsStr::new(component)).map_err(fs_error)?;
            self.check_device(&parent)?;
        }
        Ok((parent, OsString::from(components[components.len() - 1])))
    }

    fn check_device(&self, directory: &Dir) -> Result<()> {
        if directory.0.metadata().map_err(|_| FolderError::Io)?.dev() != self.identity.device {
            return Err(FolderError::Unsafe);
        }
        Ok(())
    }
}

pub struct FolderReplace {
    parent: Dir,
    destination: OsString,
    temporary: OsString,
    file: File,
    published: bool,
}

impl FolderReplace {
    pub fn publish(self) -> Result<()> {
        self.publish_checked(|| Ok(()))
    }

    fn publish_checked(mut self, after_rename: impl FnOnce() -> std::io::Result<()>) -> Result<()> {
        if !same_file(&self.parent, &self.temporary, &self.file)? {
            return Err(FolderError::Unsafe);
        }
        require_destination(&self.parent, &self.destination)?;
        let from = private_fs::cstring(&self.temporary).map_err(fs_error)?;
        let to = private_fs::cstring(&self.destination).map_err(fs_error)?;
        if unsafe {
            libc::renameat(
                self.parent.0.as_raw_fd(),
                from.as_ptr(),
                self.parent.0.as_raw_fd(),
                to.as_ptr(),
            )
        } != 0
        {
            return Err(FolderError::OutcomeUncertain);
        }
        self.published = true;
        after_rename().map_err(|_| FolderError::OutcomeUncertain)?;
        self.parent
            .sync()
            .map_err(|_| FolderError::OutcomeUncertain)
    }
}

impl Drop for FolderReplace {
    fn drop(&mut self) {
        if !self.published && same_file(&self.parent, &self.temporary, &self.file).unwrap_or(false)
        {
            let _ = self.parent.remove_file(&self.temporary);
            let _ = self.parent.sync();
        }
    }
}

fn is_staging_name(value: &str) -> bool {
    const PREFIX: &str = ".chariox-folder-";
    value.len() == PREFIX.len() + 32
        && value
            .get(..PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PREFIX))
        && value.as_bytes()[PREFIX.len()..]
            .iter()
            .all(u8::is_ascii_hexdigit)
}
fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value != "."
        && value != ".."
        && !value.contains(['\\', ':'])
        && !value.chars().any(char::is_control)
        && !is_staging_name(value)
}

fn require_destination(parent: &Dir, name: &OsStr) -> Result<()> {
    match private_fs::entry_metadata(parent, name) {
        Ok(metadata)
            if metadata.st_mode & libc::S_IFMT == libc::S_IFREG && metadata.st_nlink == 1 =>
        {
            Ok(())
        }
        Err(private_fs::FsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(())
        }
        Ok(_) => Err(FolderError::Unsafe),
        Err(error) => Err(fs_error(error)),
    }
}

#[allow(
    clippy::unnecessary_cast,
    reason = "libc device/inode types differ on macOS and Linux"
)]
fn same_file(parent: &Dir, name: &OsStr, expected: &File) -> Result<bool> {
    let metadata = private_fs::entry_metadata(parent, name).map_err(fs_error)?;
    let expected = expected.metadata().map_err(|_| FolderError::Io)?;
    Ok(metadata.st_mode & libc::S_IFMT == libc::S_IFREG
        && metadata.st_nlink == 1
        && metadata.st_dev as u64 == expected.dev()
        && metadata.st_ino as u64 == expected.ino())
}

fn fs_error(error: private_fs::FsError) -> FolderError {
    match error {
        private_fs::FsError::UnsafeEntry => FolderError::Unsafe,
        private_fs::FsError::EntryLimit => FolderError::Limit,
        private_fs::FsError::Io(_) => FolderError::Io,
    }
}

#[cfg(test)]
mod tests;
