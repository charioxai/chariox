//! `chariox.file.copy`: a file moves byte for byte between a home agent's workspace and the
//! workspace of one of its leased agents on another kernel, over the execution-lease link. The
//! bytes never pass through a model: the caller names two paths and gets back a size and a
//! SHA-256. The file travels in the Environment import's slices, every slice bound to the
//! source's revision, and lands through the Kernel Files staging engine, which publishes it only
//! when the assembled file has the source's digest.
mod transfer;
mod workspace_path;
mod workspace_slices;

#[cfg(test)]
mod tests;

pub(crate) use transfer::{pull, push, Copied, CopyOptions, LinkError, RemoteWorkspace};
pub(crate) use workspace_slices::{
    discard, read_slice, write_slice, SourceSlice, WriteSlice, MAX_FILE_BYTES,
};

use std::path::PathBuf;

/// The workspace of an agent on this kernel, as a copy may use it.
#[derive(Debug, Clone)]
pub(crate) struct Workspace {
    pub root: PathBuf,
    /// A plan-mode agent reads but never receives files.
    pub read_only: bool,
}
