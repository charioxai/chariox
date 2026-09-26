//! Whole-tree copies of an installation's private data, for App snapshots.
//! Both directions walk held directory descriptors on one device, copy only
//! regular single-link files and directories, and never follow a symlink.
use super::{fs, io, PrivateData, PrivateDataError, Result};
use crate::private_fs::{self, Dir};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::Path,
};

#[derive(Debug, Clone, Copy)]
pub struct TreeLimits {
    pub files: usize,
    pub bytes: u64,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeCopy {
    pub files: Vec<TreeFile>,
    pub bytes: u64,
    /// Entries left out: links, special files, hard-linked files and names
    /// that are not UTF-8.
    pub skipped: u64,
}

/// Volume bookkeeping at the data root, never App data.
const SYSTEM_ENTRIES: [&str; 5] = [
    "lost+found",
    ".fseventsd",
    ".Spotlight-V100",
    ".Trashes",
    ".TemporaryItems",
];
const MAX_DIRECTORY_ENTRIES: usize = 4096;

impl PrivateData {
    /// Copies the data tree into `destination`, an empty kernel-private
    /// directory, fsyncing every file. In-flight SDK replacements are skipped.
    pub fn copy_tree(&self, destination: &Path, limits: TreeLimits) -> Result<TreeCopy> {
        let to = Dir::open_private(destination).map_err(fs)?;
        let from = Dir(self.root.0.try_clone().map_err(io)?);
        copy(&from, &to, limits)
    }

    /// Replaces this installation's App data with a tree `copy_tree` wrote:
    /// everything but volume bookkeeping is removed first.
    pub fn restore_tree(&self, source: &Path, limits: TreeLimits) -> Result<TreeCopy> {
        let from = Dir::open_private(source).map_err(fs)?;
        let to = Dir(self.root.0.try_clone().map_err(io)?);
        let device = to.0.metadata().map_err(io)?.dev();
        clear(&to, 0, device, limits.depth)?;
        copy(&from, &to, limits)
    }
}

fn clear(dir: &Dir, depth: usize, device: u64, max_depth: usize) -> Result<()> {
    if depth > max_depth {
        return Err(PrivateDataError::Invalid);
    }
    for name in dir.entries(MAX_DIRECTORY_ENTRIES).map_err(fs)? {
        if depth == 0
            && SYSTEM_ENTRIES
                .iter()
                .any(|system| OsStr::new(system) == name)
        {
            continue;
        }
        let metadata = private_fs::entry_metadata(dir, &name).map_err(fs)?;
        if metadata.st_mode & libc::S_IFMT == libc::S_IFDIR {
            let child = dir.child(&name).map_err(fs)?;
            if child.0.metadata().map_err(io)?.dev() != device {
                return Err(PrivateDataError::Identity);
            }
            clear(&child, depth + 1, device, max_depth)?;
            dir.remove_directory(&name).map_err(fs)?;
        } else {
            dir.remove_file(&name).map_err(fs)?;
        }
    }
    dir.sync().map_err(fs)
}

fn copy(from: &Dir, to: &Dir, limits: TreeLimits) -> Result<TreeCopy> {
    let device = from.0.metadata().map_err(io)?.dev();
    let mut copied = TreeCopy::default();
    copy_directory(from, to, "", 0, device, limits, &mut copied)?;
    Ok(copied)
}

fn copy_directory(
    from: &Dir,
    to: &Dir,
    prefix: &str,
    depth: usize,
    device: u64,
    limits: TreeLimits,
    copied: &mut TreeCopy,
) -> Result<()> {
    if depth > limits.depth {
        return Err(PrivateDataError::Invalid);
    }
    let mut names = from.entries(MAX_DIRECTORY_ENTRIES).map_err(fs)?;
    names.sort();
    for name in names {
        let Some(text) = name.to_str() else {
            copied.skipped += 1;
            continue;
        };
        if (prefix.is_empty() && SYSTEM_ENTRIES.contains(&text))
            || private_fs::is_atomic_temporary(&name)
            || text.starts_with(".chariox-replace-")
        {
            continue;
        }
        let path = if prefix.is_empty() {
            text.to_owned()
        } else {
            format!("{prefix}/{text}")
        };
        let metadata = private_fs::entry_metadata(from, &name).map_err(fs)?;
        match metadata.st_mode & libc::S_IFMT {
            libc::S_IFDIR => {
                let child = from.child(&name).map_err(fs)?;
                if child.0.metadata().map_err(io)?.dev() != device {
                    return Err(PrivateDataError::Identity);
                }
                let target = to.create_child(&name).map_err(fs)?;
                copy_directory(&child, &target, &path, depth + 1, device, limits, copied)?;
                target.sync().map_err(fs)?;
            }
            libc::S_IFREG if metadata.st_nlink == 1 => {
                if copied.files.len() >= limits.files {
                    return Err(PrivateDataError::Invalid);
                }
                let mut input = from.read_file(&name, false).map_err(fs)?;
                let mut output = to.create_private_file(&name).map_err(fs)?;
                let remaining = limits.bytes - copied.bytes;
                let mut hasher = Sha256::new();
                let mut buffer = vec![0u8; 64 * 1024];
                let mut bytes = 0u64;
                loop {
                    let count = input.read(&mut buffer).map_err(io)?;
                    if count == 0 {
                        break;
                    }
                    bytes += count as u64;
                    if bytes > remaining {
                        return Err(PrivateDataError::Invalid);
                    }
                    hasher.update(&buffer[..count]);
                    output.write_all(&buffer[..count]).map_err(io)?;
                }
                output.sync_all().map_err(io)?;
                copied.bytes += bytes;
                copied.files.push(TreeFile {
                    path,
                    bytes,
                    sha256: format!("{:x}", hasher.finalize()),
                });
            }
            _ => copied.skipped += 1,
        }
    }
    to.sync().map_err(fs)
}
