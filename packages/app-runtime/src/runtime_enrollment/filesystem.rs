use super::{EnrollmentError, Result, MAX_BUNDLE};
use crate::private_fs::{self, Dir};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Component, Path},
};

pub(super) struct Directory {
    pub file: File,
    uid: u32,
}
impl Directory {
    pub(super) fn open(path: &Path, uid: u32) -> Result<Self> {
        if !path.is_absolute() || path.as_os_str().len() > 1024 {
            return Err(EnrollmentError::Identity);
        }
        let mut directory = Self {
            file: Dir::absolute_root().map_err(fs_error)?.0,
            uid,
        };
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    directory = directory.child(name)?;
                    let metadata = directory.file.metadata()?;
                    if metadata.uid() != 0 && metadata.uid() != uid {
                        return Err(EnrollmentError::Identity);
                    }
                    if metadata.mode() & 0o022 != 0 {
                        return Err(EnrollmentError::Identity);
                    }
                }
                _ => return Err(EnrollmentError::Identity),
            }
        }
        if directory.file.metadata()?.uid() != uid {
            return Err(EnrollmentError::Identity);
        }
        Ok(directory)
    }
    fn child(&self, name: &OsStr) -> Result<Self> {
        let name = private_fs::cstring(name).map_err(fs_error)?;
        let file = private_fs::file(unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                private_fs::directory_flags(),
            )
        })
        .map_err(fs_error)?;
        Ok(Self {
            file,
            uid: self.uid,
        })
    }
    pub(super) fn file(&self, path: &str, mode: Option<u32>) -> Result<File> {
        let components: Vec<_> = Path::new(path).components().collect();
        if components.is_empty() || components.len() > 3 {
            return Err(EnrollmentError::Identity);
        }
        let mut directory = Self {
            file: self.file.try_clone()?,
            uid: self.uid,
        };
        for (index, component) in components.iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err(EnrollmentError::Identity);
            };
            if index + 1 != components.len() {
                directory = directory.child(name)?;
                let metadata = directory.file.metadata()?;
                if metadata.uid() != self.uid || metadata.mode() & 0o022 != 0 {
                    return Err(EnrollmentError::Identity);
                }
                continue;
            }
            let name = private_fs::cstring(name).map_err(fs_error)?;
            let file = private_fs::file(unsafe {
                libc::openat(
                    directory.file.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            })
            .map_err(fs_error)?;
            let metadata = file.metadata()?;
            if !metadata.is_file()
                || metadata.uid() != self.uid
                || metadata.nlink() != 1
                || metadata.mode() & 0o7022 != 0
                || mode.is_some_and(|mode| metadata.mode() & 0o7777 != mode)
            {
                return Err(EnrollmentError::Identity);
            }
            return Ok(file);
        }
        Err(EnrollmentError::Identity)
    }
    pub(super) fn require_inventory(&self, expected: &[String]) -> Result<()> {
        let mut expected: BTreeSet<_> = expected.iter().cloned().collect();
        let mut visited = 0;
        self.walk("", &mut expected, &mut visited)?;
        if !expected.is_empty() {
            return Err(EnrollmentError::Identity);
        }
        Ok(())
    }
    fn walk(
        &self,
        prefix: &str,
        expected: &mut BTreeSet<String>,
        visited: &mut usize,
    ) -> Result<()> {
        if prefix.split('/').filter(|part| !part.is_empty()).count() > 2 {
            return Err(EnrollmentError::Limit);
        }
        let dir = Dir(self.file.try_clone()?);
        for name in dir.entries(48).map_err(fs_error)? {
            *visited += 1;
            if *visited > 48 {
                return Err(EnrollmentError::Limit);
            }
            let path = format!(
                "{prefix}{}",
                name.to_str().ok_or(EnrollmentError::Identity)?
            );
            let stat = private_fs::entry_metadata(&dir, &name).map_err(fs_error)?;
            if stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
                let next = format!("{path}/");
                if !expected.iter().any(|value| value.starts_with(&next)) {
                    return Err(EnrollmentError::Identity);
                }
                let child = self.child(&name)?;
                let metadata = child.file.metadata()?;
                if metadata.uid() != self.uid || metadata.mode() & 0o022 != 0 {
                    return Err(EnrollmentError::Identity);
                }
                child.walk(&next, expected, visited)?;
            } else if stat.st_mode & libc::S_IFMT != libc::S_IFREG || !expected.remove(&path) {
                return Err(EnrollmentError::Identity);
            }
        }
        Ok(())
    }
}
pub(super) fn small(file: &mut File, ceiling: u64) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(0))?;
    if file.metadata()?.len() > ceiling {
        return Err(EnrollmentError::Limit);
    }
    let mut bytes = Vec::new();
    file.take(ceiling + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > ceiling {
        return Err(EnrollmentError::Limit);
    }
    Ok(bytes)
}
pub(super) fn verify_file(file: &mut File, size: u64, digest: &str) -> Result<()> {
    let before = file.metadata()?;
    if size > MAX_BUNDLE || before.len() != size {
        return Err(EnrollmentError::Limit);
    }
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut count = 0_u64;
    let mut buffer = [0; 65536];
    loop {
        let received = file.read(&mut buffer)?;
        if received == 0 {
            break;
        }
        count = count
            .checked_add(received as u64)
            .ok_or(EnrollmentError::Limit)?;
        if count > size {
            return Err(EnrollmentError::Limit);
        }
        hash.update(&buffer[..received]);
    }
    let after = file.metadata()?;
    if count != size
        || format!("{:x}", hash.finalize()) != digest
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err(EnrollmentError::Identity);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}
pub(super) fn shared_lease(file: &File) -> Result<()> {
    if file.metadata()?.len() != 0 {
        return Err(EnrollmentError::Identity);
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return Err(EnrollmentError::Busy);
    }
    Err(error.into())
}
fn fs_error(error: private_fs::FsError) -> EnrollmentError {
    match error {
        private_fs::FsError::Io(error) => EnrollmentError::Io(error),
        private_fs::FsError::EntryLimit => EnrollmentError::Limit,
        private_fs::FsError::UnsafeEntry => EnrollmentError::Identity,
    }
}
