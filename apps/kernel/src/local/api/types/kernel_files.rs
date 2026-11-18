use super::*;

/// Protocol 568: one file is at most this large.
pub const KERNEL_FILE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Protocol 568: one read or write request moves at most this many bytes. One reply is one
/// indivisible relay message, about 117 KB on the wire at this size, so it never holds the link
/// for long. Smaller than this a reply would ride the priority lane, which the per-client bound on
/// bulk replies does not cover; the CLI downloads with several of these in flight.
pub const KERNEL_FILE_MAX_CHUNK_BYTES: u64 = 64 * 1024;

/// Protocol 568: what a path is on this machine. `path` is absolute or starts
/// with `~/`; a directory listing is `ListKernelDirectory`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatKernelFileRequest {
    pub path: String,
    /// Also hash the file (up to the size cap); a hash is how a transfer is verified.
    #[serde(default)]
    pub with_sha256: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListKernelDirectoryRequest {
    pub path: String,
    /// Protocol 1005: entries (sorted by name) to skip; a listing that says `truncated` continues
    /// at `offset` + its entries.
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadKernelFileRequest {
    pub path: String,
    pub offset: u64,
    /// At most `KERNEL_FILE_MAX_CHUNK_BYTES`.
    pub length: u64,
}

/// One chunk of an upload. Chunks arrive in order; the file appears at `path`
/// only when `offset` + the chunk reaches `total_size` and, when given,
/// `sha256` matches. Until then the bytes live beside it in `<path>.chariox-partial`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteKernelFileRequest {
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

impl std::fmt::Debug for WriteKernelFileRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteKernelFileRequest")
            .field("path", &self.path)
            .field("offset", &self.offset)
            .field("encoded_bytes", &self.data_base64.len())
            .field("total_size", &self.total_size)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelFileKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelFileStat {
    /// The absolute path that was looked at.
    pub path: String,
    pub exists: bool,
    pub kind: Option<KernelFileKind>,
    pub size: Option<u64>,
    pub modified_ms: Option<u64>,
    /// Protocol 1005: changes whenever the file is written or replaced.
    #[serde(default)]
    pub revision: Option<String>,
    /// Protocol 1005: the Unix permission bits.
    #[serde(default)]
    pub mode: Option<u32>,
    pub sha256: Option<String>,
    /// Bytes of an unfinished upload to `path`: where `WriteKernelFile` continues.
    pub partial_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelDirectoryEntry {
    pub name: String,
    /// What is at the name itself: a link is `symlink`, which transfers skip.
    pub kind: KernelFileKind,
    /// Protocol 1011: what a link points to, so a window can open it as that; absent for anything that is
    /// not a link and for a link that points to nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<KernelFileKind>,
    pub size: u64,
    pub modified_ms: Option<u64>,
    /// Protocol 1005: changes whenever the file is written or replaced.
    pub revision: String,
    /// Protocol 1005: the Unix permission bits.
    pub mode: u32,
}

/// Protocol 1032: move these files and folders to the machine's own trash. Each is a path as in
/// `StatKernelFileRequest`; nothing is deleted for good.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashKernelFilesRequest {
    pub paths: Vec<String>,
}

/// Protocol 1032: an item in the machine's trash and where it came from; `RestoreKernelTrash` puts it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashedKernelFile {
    /// The absolute path the item had.
    pub path: String,
    /// The absolute path of the item inside the trash.
    pub trash_path: String,
}

/// Protocol 1032: put trashed items back at their original paths. An item whose path is taken stays in
/// the trash and is reported as failed; nothing is ever overwritten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreKernelTrashRequest {
    pub items: Vec<TrashedKernelFile>,
}

/// Protocol 1032: one item of a trash or restore request that could not be done, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelFileFailure {
    pub path: String,
    pub message: String,
}
