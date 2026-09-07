//! Descriptor-relative filesystem operations shared by release staging and recovery.
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::{File, Metadata};
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

#[derive(Debug, thiserror::Error)]
pub(crate) enum FsError {
    #[error("private_fs_io: {0}")]
    Io(#[from] std::io::Error),
    #[error("private_fs_unsafe_entry")]
    UnsafeEntry,
    #[error("private_fs_entry_limit")]
    EntryLimit,
}

type Result<T> = std::result::Result<T, FsError>;

mod atomic;
pub(crate) use atomic::is_atomic_temporary;
#[cfg(test)]
mod tests;

pub(crate) struct Dir(pub(crate) File);

impl Dir {
    /// Kernel-private directory with no symlink traversal in any ancestor.
    pub(crate) fn open_private(path: &Path) -> Result<Self> {
        let root = Self::open_real(path)?;
        root.require_private()?;
        Ok(root)
    }

    /// Creates only one private child under an existing kernel-owned parent.
    /// Both validation and subsequent I/O retain the same directory descriptors.
    pub(crate) fn open_or_create_private_child(parent: &Path, name: &OsStr) -> Result<Self> {
        let parent = Self::open_real(parent)?;
        let metadata = parent.0.metadata()?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
            return Err(FsError::UnsafeEntry);
        }
        let child = match parent.create_child(name) {
            Ok(child) => child,
            Err(FsError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                parent.child(name)?
            }
            Err(error) => return Err(error),
        };
        child.require_private()?;
        // Also complete a previous creator's interrupted directory publication.
        child.sync()?;
        parent.sync()?;
        Ok(child)
    }

    fn open_real(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            return Err(FsError::UnsafeEntry);
        }
        let mut root = Self::absolute_root()?;
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => root = root.child(name)?,
                _ => return Err(FsError::UnsafeEntry),
            }
        }
        Ok(root)
    }

    fn require_private(&self) -> Result<()> {
        let metadata = self.0.metadata()?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o700 {
            return Err(FsError::UnsafeEntry);
        }
        Ok(())
    }

    pub(crate) fn absolute_root() -> Result<Self> {
        let root = CString::new("/").expect("constant root path");
        let fd = unsafe { libc::open(root.as_ptr(), directory_flags()) };
        Ok(Self(file(fd)?))
    }
    pub(crate) fn child(&self, name: &OsStr) -> Result<Self> {
        let name = cstring(name)?;
        let fd = unsafe { libc::openat(self.0.as_raw_fd(), name.as_ptr(), directory_flags()) };
        Ok(Self(file(fd)?))
    }
    pub(crate) fn create_child(&self, name: &OsStr) -> Result<Self> {
        let encoded = cstring(name)?;
        check(unsafe { libc::mkdirat(self.0.as_raw_fd(), encoded.as_ptr(), 0o700) })?;
        self.child(name)
    }
    pub(crate) fn write_new(&self, name: &OsStr, bytes: &[u8]) -> Result<()> {
        let mut output = self.create_private_file(name)?;
        output.write_all(bytes)?;
        check(unsafe { libc::fchmod(output.as_raw_fd(), 0o400) })?;
        output.sync_all()?;
        Ok(())
    }
    pub(crate) fn read_file(&self, name: &OsStr, readonly: bool) -> Result<File> {
        let name = cstring(name)?;
        let input = file(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        let metadata = input.metadata()?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
            || (readonly && metadata.mode() & 0o222 != 0)
        {
            return Err(FsError::UnsafeEntry);
        }
        Ok(input)
    }
    pub(crate) fn readonly(&self) -> Result<()> {
        check(unsafe { libc::fchmod(self.0.as_raw_fd(), 0o500) })
    }
    pub(crate) fn try_lock(&self) -> Result<bool> {
        try_lock_file(&self.0)
    }
    pub(crate) fn create_private_file(&self, name: &OsStr) -> Result<File> {
        let name = cstring(name)?;
        let output = file(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        })?;
        check(unsafe { libc::fchmod(output.as_raw_fd(), 0o600) })?;
        Ok(output)
    }
    pub(crate) fn open_private_file(&self, name: &OsStr) -> Result<File> {
        let name = cstring(name)?;
        let input = file(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        let metadata = input.metadata()?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(FsError::UnsafeEntry);
        }
        Ok(input)
    }
    /// Removes the directory entry itself, including a symlink, never its target.
    pub(crate) fn remove_file(&self, name: &OsStr) -> Result<()> {
        let name = cstring(name)?;
        let result = unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(error.into())
        }
    }
    pub(crate) fn sync(&self) -> Result<()> {
        self.0.sync_all().map_err(Into::into)
    }
    pub(crate) fn entries(&self, limit: usize) -> Result<Vec<OsString>> {
        // Reopening '.' gives readdir its own offset; dup would share it.
        let fd = self.child(OsStr::new("."))?.0;
        use std::os::fd::IntoRawFd;
        let raw = fd.into_raw_fd();
        let pointer = unsafe { libc::fdopendir(raw) };
        if pointer.is_null() {
            let error = std::io::Error::last_os_error();
            unsafe { libc::close(raw) };
            return Err(error.into());
        }
        struct IteratorHandle(*mut libc::DIR);
        impl Drop for IteratorHandle {
            fn drop(&mut self) {
                unsafe { libc::closedir(self.0) };
            }
        }
        let iterator = IteratorHandle(pointer);
        let mut names = Vec::new();
        loop {
            set_errno(0);
            let entry = unsafe { libc::readdir(iterator.0) };
            if entry.is_null() {
                if errno() != 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                return Ok(names);
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            if names.len() >= limit {
                return Err(FsError::EntryLimit);
            }
            names.push(OsString::from_vec(name.to_vec()));
        }
    }
    pub(crate) fn remove_directory(&self, name: &OsStr) -> Result<()> {
        let name = cstring(name)?;
        check(unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) })
    }
}

pub(crate) fn try_lock_file(file: &File) -> Result<bool> {
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(error.into())
    }
}

/// Keep the caller's recovery marker in the root until every payload deletion
/// has been synced. Descendants have no preserved entry. The caller removes the
/// marker only after this returns, immediately before removing the root itself.
pub(crate) fn remove_contents_preserving(
    dir: &Dir,
    remaining: &mut usize,
    marker: &OsStr,
    checkpoint: &mut impl FnMut() -> std::io::Result<()>,
) -> Result<()> {
    remove_entries(dir, remaining, 0, Some(marker), checkpoint)
}

fn remove_entries(
    dir: &Dir,
    remaining: &mut usize,
    depth: usize,
    marker: Option<&OsStr>,
    checkpoint: &mut impl FnMut() -> std::io::Result<()>,
) -> Result<()> {
    if depth >= 64 {
        return Err(FsError::EntryLimit);
    }
    check(unsafe { libc::fchmod(dir.0.as_raw_fd(), 0o700) })?;
    for name in dir.entries(*remaining)? {
        if marker == Some(name.as_os_str()) {
            continue;
        }
        *remaining = remaining.checked_sub(1).ok_or(FsError::EntryLimit)?;
        let metadata = entry_metadata(dir, &name)?;
        if metadata.st_mode & libc::S_IFMT == libc::S_IFDIR {
            let child = dir.child(&name)?;
            remove_entries(&child, remaining, depth + 1, None, checkpoint)?;
            if !same_entry(dir, &name, &child)? {
                return Err(FsError::UnsafeEntry);
            }
            dir.remove_directory(&name)?;
        } else {
            let name = cstring(&name)?;
            check(unsafe { libc::unlinkat(dir.0.as_raw_fd(), name.as_ptr(), 0) })?;
        }
        checkpoint()?;
    }
    dir.0.sync_all()?;
    Ok(())
}

pub(crate) fn cstring(value: &OsStr) -> Result<CString> {
    if value.as_bytes().is_empty()
        || value.as_bytes().contains(&b'/')
        || value.as_bytes().contains(&b'\\')
        || value == OsStr::new("..")
    {
        return Err(FsError::UnsafeEntry);
    }
    CString::new(value.as_bytes()).map_err(|_| FsError::UnsafeEntry)
}
pub(crate) fn directory_flags() -> libc::c_int {
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC
}
pub(crate) fn file(fd: libc::c_int) -> Result<File> {
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub(crate) fn check(result: libc::c_int) -> Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}
pub(crate) fn entry_metadata(dir: &Dir, name: &OsStr) -> Result<libc::stat> {
    let name = cstring(name)?;
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    check(unsafe {
        libc::fstatat(
            dir.0.as_raw_fd(),
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })?;
    Ok(unsafe { metadata.assume_init() })
}
pub(crate) fn same_entry(parent: &Dir, name: &OsStr, expected: &Dir) -> Result<bool> {
    let actual = entry_metadata(parent, name)?;
    let metadata: Metadata = expected.0.metadata()?;
    Ok(actual.st_mode & libc::S_IFMT == libc::S_IFDIR
        && actual.st_dev as u64 == metadata.dev()
        && actual.st_ino as u64 == metadata.ino())
}
pub(crate) fn publish(root: &Dir, from: &OsStr, to: &OsStr) -> std::io::Result<()> {
    let from = cstring(from)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let to = cstring(to)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            root.0.as_raw_fd(),
            from.as_ptr(),
            root.0.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            root.0.as_raw_fd(),
            from.as_ptr(),
            root.0.as_raw_fd(),
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
#[cfg(target_os = "macos")]
pub(crate) fn errno_pointer() -> *mut libc::c_int {
    unsafe { libc::__error() }
}
#[cfg(target_os = "linux")]
pub(crate) fn errno_pointer() -> *mut libc::c_int {
    unsafe { libc::__errno_location() }
}
pub(crate) fn errno() -> libc::c_int {
    unsafe { *errno_pointer() }
}
pub(crate) fn set_errno(value: libc::c_int) {
    unsafe {
        *errno_pointer() = value;
    }
}
