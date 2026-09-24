use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_UPLOAD_CHUNK_BYTES: usize = 512 * 1024;
pub const MAX_UPLOAD_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAX_UPLOADS: usize = 32;
pub(super) const MAX_RESERVED_BYTES: u64 = 512 * 1024 * 1024;
pub(super) const MAX_TTL_MS: u64 = 30 * 60 * 1_000;
pub(super) const MAX_READERS: usize = 4;
pub(super) const STATE_VERSION: u32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error("app_upload_invalid: {0}")]
    Invalid(&'static str),
    #[error("app_upload_not_found")]
    NotFound,
    #[error("app_upload_busy")]
    Busy,
    #[error("app_upload_unavailable")]
    Unavailable,
    #[error("app_upload_limit")]
    Limit,
    #[error("app_upload_conflict")]
    Conflict,
    #[error("app_upload_incomplete")]
    Incomplete,
    #[error("app_upload_digest_mismatch")]
    DigestMismatch,
    #[error("app_upload_corrupt_state")]
    CorruptState,
    #[error("app_upload_unsafe_entry")]
    UnsafeEntry,
    #[error("app_upload_io: {0}")]
    Io(#[from] std::io::Error),
}

impl From<crate::private_fs::FsError> for UploadError {
    fn from(error: crate::private_fs::FsError) -> Self {
        match error {
            crate::private_fs::FsError::Io(error) => Self::Io(error),
            crate::private_fs::FsError::UnsafeEntry => Self::UnsafeEntry,
            crate::private_fs::FsError::EntryLimit => Self::Limit,
        }
    }
}

pub(super) type Result<T> = std::result::Result<T, UploadError>;

/// Limits cover this store's reserved archive bytes, plus bounded metadata.
/// They are admission accounting, not an OS disk quota against other programs.
#[derive(Debug, Clone, Copy)]
pub struct UploadLimits {
    pub max_uploads: usize,
    pub max_uploads_per_owner: usize,
    pub max_reserved_bytes: u64,
    pub max_reserved_bytes_per_owner: u64,
    pub max_ttl_ms: u64,
}

impl Default for UploadLimits {
    fn default() -> Self {
        Self {
            max_uploads: MAX_UPLOADS,
            max_uploads_per_owner: 8,
            max_reserved_bytes: MAX_RESERVED_BYTES,
            max_reserved_bytes_per_owner: 256 * 1024 * 1024,
            max_ttl_ms: MAX_TTL_MS,
        }
    }
}

impl UploadLimits {
    pub(super) fn validate(self) -> Result<()> {
        if self.max_uploads == 0
            || self.max_uploads > MAX_UPLOADS
            || self.max_uploads_per_owner == 0
            || self.max_uploads_per_owner > self.max_uploads
            || self.max_reserved_bytes == 0
            || self.max_reserved_bytes > MAX_RESERVED_BYTES
            || self.max_reserved_bytes_per_owner == 0
            || self.max_reserved_bytes_per_owner > self.max_reserved_bytes
            || self.max_ttl_ms == 0
            || self.max_ttl_ms > MAX_TTL_MS
        {
            return Err(UploadError::Invalid("limits"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadPhase {
    Receiving,
    Finalized,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadStatus {
    pub handle: String,
    pub phase: UploadPhase,
    pub expected_size: u64,
    pub accepted_bytes: u64,
    pub sha256: String,
    pub expires_at_ms: u64,
}

/// Trusted-kernel interruption points for deterministic crash/retry drills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadCheckpoint {
    ArchiveSynced,
    BeforeStateCommit,
    /// Metadata rename is visible but its directory has not been synced yet.
    StateRenamed,
    StateCommitted,
    BeforeRecoverySync,
    RecoverySynced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub owner: String,
    pub request_id: String,
    pub expected_size: u64,
    pub sha256: String,
    pub accepted_bytes: u64,
    pub expires_at_ms: u64,
    pub phase: UploadPhase,
}

impl Entry {
    pub fn reserved_bytes(&self) -> u64 {
        if self.phase == UploadPhase::Aborted {
            0
        } else {
            self.expected_size
        }
    }
    pub fn status(&self, handle: &str) -> UploadStatus {
        UploadStatus {
            handle: handle.into(),
            phase: self.phase,
            expected_size: self.expected_size,
            accepted_bytes: self.accepted_bytes,
            sha256: self.sha256.clone(),
            expires_at_ms: self.expires_at_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DurableState {
    pub version: u32,
    pub uploads: BTreeMap<String, Entry>,
}

impl Default for DurableState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            uploads: BTreeMap::new(),
        }
    }
}

pub(super) struct StoreState {
    pub durable: DurableState,
    pub leases: BTreeSet<String>,
    pub faulted: bool,
}

pub(super) fn validate_owner(owner: &str) -> Result<()> {
    if owner.trim().is_empty() || owner.len() > 128 || owner.chars().any(char::is_control) {
        return Err(UploadError::Invalid("owner"));
    }
    Ok(())
}

pub(super) fn validate_request_id(request_id: &str) -> Result<()> {
    if request_id.trim().is_empty()
        || request_id.len() > 128
        || request_id.chars().any(char::is_control)
    {
        return Err(UploadError::Invalid("request ID"));
    }
    Ok(())
}

pub(super) fn valid_handle(handle: &str) -> bool {
    handle
        .strip_prefix("upload_")
        .is_some_and(|id| id.len() == 64 && lower_hex(id))
}

pub(super) fn valid_digest(digest: &str) -> bool {
    digest
        .strip_prefix("sha256:")
        .is_some_and(|id| id.len() == 64 && lower_hex(id))
}

fn lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn validate_state(state: &DurableState, limits: UploadLimits) -> Result<()> {
    if state.version != STATE_VERSION || state.uploads.len() > limits.max_uploads {
        return Err(UploadError::CorruptState);
    }
    let mut total = 0_u64;
    let mut owners = BTreeMap::<&str, (usize, u64)>::new();
    let mut requests = BTreeSet::new();
    for (handle, entry) in &state.uploads {
        if !valid_handle(handle)
            || validate_owner(&entry.owner).is_err()
            || validate_request_id(&entry.request_id).is_err()
            || !requests.insert((&entry.owner, &entry.request_id))
            || !valid_digest(&entry.sha256)
            || entry.expected_size == 0
            || entry.expected_size > MAX_UPLOAD_ARCHIVE_BYTES
            || entry.accepted_bytes > entry.expected_size
            || entry.expires_at_ms == 0
            || (entry.phase == UploadPhase::Finalized
                && entry.accepted_bytes != entry.expected_size)
        {
            return Err(UploadError::CorruptState);
        }
        total = total
            .checked_add(entry.reserved_bytes())
            .ok_or(UploadError::CorruptState)?;
        let count = owners.entry(&entry.owner).or_default();
        count.0 += 1;
        count.1 = count
            .1
            .checked_add(entry.reserved_bytes())
            .ok_or(UploadError::CorruptState)?;
        if count.0 > limits.max_uploads_per_owner || count.1 > limits.max_reserved_bytes_per_owner {
            return Err(UploadError::CorruptState);
        }
    }
    if total > limits.max_reserved_bytes {
        return Err(UploadError::CorruptState);
    }
    Ok(())
}
