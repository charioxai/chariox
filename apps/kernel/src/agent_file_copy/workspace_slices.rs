//! Reading a workspace file one slice at a time and writing one into a workspace, on whichever
//! kernel holds the workspace. Every answer is final: a refusal says why and what to do.
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::workspace_path::{resolve, Access};
use super::Workspace;
use crate::runtime::kernel_files::{self, StagedChunk, StagedWrite};

/// A slice is the Environment import's: one peer message stays well under the relay's bound.
pub(crate) const SLICE_BYTES: u64 = crate::project_environment::IMPORT_CHUNK_BYTES;
pub(crate) const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Up to one slice of a source file, bound to the version of the file it was read from.
pub(crate) struct SourceSlice {
    pub size_bytes: u64,
    /// The whole file's SHA-256, sent with the first slice; the receiver publishes the file only
    /// when what it assembled has this digest.
    pub sha256: Option<String>,
    /// Changes whenever the file is written or replaced, so a slice of another version is caught
    /// before it is used.
    pub revision: String,
    pub bytes: Zeroizing<Vec<u8>>,
}

/// One slice of a file arriving into a workspace.
pub(crate) struct WriteSlice<'a> {
    pub offset: u64,
    pub total_size: u64,
    pub sha256: &'a str,
    pub overwrite: bool,
    pub bytes: &'a [u8],
}

/// The slice of the file at `relative` that starts at `offset`; the first one carries the
/// file's digest.
pub(crate) fn read_slice(
    workspace: &Workspace,
    relative: &str,
    offset: u64,
) -> Result<SourceSlice, String> {
    let path = resolve(&workspace.root, relative, Access::Read)?;
    let unreadable = || format!("`{relative}` could not be read");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
        .map_err(|_| unreadable())?;
    let metadata = file.metadata().map_err(|_| unreadable())?;
    if !metadata.is_file() {
        return Err(format!("`{relative}` is not a regular file"));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(too_large(relative, metadata.len()));
    }
    let revision = revision(&metadata);
    let sha256 = if offset == 0 {
        Some(kernel_files::sha256_of_open(&file, &path).map_err(|_| unreadable())?)
    } else {
        None
    };
    let length = SLICE_BYTES.min(metadata.len().saturating_sub(offset));
    let mut bytes = Zeroizing::new(vec![0_u8; length as usize]);
    file.read_exact_at(&mut bytes, offset)
        .map_err(|_| unreadable())?;
    let after = file.metadata().map_err(|_| unreadable())?;
    if revision != self::revision(&after) {
        return Err(changed(relative));
    }
    Ok(SourceSlice {
        size_bytes: metadata.len(),
        sha256,
        revision,
        bytes,
    })
}

/// Stages `slice` for `relative` and publishes the file when its last byte lands with the
/// expected digest. A slice that does not continue what is staged is not written: the answer
/// names the offset to continue from, so a sender that lost an answer resumes where it stopped.
pub(crate) fn write_slice(
    workspace: &Workspace,
    relative: &str,
    slice: &WriteSlice<'_>,
) -> Result<StagedWrite, String> {
    if workspace.read_only {
        return Err(
            "The receiving agent is in plan mode and does not take files; switch it to build mode first"
                .to_owned(),
        );
    }
    if slice.total_size > MAX_FILE_BYTES {
        return Err(too_large(relative, slice.total_size));
    }
    let path = resolve(&workspace.root, relative, Access::Write)?;
    let staging = staging_of(&path);
    // The last slice landed and only its answer was lost: the file is already published.
    let published = || StagedWrite {
        received: slice.total_size,
        sha256: Some(slice.sha256.to_owned()),
    };
    if slice.offset == 0 {
        if !slice.overwrite && holds(&path, slice.total_size, slice.sha256) {
            return Ok(published());
        }
        let folder = path.parent().unwrap_or(&workspace.root);
        let budget = crate::managed_context::limits::disk_budget(folder);
        if slice.total_size > budget {
            return Err(crate::managed_context::limits::disk_shortage(
                "The copy", folder, 0, budget,
            ));
        }
    } else {
        let staged = kernel_files::staged_bytes(&staging);
        if staged != slice.offset {
            if staged == 0 && holds(&path, slice.total_size, slice.sha256) {
                return Ok(published());
            }
            return Ok(StagedWrite {
                received: staged,
                sha256: None,
            });
        }
    }
    kernel_files::write_at(
        &path,
        &staging,
        &StagedChunk {
            offset: slice.offset,
            total_size: slice.total_size,
            data: slice.bytes,
            overwrite: slice.overwrite,
            sha256: Some(slice.sha256),
        },
    )
    .map_err(|error| match &error {
        crate::error::DaemonError::MachineRequest { code, .. } if code == "file_hash_mismatch" => {
            changed(relative)
        }
        crate::error::DaemonError::MachineRequest { code, .. } if code == "file_exists" => {
            format!("`{relative}` already exists; copy again with overwrite to replace it")
        }
        _ => format!("`{relative}` could not be written: {error}"),
    })
}

/// Drops what an unfinished copy staged for `relative`.
pub(crate) fn discard(workspace: &Workspace, relative: &str) {
    if let Ok(path) = resolve(&workspace.root, relative, Access::Write) {
        kernel_files::discard_staging(&staging_of(&path));
    }
}

/// Where the copy to `path` stages its bytes: beside the destination, under a `.chariox` name that
/// no copy may name as source or destination, so it never collides with a workspace file and is
/// removed only by the copy that made it.
fn staging_of(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().as_encoded_bytes();
    let digest = Sha256::digest(name);
    let id: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    path.with_file_name(format!(".chariox-copy-{id}"))
}

/// Whether `path` is a regular file of `size` bytes with digest `sha256`.
fn holds(path: &Path, size: u64, sha256: &str) -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()
        .filter(|file| {
            file.metadata()
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() == size)
        })
        .and_then(|file| kernel_files::sha256_of_open(&file, path).ok())
        .is_some_and(|digest| digest == sha256)
}

fn revision(metadata: &std::fs::Metadata) -> String {
    format!(
        "{}:{}:{}:{}.{}:{}.{}",
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec()
    )
}

fn too_large(relative: &str, size: u64) -> String {
    format!(
        "`{relative}` has {size} bytes; a copy moves at most {} MiB (split it, for example with `split -b`)",
        MAX_FILE_BYTES / (1024 * 1024)
    )
}

pub(crate) fn changed(relative: &str) -> String {
    format!("`{relative}` changed while it was copied; nothing was written. Copy it again once it stops changing")
}
