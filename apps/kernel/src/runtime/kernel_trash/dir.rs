//! Directory handles for the Trash: every step below the first opened directory goes through a handle
//! (`openat`, `mkdirat`, `renameat2`, `unlinkat`) and never follows a link, so a path that another user
//! swaps for a link after it was checked cannot send a move or a restore somewhere else.
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

const DIRECTORY: libc::c_int =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

pub(super) struct Dir(OwnedFd);

fn name_of(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes()).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
}

fn checked(rc: libc::c_int) -> io::Result<libc::c_int> {
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc)
    }
}

impl Dir {
    /// The folder at `path`, which is not itself a link (links above it are followed: callers pass
    /// a canonical path).
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let path = name_of(path.as_os_str())?;
        // SAFETY: `path` is a NUL-terminated string that lives through the call.
        let fd = checked(unsafe { libc::open(path.as_ptr(), DIRECTORY) })?;
        // SAFETY: `fd` is a new descriptor that nothing else owns.
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    /// The folder `name` in this one; a link or a file there is refused.
    pub(super) fn open_at(&self, name: &OsStr) -> io::Result<Self> {
        let name = name_of(name)?;
        // SAFETY: `name` is a NUL-terminated string that lives through the call.
        let fd = checked(unsafe { libc::openat(self.0.as_raw_fd(), name.as_ptr(), DIRECTORY) })?;
        // SAFETY: `fd` is a new descriptor that nothing else owns.
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    /// Makes the folder `name` (mode `mode`); one that exists already is fine.
    pub(super) fn make_at(&self, name: &OsStr, mode: u32) -> io::Result<()> {
        let name = name_of(name)?;
        // SAFETY: `name` is a NUL-terminated string that lives through the call.
        let rc = unsafe { libc::mkdirat(self.0.as_raw_fd(), name.as_ptr(), mode as libc::mode_t) };
        match checked(rc) {
            Err(error) if error.raw_os_error() != Some(libc::EEXIST) => Err(error),
            _ => Ok(()),
        }
    }

    pub(super) fn stat(&self) -> io::Result<libc::stat> {
        // SAFETY: an all-zero `stat` is a valid value for the call to fill.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: the descriptor is open and `stat` is a valid out pointer.
        checked(unsafe { libc::fstat(self.0.as_raw_fd(), &mut stat) })?;
        Ok(stat)
    }

    /// What `name` is itself (a link is not followed).
    pub(super) fn stat_at(&self, name: &OsStr) -> io::Result<libc::stat> {
        let name = name_of(name)?;
        // SAFETY: an all-zero `stat` is a valid value for the call to fill.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: the descriptor is open, `name` is NUL-terminated and `stat` is a valid out pointer.
        let rc = unsafe {
            libc::fstatat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        checked(rc)?;
        Ok(stat)
    }

    /// A new private file `name`; one that exists (or is a link) is refused.
    pub(super) fn create_at(&self, name: &OsStr) -> io::Result<File> {
        let name = name_of(name)?;
        let flags =
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: `name` is a NUL-terminated string that lives through the call.
        let fd = checked(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                flags,
                0o600 as libc::c_uint,
            )
        })?;
        // SAFETY: `fd` is a new descriptor that nothing else owns.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    /// The text of the plain file `name`, which must belong to `uid`; a link is refused.
    pub(super) fn read_at(&self, name: &OsStr, uid: u32) -> io::Result<String> {
        use std::io::Read as _;
        let name = name_of(name)?;
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: `name` is a NUL-terminated string that lives through the call.
        let fd = checked(unsafe { libc::openat(self.0.as_raw_fd(), name.as_ptr(), flags) })?;
        // SAFETY: `fd` is a new descriptor that nothing else owns.
        let mut file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata()?;
        if !metadata.is_file() || std::os::unix::fs::MetadataExt::uid(&metadata) != uid {
            return Err(io::Error::from_raw_os_error(libc::EPERM));
        }
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        Ok(text)
    }

    pub(super) fn unlink_at(&self, name: &OsStr) -> io::Result<()> {
        let name = name_of(name)?;
        // SAFETY: `name` is a NUL-terminated string that lives through the call.
        checked(unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) }).map(drop)
    }

    /// Whether this folder is `uid`'s alone: owned by `uid`, and closed to everyone else (a folder of
    /// `uid` that is open to others is closed first). A folder of another owner is never used.
    pub(super) fn is_private_to(&self, uid: u32) -> io::Result<bool> {
        let stat = self.stat()?;
        if stat.st_uid != uid {
            return Ok(false);
        }
        if stat.st_mode & 0o777 != 0o700 {
            // SAFETY: the descriptor is open.
            checked(unsafe { libc::fchmod(self.0.as_raw_fd(), 0o700) })?;
        }
        Ok(true)
    }

    /// Moves `name` of this folder to `to` as `new_name`; fails with `EEXIST` when `new_name` is taken.
    pub(super) fn move_into(&self, name: &OsStr, to: &Self, new_name: &OsStr) -> io::Result<()> {
        let (from, target) = (name_of(name)?, name_of(new_name)?);
        // SAFETY: both names are NUL-terminated strings that live through the call.
        let rc = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                self.0.as_raw_fd(),
                from.as_ptr(),
                to.0.as_raw_fd(),
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}
