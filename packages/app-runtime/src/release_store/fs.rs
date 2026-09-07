//! Descriptor-relative filesystem operations shared by release staging and recovery.
use super::unix::ReleaseStoreError;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::{File, Metadata};
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;

type Result<T> = std::result::Result<T, ReleaseStoreError>;

pub(super) struct Dir(pub(super) File);

impl Dir {
    pub(super) fn absolute_root() -> Result<Self> {
        let root = CString::new("/").expect("constant root path");
        let fd = unsafe { libc::open(root.as_ptr(), directory_flags()) };
        Ok(Self(file(fd)?))
    }
    pub(super) fn child(&self, name: &OsStr) -> Result<Self> {
        let name = cstring(name)?;
        let fd = unsafe { libc::openat(self.0.as_raw_fd(), name.as_ptr(), directory_flags()) };
        Ok(Self(file(fd)?))
    }
    pub(super) fn create_child(&self, name: &OsStr) -> Result<Self> {
        let encoded = cstring(name)?;
        check(unsafe { libc::mkdirat(self.0.as_raw_fd(), encoded.as_ptr(), 0o700) })?;
        self.child(name)
    }
    pub(super) fn write_new(&self, name: &OsStr, bytes: &[u8]) -> Result<()> {
        let name = cstring(name)?;
        let mut output = file(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        })?;
        output.write_all(bytes)?;
        check(unsafe { libc::fchmod(output.as_raw_fd(), 0o400) })?;
        output.sync_all()?;
        Ok(())
    }
    pub(super) fn read_file(&self, name: &OsStr, readonly: bool) -> Result<File> {
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
            return Err(ReleaseStoreError::UnsafeEntry);
        }
        Ok(input)
    }
    pub(super) fn readonly(&self) -> Result<()> {
        check(unsafe { libc::fchmod(self.0.as_raw_fd(), 0o500) })
    }
    pub(super) fn try_lock(&self) -> Result<bool> {
        if unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            Ok(false)
        } else {
            Err(error.into())
        }
    }
    pub(super) fn entries(&self, limit: usize) -> Result<Vec<OsString>> {
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
                return Err(ReleaseStoreError::EntryLimit);
            }
            names.push(OsString::from_vec(name.to_vec()));
        }
    }
    pub(super) fn remove_directory(&self, name: &OsStr) -> Result<()> {
        let name = cstring(name)?;
        check(unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) })
    }
}

pub(super) fn remove_contents(dir: &Dir, remaining: &mut usize, depth: usize) -> Result<()> {
    if depth >= 64 {
        return Err(ReleaseStoreError::EntryLimit);
    }
    check(unsafe { libc::fchmod(dir.0.as_raw_fd(), 0o700) })?;
    for name in dir.entries(*remaining)? {
        *remaining = remaining
            .checked_sub(1)
            .ok_or(ReleaseStoreError::EntryLimit)?;
        let metadata = entry_metadata(dir, &name)?;
        if metadata.st_mode & libc::S_IFMT == libc::S_IFDIR {
            let child = dir.child(&name)?;
            remove_contents(&child, remaining, depth + 1)?;
            if !same_entry(dir, &name, &child)? {
                return Err(ReleaseStoreError::UnsafeEntry);
            }
            dir.remove_directory(&name)?;
        } else {
            let name = cstring(&name)?;
            check(unsafe { libc::unlinkat(dir.0.as_raw_fd(), name.as_ptr(), 0) })?;
        }
    }
    dir.0.sync_all()?;
    Ok(())
}

pub(super) fn cstring(value: &OsStr) -> Result<CString> {
    if value.as_bytes().is_empty()
        || value.as_bytes().contains(&b'/')
        || value.as_bytes().contains(&b'\\')
        || value == OsStr::new("..")
    {
        return Err(ReleaseStoreError::UnsafeEntry);
    }
    CString::new(value.as_bytes()).map_err(|_| ReleaseStoreError::UnsafeEntry)
}
pub(super) fn directory_flags() -> libc::c_int {
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC
}
pub(super) fn file(fd: libc::c_int) -> Result<File> {
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub(super) fn check(result: libc::c_int) -> Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}
pub(super) fn entry_metadata(dir: &Dir, name: &OsStr) -> Result<libc::stat> {
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
pub(super) fn same_entry(parent: &Dir, name: &OsStr, expected: &Dir) -> Result<bool> {
    let actual = entry_metadata(parent, name)?;
    let metadata: Metadata = expected.0.metadata()?;
    Ok(actual.st_mode & libc::S_IFMT == libc::S_IFDIR
        && actual.st_dev as u64 == metadata.dev()
        && actual.st_ino as u64 == metadata.ino())
}
pub(super) fn publish(root: &Dir, from: &OsStr, to: &OsStr) -> std::io::Result<()> {
    let from = CString::new(from.as_bytes()).expect("generated stage name");
    let to = CString::new(to.as_bytes()).expect("validated digest");
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
pub(super) fn errno_pointer() -> *mut libc::c_int {
    unsafe { libc::__error() }
}
#[cfg(target_os = "linux")]
pub(super) fn errno_pointer() -> *mut libc::c_int {
    unsafe { libc::__errno_location() }
}
pub(super) fn errno() -> libc::c_int {
    unsafe { *errno_pointer() }
}
pub(super) fn set_errno(value: libc::c_int) {
    unsafe {
        *errno_pointer() = value;
    }
}
