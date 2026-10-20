use super::*;

/// Protocol 568: one file is at most this large.
pub const MACHINE_FILE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Protocol 568: one read or write request moves at most this many bytes. A write chunk
/// travels base64 inside one relayed request, which the target accepts up to 768 KiB.
pub const MACHINE_FILE_MAX_CHUNK_BYTES: u64 = 256 * 1024;

/// Protocol 568: what a path is on this machine. `path` is absolute or starts
/// with `~/`; a directory listing is `ListMachineDirectory`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatMachineFileRequest {
    pub path: String,
    /// Also hash the file (up to the size cap); a hash is how a transfer is verified.
    #[serde(default)]
    pub with_sha256: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListMachineDirectoryRequest {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadMachineFileRequest {
    pub path: String,
    pub offset: u64,
    /// At most `MACHINE_FILE_MAX_CHUNK_BYTES`.
    pub length: u64,
}

/// One chunk of an upload. Chunks arrive in order; the file appears at `path`
/// only when `offset` + the chunk reaches `total_size` and, when given,
/// `sha256` matches. Until then the bytes live beside it in `<path>.chariox-partial`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteMachineFileRequest {
    pub path: String,
    pub offset: u64,
    pub data_base64: String,
    pub total_size: u64,
    /// Hex sha256 of the whole file.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Replace an existing file at `path` when the upload completes.
    #[serde(default)]
    pub overwrite: bool,
}

impl std::fmt::Debug for WriteMachineFileRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteMachineFileRequest")
            .field("path", &self.path)
            .field("offset", &self.offset)
            .field("encoded_bytes", &self.data_base64.len())
            .field("total_size", &self.total_size)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineFileKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineFileStat {
    /// The absolute path that was looked at.
    pub path: String,
    pub exists: bool,
    pub kind: Option<MachineFileKind>,
    pub size: Option<u64>,
    pub modified_ms: Option<u64>,
    pub sha256: Option<String>,
    /// Bytes of an unfinished upload to `path`: where `WriteMachineFile` continues.
    pub partial_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineDirectoryEntry {
    pub name: String,
    pub kind: MachineFileKind,
    pub size: u64,
    pub modified_ms: Option<u64>,
}
