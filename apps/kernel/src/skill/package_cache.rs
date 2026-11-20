//! MP-08 / MP-10 / MP-11: serialize verified repair of execution-owned caches.
use super::failure;
use crate::error::DaemonError;
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

pub(super) fn lock(parent: &Path) -> Result<File, DaemonError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(parent.join(".materialization.lock"))
        .map_err(|_| failure("cannot open skill materialization lock"))?;
    let metadata = file
        .metadata()
        .map_err(|_| failure("cannot inspect skill materialization lock"))?;
    if !metadata.is_file() || !owned(&metadata) {
        return Err(failure(
            "skill materialization lock must be an owned regular file",
        ));
    }
    fs2::FileExt::lock_exclusive(&file)
        .map_err(|_| failure("cannot lock skill materialization"))?;
    Ok(file)
}

pub(super) fn owned(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.uid() == unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        true
    }
}

/// Only a fully verified replacement may reach this call. Do not follow links
/// or remove a foreign-owned tree. Readers reject corruption while staging;
/// a crash after removal leaves a miss which the same verified resend repairs.
pub(super) fn remove_damaged(root: &Path) -> Result<(), DaemonError> {
    let metadata =
        fs::symlink_metadata(root).map_err(|_| failure("cannot inspect damaged skill cache"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || !owned(&metadata) {
        return Err(failure("damaged skill cache is not an owned directory"));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut remaining = 4096usize;
    while let Some(directory) = pending.pop() {
        for entry in
            fs::read_dir(directory).map_err(|_| failure("cannot inspect damaged skill cache"))?
        {
            let entry = entry.map_err(|_| failure("cannot inspect damaged skill cache"))?;
            remaining = remaining
                .checked_sub(1)
                .ok_or_else(|| failure("damaged skill cache exceeds safe repair bounds"))?;
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|_| failure("cannot inspect damaged skill cache"))?;
            if !owned(&metadata) {
                return Err(failure("damaged skill cache contains foreign-owned files"));
            }
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                pending.push(entry.path());
            } else if !metadata.is_file() && !metadata.file_type().is_symlink() {
                return Err(failure("damaged skill cache contains a special file"));
            }
        }
    }
    fs::remove_dir_all(root).map_err(|_| failure("cannot remove damaged skill cache"))
}
