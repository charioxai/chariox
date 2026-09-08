//! Root-owned descriptor-relative image/journal operations. Image bytes and
//! mount targets are never opened from a client-supplied pathname.
use super::{
    model::{Identity, Journal},
    Error, Result,
};
use crate::private_fs::{entry_metadata, Dir};
use std::{
    ffi::{CString, OsStr},
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::{Component, Path},
};

pub(super) fn identity(file: &File) -> Result<Identity> {
    let metadata = file.metadata()?;
    Ok(Identity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}
pub(super) fn require(parent: &Dir, name: &str, file: &File, expected: &Identity) -> Result<()> {
    let named = entry_metadata(parent, OsStr::new(name))?;
    if identity(file)? != *expected
        || named.st_dev as u64 != expected.device
        || named.st_ino as u64 != expected.inode
    {
        return Err(Error::Identity);
    }
    Ok(())
}

/// Installer-owned absolute path, with every ancestor opened without symlinks.
/// Only root may change the namespace where our derived image paths are used.
pub(super) fn root_directory(path: &Path) -> Result<Dir> {
    if !path.is_absolute() || path.as_os_str().as_encoded_bytes().len() > 700 {
        return Err(Error::Identity);
    }
    let mut dir = Dir::absolute_root()?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                dir = dir.child(name)?;
            }
            _ => return Err(Error::Identity),
        }
        root_owned(&dir.0, true)?;
    }
    Ok(dir)
}
pub(super) fn root_owned(file: &File, directory: bool) -> Result<()> {
    let metadata = file.metadata()?;
    if metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || metadata.is_dir() != directory
        || (!directory && (!metadata.is_file() || metadata.nlink() != 1))
    {
        return Err(Error::Identity);
    }
    Ok(())
}
pub(super) fn child(parent: &Dir, name: &str, mode: u32) -> Result<Dir> {
    if ![0o700, 0o711].contains(&mode) {
        return Err(Error::Invalid);
    }
    component(name)?;
    let dir = match parent.create_child(OsStr::new(name)) {
        Ok(dir) => dir,
        Err(crate::private_fs::FsError::Io(error))
            if error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            parent.child(OsStr::new(name))?
        }
        Err(error) => return Err(error.into()),
    };
    root_owned(&dir.0, true)?;
    if unsafe { libc::fchmod(dir.0.as_raw_fd(), mode) } != 0 {
        return Err(Error::Io);
    }
    dir.sync()?;
    parent.sync()?;
    Ok(dir)
}

pub(super) fn open_image(parent: &Dir, name: &str) -> Result<Option<File>> {
    let name = component(name)?;
    let fd = unsafe {
        libc::openat(
            parent.0.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(Error::Io)
        };
    }
    let file = unsafe { File::from_raw_fd(fd) };
    root_owned(&file, false)?;
    if file.metadata()?.mode() & 0o7777 != 0o600 {
        return Err(Error::Identity);
    }
    if !crate::private_fs::try_lock_file(&file)? {
        return Err(Error::Busy);
    }
    Ok(Some(file))
}

pub(super) fn create_image(parent: &Dir, name: &str, capacity: u64) -> Result<File> {
    if ![super::DATA_BYTES, super::TMP_BYTES].contains(&capacity) {
        return Err(Error::Capacity);
    }
    let mut file = parent.create_private_file(OsStr::new(name))?;
    if !crate::private_fs::try_lock_file(&file)? {
        return Err(Error::Busy);
    }
    if unsafe { libc::fchmod(file.as_raw_fd(), 0o600) } != 0 {
        return Err(Error::Io);
    }
    // Allocate real host storage before formatting. Failure retains the named
    // creation intent for recovery; it never fabricates a successful reservation.
    let error = unsafe { libc::posix_fallocate(file.as_raw_fd(), 0, capacity as libc::off_t) };
    if error != 0 {
        return Err(if [libc::ENOSPC, libc::EDQUOT].contains(&error) {
            Error::Capacity
        } else {
            Error::Io
        });
    }
    if file.metadata()?.len() != capacity {
        return Err(Error::Identity);
    }
    file.flush()?;
    file.sync_all()?;
    parent.sync()?;
    Ok(file)
}

pub(super) fn read_journal(parent: &Dir) -> Result<Option<Journal>> {
    let file = match parent.read_file(OsStr::new("journal.json"), false) {
        Ok(file) => file,
        Err(crate::private_fs::FsError::Io(error))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error.into()),
    };
    root_owned(&file, false)?;
    if file.metadata()?.mode() & 0o7777 != 0o600 {
        return Err(Error::Identity);
    }
    let mut bytes = Vec::new();
    file.take(16385).read_to_end(&mut bytes)?;
    if bytes.len() > 16384 {
        return Err(Error::Identity);
    }
    Ok(Some(
        serde_json::from_slice(&bytes).map_err(|_| Error::Identity)?,
    ))
}
pub(super) fn save_journal(parent: &Dir, journal: &Journal) -> Result<()> {
    let bytes = serde_json::to_vec(journal).map_err(|_| Error::Invalid)?;
    if bytes.len() > 16384 {
        return Err(Error::Invalid);
    }
    // Existing descriptor-relative atomic replace fsyncs file and directory.
    parent.atomic_replace(OsStr::new("journal.json"), &bytes)?;
    Ok(())
}

pub(super) fn component(value: &str) -> Result<CString> {
    if value.is_empty()
        || value.len() > 128
        || value.contains('/')
        || value.contains('\0')
        || [".", ".."].contains(&value)
    {
        return Err(Error::Identity);
    }
    CString::new(value).map_err(|_| Error::Identity)
}
