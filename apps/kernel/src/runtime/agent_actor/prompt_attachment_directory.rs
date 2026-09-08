//! Create private Unix attachment directories without following parent symlinks.
use std::ffi::CString;
use std::fs::{File, Permissions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path};

pub(super) fn current_uid() -> u32 {
    // SAFETY: geteuid has no pointer arguments or preconditions.
    unsafe { libc::geteuid() }
}

fn make_private(directory: &File, owner: u32) -> io::Result<()> {
    if directory.metadata()?.uid() != owner {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "attachment directory belongs to another user",
        ));
    }
    directory.set_permissions(Permissions::from_mode(0o700))
}

pub(super) fn prepare(root: &Path, temp: &Path) -> io::Result<()> {
    let relative = root
        .strip_prefix(temp)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if relative.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "attachment root cannot be the temp directory",
        ));
    }
    // The process temp directory is the trusted starting point. Hold each
    // parent open while resolving its child, and never chmod a path target.
    let mut parent = File::open(temp)?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid attachment directory component",
            ));
        };
        let name = CString::new(name.as_bytes())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        // SAFETY: parent is live and name is a NUL-terminated single component.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        // SAFETY: same live parent/name; O_NOFOLLOW rejects a substituted link.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a new owned descriptor, transferred once.
        let child = unsafe { File::from_raw_fd(fd) };
        make_private(&child, current_uid())?;
        parent = child;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_owner_is_rejected_before_chmod() {
        let directory = File::open(std::env::temp_dir()).unwrap();
        let mode = directory.metadata().unwrap().permissions().mode();
        let wrong_owner = directory.metadata().unwrap().uid().wrapping_add(1);
        assert_eq!(
            make_private(&directory, wrong_owner).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(directory.metadata().unwrap().permissions().mode(), mode);
    }
}
