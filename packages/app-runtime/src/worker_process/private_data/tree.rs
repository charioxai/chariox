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
/// As many entries in one directory as files in a whole copy.
const MAX_DIRECTORY_ENTRIES: usize = 10_000;

impl PrivateData {
    /// Copies the data tree into `destination`, an empty kernel-private
    /// directory, fsyncing every file. In-flight SDK replacements and entries
    /// that vanish mid-walk are skipped. `proceed` is asked before each file;
    /// when it answers false the copy stops with `Io`.
    pub fn copy_tree(
        &self,
        destination: &Path,
        limits: TreeLimits,
        proceed: &mut dyn FnMut() -> bool,
    ) -> Result<TreeCopy> {
        let to = Dir::open_private(destination).map_err(fs)?;
        let from = Dir(self.root.0.try_clone().map_err(io)?);
        copy(&from, Some(&to), limits, proceed)
    }

    /// Replaces this installation's App data with a tree `copy_tree` wrote:
    /// the source is walked within `limits` first, then everything but volume
    /// bookkeeping is removed and the source copied in.
    pub fn restore_tree(&self, source: &Path, limits: TreeLimits) -> Result<TreeCopy> {
        let from = Dir::open_private(source).map_err(fs)?;
        copy(&from, None, limits, &mut || true)?;
        let to = Dir(self.root.0.try_clone().map_err(io)?);
        let device = to.0.metadata().map_err(io)?.dev();
        clear(&to, 0, device, limits.depth)?;
        copy(&from, Some(&to), limits, &mut || true)
    }
}

fn clear(dir: &Dir, depth: usize, device: u64, max_depth: usize) -> Result<()> {
    if depth > max_depth {
        return Err(PrivateDataError::Invalid);
    }
    for name in entries(dir)? {
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

fn copy(
    from: &Dir,
    to: Option<&Dir>,
    limits: TreeLimits,
    proceed: &mut dyn FnMut() -> bool,
) -> Result<TreeCopy> {
    let device = from.0.metadata().map_err(io)?.dev();
    let mut copied = TreeCopy::default();
    copy_directory(from, to, "", 0, device, limits, proceed, &mut copied)?;
    Ok(copied)
}

fn entries(dir: &Dir) -> Result<Vec<std::ffi::OsString>> {
    dir.entries(MAX_DIRECTORY_ENTRIES)
        .map_err(|error| match error {
            private_fs::FsError::EntryLimit => PrivateDataError::Invalid,
            other => fs(other),
        })
}

fn vanished(error: &private_fs::FsError) -> bool {
    matches!(error, private_fs::FsError::Io(error) if error.kind() == std::io::ErrorKind::NotFound)
}

/// Walks `from`; with no `to` it only measures the tree against `limits`.
#[allow(clippy::too_many_arguments)]
fn copy_directory(
    from: &Dir,
    to: Option<&Dir>,
    prefix: &str,
    depth: usize,
    device: u64,
    limits: TreeLimits,
    proceed: &mut dyn FnMut() -> bool,
    copied: &mut TreeCopy,
) -> Result<()> {
    if depth > limits.depth {
        return Err(PrivateDataError::Invalid);
    }
    let mut names = entries(from)?;
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
        let metadata = match private_fs::entry_metadata(from, &name) {
            Ok(metadata) => metadata,
            Err(error) if vanished(&error) => continue,
            Err(error) => return Err(fs(error)),
        };
        match metadata.st_mode & libc::S_IFMT {
            libc::S_IFDIR => {
                let child = match from.child(&name) {
                    Ok(child) => child,
                    Err(error) if vanished(&error) => continue,
                    Err(error) => return Err(fs(error)),
                };
                if child.0.metadata().map_err(io)?.dev() != device {
                    return Err(PrivateDataError::Identity);
                }
                let target = match to {
                    Some(to) => Some(to.create_child(&name).map_err(fs)?),
                    None => None,
                };
                copy_directory(
                    &child,
                    target.as_ref(),
                    &path,
                    depth + 1,
                    device,
                    limits,
                    proceed,
                    copied,
                )?;
                if let Some(target) = target {
                    target.sync().map_err(fs)?;
                }
            }
            libc::S_IFREG if metadata.st_nlink == 1 => {
                if copied.files.len() >= limits.files {
                    return Err(PrivateDataError::Invalid);
                }
                if !proceed() {
                    return Err(PrivateDataError::Io);
                }
                let mut input = match from.read_file(&name, false) {
                    Ok(input) => input,
                    Err(error) if vanished(&error) => continue,
                    Err(error) => return Err(fs(error)),
                };
                let mut output = match to {
                    Some(to) => Some(to.create_private_file(&name).map_err(fs)?),
                    None => None,
                };
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
                    if let Some(output) = output.as_mut() {
                        output.write_all(&buffer[..count]).map_err(io)?;
                    }
                }
                if let Some(output) = output {
                    output.sync_all().map_err(io)?;
                }
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
    match to {
        Some(to) => to.sync().map_err(fs),
        None => Ok(()),
    }
}
