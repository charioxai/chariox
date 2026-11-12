//! A transfer's private staging directory, pinned from pruning through publication and cleanup.
use std::collections::HashSet;
use std::ffi::{CStr, CString, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) struct Stage {
    pub path: PathBuf,
    directory: File,
    parent: File,
    name: CString,
}

impl Stage {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let result = (|| {
            let parent_path = path
                .parent()
                .ok_or_else(|| io::Error::other("missing parent"))?;
            std::fs::create_dir_all(parent_path)?;
            let parent = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(parent_path)?;
            let name = name(
                path.file_name()
                    .ok_or_else(|| io::Error::other("missing name"))?,
            )?;
            // SAFETY: parent and name remain valid for these descriptor-relative calls.
            if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0
                && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
            {
                return Err(io::Error::last_os_error());
            }
            let directory = child(&parent, &name)?;
            private(&directory)?;
            Ok(Self {
                path: path.clone(),
                directory,
                parent,
                name,
            })
        })();
        result.map_err(|error: io::Error| format!("The staging folder `{}` is not a private directory owned by this kernel's user: {error}. Remove it or choose another destination", path.display()))
    }

    pub fn check(&self) -> Result<(), String> {
        self.check_name().and_then(|()| {
            let current = std::fs::symlink_metadata(&self.path)?;
            let opened = self.directory.metadata()?;
            if current.is_dir() && current.dev() == opened.dev() && current.ino() == opened.ino() {
                Ok(())
            } else { Err(io::Error::other("the staging folder was replaced")) }
        }).map_err(|error| format!("The staging folder `{}` changed during the transfer: {error}; nothing was published", self.path.display()))
    }

    fn check_name(&self) -> io::Result<()> {
        let current = child(&self.parent, &self.name)?.metadata()?;
        let opened = self.directory.metadata()?;
        if current.dev() != opened.dev() || current.ino() != opened.ino() {
            return Err(io::Error::other("the staging folder was replaced"));
        }
        Ok(())
    }

    pub fn prune(
        &self,
        folders: &HashSet<&str>,
        files: &HashSet<&str>,
        sidecars: &HashSet<String>,
    ) -> Result<(), String> {
        self.check()?;
        prune(&self.directory, "", folders, files, sidecars).map_err(|error| {
            format!(
                "The staged copy in `{}` could not be checked: {error}",
                self.path.display()
            )
        })?;
        self.check()
    }

    pub fn cleanup(&self) -> Result<(), String> {
        self.check()?;
        prune(
            &self.directory,
            "",
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .and_then(|()| self.check_name())
        .and_then(|()| unlink(&self.parent, &self.name, true))
        .map_err(|error| format!("The staging folder could not be removed: {error}"))
    }

    pub fn restore_mode(&self, mode: Option<u32>) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        if let Some(mode) = mode {
            self.directory
                .set_permissions(std::fs::Permissions::from_mode(mode & 0o7777))
                .map_err(|error| {
                    format!("The copied folder's permissions could not be restored: {error}")
                })?;
        }
        Ok(())
    }

    pub fn is_published_at(&self, path: &Path) -> bool {
        let Ok(current) = std::fs::symlink_metadata(path) else {
            return false;
        };
        self.directory.metadata().is_ok_and(|opened| {
            current.is_dir() && current.dev() == opened.dev() && current.ino() == opened.ino()
        })
    }
}

fn name(name: &std::ffi::OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes()).map_err(|_| io::Error::other("invalid file name"))
}

fn child(parent: &File, name: &CStr) -> io::Result<File> {
    // SAFETY: both arguments stay valid; a successful call transfers one new descriptor.
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
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn private(directory: &File) -> io::Result<()> {
    let metadata = directory.metadata()?;
    // SAFETY: geteuid has no preconditions.
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::other(
            "the directory has another owner or permits other users",
        ));
    }
    Ok(())
}

struct Listing(*mut libc::DIR);
impl Drop for Listing {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}

fn entries(directory: &File) -> io::Result<Vec<OsString>> {
    let dot = c".";
    let opened = child(directory, dot)?;
    use std::os::fd::IntoRawFd;
    let fd = opened.into_raw_fd();
    // SAFETY: fdopendir takes ownership of this fresh descriptor on success.
    let listing = unsafe { libc::fdopendir(fd) };
    if listing.is_null() {
        let error = io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        return Err(error);
    }
    let listing = Listing(listing);
    let mut found = Vec::new();
    loop {
        // SAFETY: listing is owned here, and d_name is copied before the next readdir call.
        unsafe {
            *errno() = 0;
        }
        let entry = unsafe { libc::readdir(listing.0) };
        if entry.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(0) {
                return Err(error);
            }
            break;
        }
        let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes != b"." && bytes != b".." {
            found.push(std::ffi::OsStr::from_bytes(bytes).to_owned());
        }
    }
    Ok(found)
}

fn unlink(parent: &File, name: &CStr, directory: bool) -> io::Result<()> {
    // SAFETY: the descriptor and name remain valid; unlinkat never follows a final symlink.
    if unsafe {
        libc::unlinkat(
            parent.as_raw_fd(),
            name.as_ptr(),
            if directory { libc::AT_REMOVEDIR } else { 0 },
        )
    } == 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn prune(
    directory: &File,
    relative: &str,
    folders: &HashSet<&str>,
    files: &HashSet<&str>,
    sidecars: &HashSet<String>,
) -> io::Result<()> {
    for entry in entries(directory)? {
        let name = name(&entry)?;
        let below = if relative.is_empty() {
            entry.to_string_lossy().into_owned()
        } else {
            format!("{relative}/{}", entry.to_string_lossy())
        };
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: fstatat fills metadata on success; AT_SYMLINK_NOFOLLOW prevents link traversal.
        if unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let metadata = unsafe { metadata.assume_init() };
        let kind = metadata.st_mode & libc::S_IFMT;
        let is_dir = kind == libc::S_IFDIR;
        let wanted = if entry.as_bytes().starts_with(b".chariox-copy-") {
            kind == libc::S_IFREG && sidecars.contains(&below)
        } else if is_dir {
            folders.contains(below.as_str())
        } else {
            kind == libc::S_IFREG && files.contains(below.as_str())
        };
        if is_dir {
            let opened = child(directory, &name)?;
            if wanted {
                prune(&opened, &below, folders, files, sidecars)?;
            } else {
                prune(
                    &opened,
                    "",
                    &HashSet::new(),
                    &HashSet::new(),
                    &HashSet::new(),
                )?;
                unlink(directory, &name, true)?;
            }
        } else if !wanted {
            unlink(directory, &name, false)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
unsafe fn errno() -> *mut libc::c_int {
    libc::__errno_location()
}
#[cfg(target_os = "macos")]
unsafe fn errno() -> *mut libc::c_int {
    libc::__error()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l9_owned_stage_cleanup_removes_folders_with_restored_modes() {
        use std::os::unix::fs::PermissionsExt;
        let root = crate::test_support::TestWorktree::new("stage-cleanup-modes");
        let path = root.path().join("stage");
        let stage = Stage::open(path.clone()).unwrap();
        std::fs::create_dir(path.join("sub")).unwrap();
        std::fs::write(path.join("sub/file"), b"staged bytes").unwrap();
        std::fs::set_permissions(path.join("sub"), std::fs::Permissions::from_mode(0o755)).unwrap();
        stage.cleanup().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn l9_replacing_a_valid_stage_root_never_redirects_pruning_or_cleanup() {
        let root = crate::test_support::TestWorktree::new("stage-identity");
        let path = root.path().join("stage");
        let stage = Stage::open(path.clone()).unwrap();
        std::fs::write(path.join("stale"), b"staged bytes").unwrap();
        let held = root.path().join("moved-stage");
        std::fs::rename(&path, &held).unwrap();
        let victim = root.path().join("victim");
        std::fs::create_dir(&victim).unwrap();
        std::fs::write(victim.join("keep"), b"original bytes").unwrap();
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        assert!(stage
            .prune(&HashSet::new(), &HashSet::new(), &HashSet::new())
            .is_err());
        assert!(stage.cleanup().is_err());
        assert!(!stage.is_published_at(&path));
        assert!(stage.is_published_at(&held));
        assert_eq!(
            std::fs::read(victim.join("keep")).unwrap(),
            b"original bytes"
        );
        assert_eq!(std::fs::read(held.join("stale")).unwrap(), b"staged bytes");
    }
}
