//! Where a copy reads and writes: the workspace of an agent, or a whole machine. The slice and
//! resume engine is the same; a place only says which path names which file and what it refuses.
use std::path::{Path, PathBuf};

use super::workspace_path::{resolve, Access};
use super::Workspace;

/// A workspace file moves at most this much in one copy; a place may allow more.
const WORKSPACE_MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

pub(crate) trait Place: Clone + Send + Sync + 'static {
    /// The file `path` names here, with any folder a write needs created, or why it may not be used.
    fn resolve(&self, path: &str, access: Access) -> Result<PathBuf, String>;
    /// Why no file may arrive here, when none may.
    fn accepts_files(&self) -> Result<(), String>;
    fn max_file_bytes(&self) -> u64;
    /// Where the disk budget of a write is measured when its path has no folder.
    fn root(&self) -> &Path;
}

impl Place for Workspace {
    fn resolve(&self, path: &str, access: Access) -> Result<PathBuf, String> {
        resolve(&self.root, path, access)
    }

    fn accepts_files(&self) -> Result<(), String> {
        if self.read_only {
            return Err(
                "The receiving agent is in plan mode and does not take files; switch it to build mode first"
                    .to_owned(),
            );
        }
        Ok(())
    }

    fn max_file_bytes(&self) -> u64 {
        WORKSPACE_MAX_FILE_BYTES
    }

    fn root(&self) -> &Path {
        &self.root
    }
}
