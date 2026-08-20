use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};

use flate2::{Compression, GzBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;

const DEVELOPMENT_CONTEXT_SCHEMA_VERSION: u32 = 1;
const MAX_REPOSITORIES: usize = 32;
const MAX_OVERLAY_FILES_PER_REPOSITORY: usize = 20_000;
const MAX_OVERLAY_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OVERLAY_BYTES_PER_REPOSITORY: u64 = 256 * 1024 * 1024;
const MAX_BUNDLE_BYTES_PER_REPOSITORY: u64 = 512 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_CONTEXT_IDENTIFIER_BYTES: usize = 4096;
const MAX_TARGET_DIRECTORY_BASE_BYTES: usize = 200;
const MAX_REPOSITORY_TREE_ENTRIES: usize = 500_000;
const MAX_GIT_NUL_RECORD_BYTES: usize = 64 * 1024;
const MAX_CONTEXT_IGNORE_BYTES: u64 = 256 * 1024;
const MAX_CONTEXT_IGNORE_PATTERNS: usize = 1024;
const MAX_CONTEXT_IGNORE_PATTERN_BYTES: usize = 4096;
const MAX_MANIFEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_GIT_TEXT_BYTES: usize = 64 * 1024;
const MAX_GIT_COMMAND_OUTPUT_BYTES: usize = MAX_OVERLAY_FILE_BYTES as usize + 64 * 1024;
const MAX_GIT_ERROR_BYTES: usize = 64 * 1024;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevelopmentRepositoryRole {
    Primary,
    Supporting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevelopmentRepositorySelection {
    pub workspace_id: String,
    pub worktree_path: PathBuf,
    pub role: DevelopmentRepositoryRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevelopmentContextExportRequest {
    pub project_id: String,
    pub repositories: Vec<DevelopmentRepositorySelection>,
    pub archive_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevelopmentContextManifest {
    pub schema_version: u32,
    pub project_id: String,
    pub repositories: Vec<DevelopmentRepositoryManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevelopmentRepositoryManifest {
    pub repository_id: String,
    pub logical_name: String,
    pub role: DevelopmentRepositoryRole,
    pub target_directory: String,
    pub head_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_url: Option<String>,
    pub bundle_path: String,
    pub bundle_sha256: String,
    pub bundle_size_bytes: u64,
    pub overlay: Vec<DevelopmentOverlayEntry>,
    pub overlay_size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevelopmentOverlayEntry {
    pub path: String,
    pub index: DevelopmentFileState,
    pub worktree: DevelopmentFileState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DevelopmentFileState {
    Absent,
    File {
        object_path: String,
        sha256: String,
        size_bytes: u64,
        executable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevelopmentSourceRepositoryMapping {
    pub source_workspace_id: String,
    pub repository_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevelopmentContextExportResult {
    pub manifest: DevelopmentContextManifest,
    pub archive_path: PathBuf,
    pub archive_sha256: String,
    pub archive_size_bytes: u64,
    pub source_repositories: Vec<DevelopmentSourceRepositoryMapping>,
}

struct ManifestMemoryBudget {
    used_bytes: usize,
}

impl ManifestMemoryBudget {
    fn new() -> Self {
        Self { used_bytes: 0 }
    }

    fn consume(&mut self, bytes: usize) -> Result<(), DaemonError> {
        self.used_bytes = self.used_bytes.saturating_add(bytes);
        if self.used_bytes > MAX_MANIFEST_BYTES {
            return Err(context_error(format!(
                "development context manifest metadata exceeds {MAX_MANIFEST_BYTES} bytes"
            )));
        }
        Ok(())
    }
}

mod archive;
mod export;
mod git;
mod overlay;

use archive::write_archive;
pub use export::export_development_context;
#[cfg(test)]
use export::publish_archive_no_clobber;
use git::{
    create_git_bundle, ensure_worktree_root, git_blob_size, git_bytes, git_optional_text,
    git_output, git_text, reject_lfs_attributes, reject_lfs_pointer,
    reject_unsupported_repository_features, split_nul, stream_git_nul_records, verify_git_bundle,
};
use overlay::{export_overlay, validate_relative_path};

fn create_private_directory(path: &Path) -> Result<(), DaemonError> {
    fs::create_dir_all(path)
        .map_err(|error| context_io_error("create private directory", error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| context_io_error("secure private directory", error))?;
    }
    Ok(())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), DaemonError> {
    let mut file = private_create_new(path)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| context_io_error("write private file", error))
}

fn private_create_new(path: &Path) -> Result<File, DaemonError> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| context_io_error("create private file", error))
}

fn create_unique_private_directory(parent: &Path, prefix: &str) -> Result<PathBuf, DaemonError> {
    let timestamp = crate::session::unix_epoch_ms();
    for salt in 0_u64.. {
        let candidate = parent.join(format!("{prefix}-{timestamp}-{salt}"));
        match fs::create_dir(&candidate) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Err(error) =
                        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o700))
                    {
                        let _ = fs::remove_dir(&candidate);
                        return Err(context_io_error("secure private staging directory", error));
                    }
                }
                return Ok(candidate);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(context_io_error("create private staging directory", error)),
        }
    }
    unreachable!("temporary path salt space is unbounded")
}

fn sha256_file(path: &Path) -> Result<String, DaemonError> {
    let mut file =
        File::open(path).map_err(|error| context_io_error("open file for hash", error))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| context_io_error("read file for hash", error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(unix)]
fn file_is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn file_is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

fn context_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "managed development context",
        message: message.into(),
    }
}

fn context_io_error(operation: &'static str, error: io::Error) -> DaemonError {
    context_error(format!("{operation}: {error}"))
}
#[cfg(test)]
mod tests;
