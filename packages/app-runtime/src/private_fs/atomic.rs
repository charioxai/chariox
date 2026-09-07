//! Complete metadata replacement inside one already-open kernel directory.
use super::{check, cstring, entry_metadata, try_lock_file, Dir, FsError, Result};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_REPLACEMENT: AtomicU64 = AtomicU64::new(0);

impl Dir {
    /// Replaces one private regular metadata file atomically, or creates it.
    /// Callers bound the payload and serialize their state transitions. An I/O
    /// error after rename has an unknown durability outcome; recover by reading
    /// and validating state rather than assuming the old value survived.
    pub(crate) fn atomic_replace(&self, name: &OsStr, bytes: &[u8]) -> Result<()> {
        let destination = cstring(name)?;
        if name == OsStr::new(".") || is_atomic_temporary(name) {
            return Err(FsError::UnsafeEntry);
        }
        match self.open_private_file(name) {
            Ok(_) => {}
            Err(FsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut temporary = TemporaryFile::create(self)?;
        temporary.file.write_all(bytes)?;
        temporary.file.sync_all()?;
        if !same_file(self, &temporary.name, &temporary.file)? {
            return Err(FsError::UnsafeEntry);
        }
        let source = cstring(&temporary.name)?;
        check(unsafe {
            libc::renameat(
                self.0.as_raw_fd(),
                source.as_ptr(),
                self.0.as_raw_fd(),
                destination.as_ptr(),
            )
        })?;
        temporary.committed = true;
        self.sync()
    }
}

pub(crate) fn is_atomic_temporary(name: &OsStr) -> bool {
    let Some(name) = name.to_str().filter(|name| name.len() < 128) else {
        return false;
    };
    let Some(tail) = name.strip_prefix(".replace-") else {
        return false;
    };
    let fields: Vec<_> = tail.split('-').collect();
    fields.len() == 3
        && fields.iter().all(|field| !field.is_empty())
        && fields[..2]
            .iter()
            .all(|field| field.bytes().all(|byte| byte.is_ascii_digit()))
        && fields[2].bytes().all(|byte| byte.is_ascii_hexdigit())
}

struct TemporaryFile<'a> {
    dir: &'a Dir,
    name: OsString,
    file: File,
    committed: bool,
}
impl<'a> TemporaryFile<'a> {
    fn create(dir: &'a Dir) -> Result<Self> {
        for _ in 0..32 {
            let name = OsString::from(format!(
                ".replace-{}-{}-{:x}",
                std::process::id(),
                NEXT_REPLACEMENT.fetch_add(1, Ordering::Relaxed),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            match dir.create_private_file(&name) {
                Ok(file) => {
                    let temporary = Self {
                        dir,
                        name,
                        file,
                        committed: false,
                    };
                    if !try_lock_file(&temporary.file)? {
                        return Err(FsError::UnsafeEntry);
                    }
                    return Ok(temporary);
                }
                Err(FsError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    continue
                }
                Err(error) => return Err(error),
            }
        }
        Err(FsError::UnsafeEntry)
    }
}
impl Drop for TemporaryFile<'_> {
    fn drop(&mut self) {
        if !self.committed && same_file(self.dir, &self.name, &self.file).unwrap_or(false) {
            let _ = self.dir.remove_file(&self.name);
            let _ = self.dir.sync();
        }
    }
}

fn same_file(dir: &Dir, name: &OsStr, expected: &File) -> Result<bool> {
    let entry = entry_metadata(dir, name)?;
    let metadata = expected.metadata()?;
    Ok(entry.st_mode & libc::S_IFMT == libc::S_IFREG
        && entry.st_dev as u64 == metadata.dev()
        && entry.st_ino as u64 == metadata.ino())
}
