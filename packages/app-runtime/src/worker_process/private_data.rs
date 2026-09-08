//! Private single-file replacement, separate from structured-state transactions.
//! Held descriptors prevent symlink traversal. A staged write keeps the actual
//! process preparation alive through publication or cleanup. Concurrent ordinary
//! App writes retain normal filesystem semantics; this is not a multi-file lock.
use super::{PreparedWorker, WorkerError, WorkerProcess};
use crate::private_fs::{self, Dir};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::Write,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    sync::{Arc, Mutex},
};

pub const MAX_REPLACE_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PrivateDataError {
    #[error("app_file_invalid")]
    Invalid,
    #[error("app_file_identity")]
    Identity,
    #[error("app_file_io")]
    Io,
    #[error("app_file_outcome_uncertain")]
    OutcomeUncertain,
}
type Result<T> = std::result::Result<T, PrivateDataError>;

#[derive(Clone)]
pub struct PrivateData {
    root: Arc<Dir>,
    // Last: close cloned directories before helper storage reclamation.
    preparation: Arc<Mutex<PreparedWorker>>,
    installation: String,
    generation: u64,
    release_digest: String,
}

impl WorkerProcess {
    pub fn private_data(&self) -> std::result::Result<PrivateData, WorkerError> {
        let prepared = self
            ._preparation
            .lock()
            .map_err(|_| WorkerError::Preparation)?;
        let directory = prepared.domain.private_data_directory()?;
        Ok(PrivateData {
            root: Arc::new(Dir(directory)),
            preparation: self._preparation.clone(),
            installation: self.installation.clone(),
            generation: self.generation.parse().map_err(|_| WorkerError::Identity)?,
            release_digest: self.release_digest.clone(),
        })
    }
}

impl PrivateData {
    pub fn installation_id(&self) -> &str {
        &self.installation
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn release_digest(&self) -> &str {
        &self.release_digest
    }

    /// Writes/fsyncs the new inode before entering the kernel's final generation
    /// fence. No destination changes until `publish` consumes this private value.
    pub fn prepare_replace(&self, path: &str, contents: &[u8]) -> Result<PreparedDataReplace> {
        let parts = relative_path(path)?;
        if contents.len() > MAX_REPLACE_BYTES {
            return Err(PrivateDataError::Invalid);
        }
        let device = self.root.0.metadata().map_err(io)?.dev();
        let mut parent = Dir(self.root.0.try_clone().map_err(io)?);
        for name in &parts[..parts.len() - 1] {
            parent = parent.child(OsStr::new(name)).map_err(fs)?;
            if parent.0.metadata().map_err(io)?.dev() != device {
                return Err(PrivateDataError::Identity);
            }
        }
        let destination = OsString::from(parts[parts.len() - 1]);
        require_destination(&parent, &destination)?;
        let mut entropy = [0u8; 32];
        if unsafe { libc::getentropy(entropy.as_mut_ptr().cast(), entropy.len()) } != 0 {
            return Err(PrivateDataError::Io);
        }
        let temporary = OsString::from(format!(
            ".chariox-replace-{}",
            entropy
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ));
        let file = parent.create_private_file(&temporary).map_err(fs)?;
        let mut staged = PreparedDataReplace {
            parent,
            temporary,
            destination,
            file,
            published: false,
            data: self.clone(),
        };
        staged.file.write_all(contents).map_err(io)?;
        staged.file.sync_all().map_err(io)?;
        Ok(staged)
    }
}

pub struct PreparedDataReplace {
    parent: Dir,
    temporary: OsString,
    destination: OsString,
    file: File,
    published: bool,
    // Last so directory/file handles close before releasing preparation.
    data: PrivateData,
}
impl PreparedDataReplace {
    pub fn data(&self) -> &PrivateData {
        &self.data
    }
    /// The kernel must hold its current installation/signer fence here. Once
    /// rename begins, cancellation cannot promise rollback or authorize retry.
    pub fn publish(self) -> Result<()> {
        self.publish_checked(|| Ok(()))
    }
    fn publish_checked(mut self, after_rename: impl FnOnce() -> std::io::Result<()>) -> Result<()> {
        if !same_file(&self.parent, &self.temporary, &self.file)? {
            return Err(PrivateDataError::Identity);
        }
        require_destination(&self.parent, &self.destination)?;
        let source = private_fs::cstring(&self.temporary).map_err(fs)?;
        let destination = private_fs::cstring(&self.destination).map_err(fs)?;
        if unsafe {
            libc::renameat(
                self.parent.0.as_raw_fd(),
                source.as_ptr(),
                self.parent.0.as_raw_fd(),
                destination.as_ptr(),
            )
        } != 0
        {
            return Err(PrivateDataError::OutcomeUncertain);
        }
        self.published = true;
        after_rename().map_err(|_| PrivateDataError::OutcomeUncertain)?;
        self.parent
            .sync()
            .map_err(|_| PrivateDataError::OutcomeUncertain)
    }
}
impl Drop for PreparedDataReplace {
    fn drop(&mut self) {
        if !self.published && same_file(&self.parent, &self.temporary, &self.file).unwrap_or(false)
        {
            let _ = self.parent.remove_file(&self.temporary);
            let _ = self.parent.sync();
        }
    }
}
fn relative_path(path: &str) -> Result<Vec<&str>> {
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\\')
        || path.bytes().any(|b| b < 32 || b == 127)
        || (path.as_bytes().get(1) == Some(&b':') && path.as_bytes()[0].is_ascii_alphabetic())
    {
        return Err(PrivateDataError::Invalid);
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() > 256
        || parts
            .iter()
            .any(|part| matches!(*part, "" | "." | "..") || part.len() > 255)
    {
        return Err(PrivateDataError::Invalid);
    }
    Ok(parts)
}
fn require_destination(parent: &Dir, name: &OsStr) -> Result<()> {
    match private_fs::entry_metadata(parent, name) {
        Ok(metadata) if metadata.st_mode & libc::S_IFMT == libc::S_IFREG => Ok(()),
        Ok(_) => Err(PrivateDataError::Identity),
        Err(private_fs::FsError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(())
        }
        Err(error) => Err(fs(error)),
    }
}
fn same_file(parent: &Dir, name: &OsStr, expected: &File) -> Result<bool> {
    let entry = private_fs::entry_metadata(parent, name).map_err(fs)?;
    let metadata = expected.metadata().map_err(io)?;
    Ok(entry.st_mode & libc::S_IFMT == libc::S_IFREG
        && entry.st_dev as u64 == metadata.dev()
        && entry.st_ino as u64 == metadata.ino())
}
fn io(_: std::io::Error) -> PrivateDataError {
    PrivateDataError::Io
}
fn fs(error: private_fs::FsError) -> PrivateDataError {
    match error {
        private_fs::FsError::Io(_) => PrivateDataError::Io,
        _ => PrivateDataError::Identity,
    }
}

#[cfg(test)]
mod tests;
