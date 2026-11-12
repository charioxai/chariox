use super::*;

/// Protocol 1005: how a transfer treats a name that is already taken at the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTransferConflict {
    /// The transfer ends with an error that names the choices.
    #[default]
    Fail,
    /// The existing file or folder is replaced when the new one is complete.
    Replace,
    /// The new one lands beside it as `name (2)`, `name (3)`, ...
    KeepBoth,
    /// Nothing is copied and the transfer ends as done.
    Skip,
}

/// A file or folder on one of the owner's machines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileTransferEndpoint {
    /// The kernel (id, name or machine name) that holds the path; this kernel when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel: Option<String>,
    /// Absolute, or `~/`.
    pub path: String,
}

/// Protocol 1005: copy or move a file or folder from one machine to a folder of another. The
/// kernel that receives the request is the destination: it pulls the bytes from the source over
/// the relay, so no client carries them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartFileTransferRequest {
    pub source: FileTransferEndpoint,
    /// The folder the item lands in, on this kernel.
    pub destination_folder: String,
    /// The item's name at the destination; the source's own name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub conflict: FileTransferConflict,
    /// Delete the source once the destination holds a verified copy.
    #[serde(default, rename = "move")]
    pub delete_source: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileTransferRef {
    pub job_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTransferState {
    /// Reading the source's folders to know what there is to copy.
    Scanning,
    Copying,
    /// Copied and verified; the source is being deleted (a move).
    Finishing,
    Done,
    Failed,
    Canceled,
}

impl FileTransferState {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Canceled)
    }
}

/// One transfer, as it stands. The kernel keeps it after it ends so a client that was away sees
/// how it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTransferJob {
    pub job_id: String,
    pub state: FileTransferState,
    pub source: FileTransferEndpoint,
    /// Where the item lands, once settled (a kept-both name included); empty before.
    pub destination: String,
    pub delete_source: bool,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: u64,
    pub files_total: u64,
    /// Entries left out: a name taken under `skip`, links and special files.
    pub skipped: u64,
    /// The file being copied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    /// The SHA-256 of a single file's copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Why the transfer ended without a copy, and what to do next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<u64>,
}
