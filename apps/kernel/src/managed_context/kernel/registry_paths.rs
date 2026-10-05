//! Ordinary registry operations relative to retained, no-follow directory descriptors.
use super::*;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

#[cfg(unix)]
pub(super) struct RegistryDirectory(File);

#[cfg(unix)]
impl RegistryDirectory {
    pub(super) fn root(path: &Path) -> Result<Self, DaemonError> {
        if !path.is_absolute() {
            return Err(import_error("ordinary registry root is not absolute"));
        }
        let file = File::open("/").map_err(|e| import_io_error("open registry root", e))?;
        let mut directory = Self(file);
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => directory = directory.child(name, true)?,
                _ => return Err(import_error("ordinary registry root contains traversal")),
            }
        }
        Ok(directory)
    }

    fn child(&self, name: &std::ffi::OsStr, create: bool) -> Result<Self, DaemonError> {
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| import_error("registry name contains NUL"))?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let mut fd = unsafe { libc::openat(self.0.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 && create && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
            let result = unsafe { libc::mkdirat(self.0.as_raw_fd(), name.as_ptr(), 0o700) };
            if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(import_io_error(
                    "create ordinary registry directory",
                    io::Error::last_os_error(),
                ));
            }
            fd = unsafe { libc::openat(self.0.as_raw_fd(), name.as_ptr(), flags) };
        }
        if fd < 0 {
            return Err(import_io_error(
                "open no-follow ordinary registry directory",
                io::Error::last_os_error(),
            ));
        }
        Ok(Self(unsafe { File::from_raw_fd(fd) }))
    }

    pub(super) fn parent(&self, relative: &str) -> Result<(Self, std::ffi::CString), DaemonError> {
        validate_portable_package_path(relative)?;
        let path = Path::new(relative);
        let mut directory = Self(
            self.0
                .try_clone()
                .map_err(|e| import_io_error("retain registry directory", e))?,
        );
        for component in path.parent().unwrap_or(Path::new("")).components() {
            let Component::Normal(name) = component else {
                return Err(import_error("registry path contains traversal"));
            };
            directory = directory.child(name, true)?;
        }
        let name = std::ffi::CString::new(
            path.file_name()
                .ok_or_else(|| import_error("registry entry has no name"))?
                .as_bytes(),
        )
        .map_err(|_| import_error("registry entry contains NUL"))?;
        Ok((directory, name))
    }

    pub(super) fn absent(&self, name: &std::ffi::CStr) -> Result<bool, DaemonError> {
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            libc::fstatat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == 0 {
            return Ok(false);
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::NotFound {
            Ok(true)
        } else {
            Err(import_io_error("inspect ordinary registry entry", error))
        }
    }

    pub(super) fn publish(&self, source: &Path, name: &std::ffi::CStr) -> Result<(), DaemonError> {
        let source = std::ffi::CString::new(source.as_os_str().as_bytes())
            .map_err(|_| import_error("staging path contains NUL"))?;
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                libc::AT_FDCWD,
                source.as_ptr(),
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return Err(import_error(
            "descriptor-relative registry publication is unsupported",
        ));
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if result != 0 {
            return Err(import_io_error(
                "publish ordinary registry entry",
                io::Error::last_os_error(),
            ));
        }
        self.0
            .sync_all()
            .map_err(|e| import_io_error("sync ordinary registry", e))
    }

    pub(super) fn remove(&self, name: &std::ffi::CStr) -> Result<(), DaemonError> {
        // Opening a child never follows a link. Unlinking a non-directory removes only its entry.
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd >= 0 {
            let directory = Self(unsafe { File::from_raw_fd(fd) });
            let iter_fd = unsafe { libc::dup(directory.0.as_raw_fd()) };
            if iter_fd < 0 {
                return Err(import_io_error(
                    "duplicate registry directory",
                    io::Error::last_os_error(),
                ));
            }
            let stream = unsafe { libc::fdopendir(iter_fd) };
            if stream.is_null() {
                unsafe {
                    libc::close(iter_fd);
                }
                return Err(import_io_error(
                    "read registry directory",
                    io::Error::last_os_error(),
                ));
            }
            let result = (|| {
                loop {
                    let entry = unsafe { libc::readdir(stream) };
                    if entry.is_null() {
                        break;
                    }
                    let child = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
                    if child.to_bytes() == b"." || child.to_bytes() == b".." {
                        continue;
                    }
                    directory.remove(child)?;
                }
                Ok(())
            })();
            unsafe {
                libc::closedir(stream);
            }
            result?;
        } else {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(());
            }
            if !matches!(error.raw_os_error(), Some(libc::ENOTDIR | libc::ELOOP)) {
                return Err(import_io_error("inspect registry removal", error));
            }
        }
        let flags = if fd >= 0 { libc::AT_REMOVEDIR } else { 0 };
        let result = unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), flags) };
        if result != 0 {
            return Err(import_io_error(
                "remove ordinary registry entry",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }
}
