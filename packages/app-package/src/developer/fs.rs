//! Descriptor-relative developer file I/O. Never follows a path component or
//! accepts special/hard-linked files. No package entry is extracted here.

use std::{
    ffi::{CStr, CString, OsStr},
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};

use crate::{ErrorCode, PackageError, Result};
use rand::{rngs::OsRng, RngCore};

pub(crate) fn io_error() -> PackageError {
    PackageError::new(ErrorCode::Io, "developer file operation failed")
}

fn invalid_path() -> PackageError {
    PackageError::new(
        ErrorCode::InvalidPath,
        "path must not contain links, parent components, or special files",
    )
}

fn open_error() -> PackageError {
    if matches!(errno(), libc::ELOOP | libc::ENOTDIR) {
        invalid_path()
    } else {
        io_error()
    }
}

fn name(value: &OsStr) -> Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| invalid_path())
}

pub(crate) struct Directory(File);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Identity {
    pub device: u64,
    pub inode: u64,
}

pub(crate) fn identity(file: &File) -> Result<Identity> {
    let metadata = file.metadata().map_err(|_| io_error())?;
    Ok(Identity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

impl Directory {
    fn require_owned_output(&self) -> Result<()> {
        let metadata = self.0.metadata().map_err(|_| io_error())?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
            return Err(PackageError::new(ErrorCode::InvalidPath, "output directory must be owned by the current user and not writable by group or others"));
        }
        Ok(())
    }
    pub(crate) fn require_private(&self) -> Result<()> {
        let metadata = self.0.metadata().map_err(|_| io_error())?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(PackageError::new(
                ErrorCode::InvalidDeveloperKey,
                "signing key directory must be owned by the current user and private (0700)",
            ));
        }
        Ok(())
    }
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let start = if path.is_absolute() { c"/" } else { c"." };
        let fd = unsafe {
            libc::open(
                start.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(io_error());
        }
        let mut directory = Self(unsafe { File::from_raw_fd(fd) });
        for component in path.components() {
            match component {
                Component::RootDir | Component::CurDir => {}
                Component::Normal(value) => directory = directory.child(&name(value)?)?,
                _ => return Err(invalid_path()),
            }
        }
        Ok(directory)
    }

    fn child(&self, name: &CStr) -> Result<Self> {
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(open_error());
        }
        Ok(Self(unsafe { File::from_raw_fd(fd) }))
    }

    pub(crate) fn names(&self, maximum: usize) -> Result<Vec<CString>> {
        // openat(".") creates a distinct directory offset; fdopendir owns it.
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY,
            )
        };
        if fd < 0 {
            return Err(io_error());
        }
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err(io_error());
        }
        struct Stream(*mut libc::DIR);
        impl Drop for Stream {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let stream = Stream(stream);
        let mut names = Vec::new();
        loop {
            // POSIX distinguishes EOF from readdir failure through errno.
            set_errno(0);
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                if errno() != 0 {
                    return Err(io_error());
                }
                break;
            }
            let item = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if item.to_bytes() == b"." || item.to_bytes() == b".." {
                continue;
            }
            if names.len() >= maximum {
                return Err(PackageError::new(
                    ErrorCode::ArchiveLimit,
                    "bundle directory exceeds entry limit",
                ));
            }
            names.push(item.to_owned());
        }
        names.sort();
        Ok(names)
    }

    pub(crate) fn entry(&self, name: &CStr) -> Result<Entry> {
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(open_error());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata().map_err(|_| io_error())?;
        if metadata.is_dir() {
            Ok(Entry::Directory(Self(file)))
        } else if metadata.is_file() && metadata.nlink() == 1 {
            Ok(Entry::File(file))
        } else {
            Err(invalid_path())
        }
    }

    pub(crate) fn create(&self, name: &CStr) -> Result<File> {
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub(crate) fn remove(&self, name: &CStr) -> bool {
        unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) == 0 }
    }
    pub(crate) fn sync(&self) -> Result<()> {
        self.0.sync_all().map_err(|_| io_error())
    }
}

#[cfg(target_os = "linux")]
fn errno() -> i32 {
    unsafe { *libc::__errno_location() }
}
#[cfg(target_os = "linux")]
fn set_errno(value: i32) {
    unsafe {
        *libc::__errno_location() = value;
    }
}
#[cfg(target_os = "macos")]
fn errno() -> i32 {
    unsafe { *libc::__error() }
}
#[cfg(target_os = "macos")]
fn set_errno(value: i32) {
    unsafe {
        *libc::__error() = value;
    }
}

pub(crate) enum Entry {
    Directory(Directory),
    File(File),
}

pub(crate) fn parent(path: &Path) -> Result<(Directory, CString)> {
    let filename = path.file_name().ok_or_else(invalid_path)?;
    let directory = Directory::open(path.parent().unwrap_or_else(|| Path::new(".")))?;
    Ok((directory, name(filename)?))
}

pub(crate) fn read(path: &Path, maximum: usize, private: bool) -> Result<Vec<u8>> {
    let (_, file) = open_file(path, private)?;
    read_open(file, maximum)
}

pub(crate) fn open_file(path: &Path, private: bool) -> Result<(Directory, File)> {
    let (directory, name) = parent(path)?;
    let Entry::File(file) = directory.entry(&name)? else {
        return Err(invalid_path());
    };
    if private {
        directory.require_private()?;
        let metadata = file.metadata().map_err(|_| io_error())?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(PackageError::new(
                ErrorCode::InvalidDeveloperKey,
                "private signing key must be owned by the current user with mode 0600",
            ));
        }
    }
    Ok((directory, file))
}

pub(crate) fn read_open(mut file: File, maximum: usize) -> Result<Vec<u8>> {
    let before = file.metadata().map_err(|_| io_error())?;
    if before.len() > maximum as u64 {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "input exceeds byte limit",
        ));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| io_error())?;
    let after = file.metadata().map_err(|_| io_error())?;
    if bytes.len() > maximum
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || bytes.len() as u64 != after.len()
    {
        return Err(PackageError::new(
            ErrorCode::Io,
            "input changed while reading or exceeded byte limit",
        ));
    }
    Ok(bytes)
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    Destination::open(path, false)?.prepare(bytes)?.publish()
}

pub(crate) struct Destination {
    pub directory: Directory,
    name: CString,
}

impl Destination {
    pub(crate) fn open(path: &Path, private: bool) -> Result<Self> {
        let (directory, name) = parent(path)?;
        directory.require_owned_output()?;
        if private {
            directory.require_private()?;
        }
        Ok(Self { directory, name })
    }

    pub(crate) fn prepare(self, bytes: &[u8]) -> Result<PreparedFile> {
        let Self { directory, name } = self;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                directory.0.as_raw_fd(),
                name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
            || errno() != libc::ENOENT
        {
            return Err(io_error());
        }
        let mut nonce = [0u8; 16];
        OsRng.try_fill_bytes(&mut nonce).map_err(|_| io_error())?;
        let temporary = CString::new(format!(".cxapp-write-{:032x}", u128::from_le_bytes(nonce)))
            .expect("hex filename has no NUL");
        let file = directory.create(&temporary)?;
        let mut pending = PreparedFile {
            directory,
            name,
            temporary,
            temporary_exists: true,
            file,
        };
        if unsafe { libc::fchmod(pending.file.as_raw_fd(), 0o600) } != 0 {
            return Err(io_error());
        }
        pending
            .file
            .write_all(bytes)
            .and_then(|()| pending.file.sync_all())
            .map_err(|_| io_error())?;
        Ok(pending)
    }
}

/// A synced private temporary inode, atomically renamed without replacement.
/// The inode stays single-linked across a crash. Held FDs and identity checks
/// prevent cleanup/publication of a different temporary-name occupant.
pub(crate) struct PreparedFile {
    directory: Directory,
    name: CString,
    temporary: CString,
    temporary_exists: bool,
    file: File,
}

impl PreparedFile {
    pub(crate) fn new(path: &Path, bytes: &[u8], private: bool) -> Result<Self> {
        Destination::open(path, private)?.prepare(bytes)
    }

    pub(crate) fn same_destination(&self, other: &Self) -> Result<bool> {
        let first = self.directory.0.metadata().map_err(|_| io_error())?;
        let second = other.directory.0.metadata().map_err(|_| io_error())?;
        Ok(first.dev() == second.dev() && first.ino() == second.ino() && self.name == other.name)
    }

    pub(crate) fn publish(mut self) -> Result<()> {
        if !self.matches_temporary() {
            return Err(io_error());
        }
        let fd = self.directory.0.as_raw_fd();
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                fd,
                self.temporary.as_ptr(),
                fd,
                self.name.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                fd,
                self.temporary.as_ptr(),
                fd,
                self.name.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        if result != 0 {
            return Err(io_error());
        }
        self.temporary_exists = false;
        self.directory.sync()
    }

    fn matches_temporary(&self) -> bool {
        let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.directory.0.as_raw_fd(),
                self.temporary.as_ptr(),
                current.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return false;
        }
        let current = unsafe { current.assume_init() };
        let Ok(expected) = identity(&self.file) else {
            return false;
        };
        current.st_dev as u64 == expected.device
            && current.st_ino as u64 == expected.inode
            && current.st_mode & libc::S_IFMT == libc::S_IFREG
    }
}

impl Drop for PreparedFile {
    fn drop(&mut self) {
        if self.temporary_exists && self.matches_temporary() {
            self.directory.remove(&self.temporary);
        }
    }
}

pub(crate) fn outside_bundle(bundle: &Directory, directory: &Directory) -> Result<()> {
    let bundle_identity = identity(&bundle.0)?;
    let mut current = directory.child(c".")?;
    // Compare directory identities rather than path strings: macOS case aliases
    // must not let a private key or output appear inside its own input bundle.
    loop {
        let current_identity = identity(&current.0)?;
        if current_identity == bundle_identity {
            return Err(PackageError::new(
                ErrorCode::InvalidPath,
                "signing keys and package outputs must be outside the build bundle",
            ));
        }
        let ancestor = current.child(c"..")?;
        if identity(&ancestor.0)? == current_identity {
            break;
        }
        current = ancestor;
    }
    Ok(())
}

/// Writes a fixed, bounded source scaffold beneath a newly created directory.
/// Every descendant is reached through held descriptors. Existing destinations
/// are never opened for writing or adopted, even when they are empty.
pub(crate) fn create_source_tree(
    destination: &Path,
    files: &std::collections::BTreeMap<&str, Vec<u8>>,
) -> Result<()> {
    let (parent, leaf) = parent(destination)?;
    parent.require_owned_output()?;
    if unsafe { libc::mkdirat(parent.0.as_raw_fd(), leaf.as_ptr(), 0o700) } != 0 {
        return Err(PackageError::new(
            ErrorCode::Io,
            "scaffold destination must not exist and its parent must be writable",
        ));
    }
    let root = parent.child(&leaf)?;
    let mut directories = std::collections::BTreeMap::new();
    directories.insert(String::new(), root);
    for (path, bytes) in files {
        let mut prefix = String::new();
        let mut components = path.split('/').peekable();
        while let Some(component) = components.next() {
            if component.is_empty() || matches!(component, "." | "..") {
                return Err(invalid_path());
            }
            let encoded = name(OsStr::new(component))?;
            let current = directories.get(&prefix).ok_or_else(io_error)?;
            if components.peek().is_none() {
                let mut output = current.create(&encoded)?;
                output
                    .write_all(bytes)
                    .and_then(|_| output.sync_all())
                    .map_err(|_| io_error())?;
            } else {
                let next = if prefix.is_empty() {
                    component.into()
                } else {
                    format!("{prefix}/{component}")
                };
                if !directories.contains_key(&next) {
                    if unsafe { libc::mkdirat(current.0.as_raw_fd(), encoded.as_ptr(), 0o700) } != 0
                    {
                        return Err(io_error());
                    }
                    let child = current.child(&encoded)?;
                    directories.insert(next.clone(), child);
                }
                prefix = next;
            }
        }
    }
    for directory in directories.values().rev() {
        directory.sync()?;
    }
    parent.sync()
}
