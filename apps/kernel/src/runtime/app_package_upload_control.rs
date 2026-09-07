//! Kernel-owned package transport, below every terminal and relay adapter.
//! Uploaded bytes are untrusted: this service neither verifies publishers nor
//! activates Apps. Inspection will consume a finalized descriptor separately.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, TryLockError};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chariox_app_runtime::package_upload::{
    PackageUploadStore, UploadError, UploadLimits, UploadStatus, MAX_UPLOAD_ARCHIVE_BYTES,
    MAX_UPLOAD_CHUNK_BYTES,
};
use tokio::sync::OwnedSemaphorePermit;

#[cfg(test)]
mod tests;

pub(crate) const MAX_ENCODED_UPLOAD_CHUNK_BYTES: usize = MAX_UPLOAD_CHUNK_BYTES.div_ceil(3) * 4;
const UPLOAD_TTL_MS: u64 = 30 * 60 * 1_000;

/// The owner is deliberately absent: AppControl derives it from KernelCaller.
pub(crate) enum UploadCommand {
    Begin {
        request_id: String,
        expected_size: u64,
        sha256: String,
    },
    Chunk {
        handle: String,
        offset: u64,
        data_base64: String,
        chunk_sha256: String,
    },
    Status {
        handle: String,
    },
    Abort {
        handle: String,
    },
}

/// Stable, safe classifications. No raw I/O text or host paths cross the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UploadControlError {
    InvalidRequest,
    NotFound,
    Busy,
    LimitExceeded,
    Conflict,
    DigestMismatch,
    StorageUnavailable,
}

#[derive(Clone)]
pub(crate) struct AppPackageUploadControl {
    inner: Arc<Inner>,
}

struct Inner {
    database_path: PathBuf,
    // The live store owns its directory lock across requests and service clones.
    store: Mutex<Option<PackageUploadStore>>,
}

impl AppPackageUploadControl {
    /// Must be the existing DurableKernelStateStore::path(), never a request
    /// field, workspace path or process-current-directory fallback. No I/O here.
    pub(crate) fn new(database_path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                database_path,
                store: Mutex::new(None),
            }),
        }
    }

    /// Uses AppControl's shared admission permit. The blocking operation retains
    /// it even when the awaiting client disconnects; there is no second queue.
    pub(crate) async fn execute(
        &self,
        trusted_owner: String,
        command: UploadCommand,
        permit: OwnedSemaphorePermit,
    ) -> Result<UploadStatus, UploadControlError> {
        let service = self.clone();
        admitted_blocking(permit, move || {
            service.execute_at(&trusted_owner, command, crate::session::unix_epoch_ms())
        })
        .await
    }

    fn execute_at(
        &self,
        owner: &str,
        command: UploadCommand,
        now_ms: u64,
    ) -> Result<UploadStatus, UploadControlError> {
        if !valid_identity(owner) {
            return Err(UploadControlError::InvalidRequest);
        }
        match command {
            UploadCommand::Begin {
                request_id,
                expected_size,
                sha256,
            } => {
                if !valid_identity(&request_id)
                    || expected_size == 0
                    || expected_size > MAX_UPLOAD_ARCHIVE_BYTES
                    || !valid_digest(&sha256)
                {
                    return Err(UploadControlError::InvalidRequest);
                }
                let expires_at_ms = now_ms
                    .checked_add(UPLOAD_TTL_MS)
                    .ok_or(UploadControlError::StorageUnavailable)?;
                self.store(now_ms)?
                    .begin(
                        owner,
                        &request_id,
                        expected_size,
                        &sha256,
                        expires_at_ms,
                        now_ms,
                    )
                    .map_err(upload_error)
            }
            UploadCommand::Chunk {
                handle,
                offset,
                data_base64,
                chunk_sha256,
            } => {
                // Reject the encoded allocation bound before decoding or opening
                // the store. The wire frame has its own transport-size bound.
                let bytes = decode_chunk(&data_base64)?;
                validate_handle(&handle)?;
                if offset > MAX_UPLOAD_ARCHIVE_BYTES || !valid_digest(&chunk_sha256) {
                    return Err(UploadControlError::InvalidRequest);
                }
                self.store(now_ms)?
                    .chunk(owner, &handle, offset, &bytes, &chunk_sha256, now_ms)
                    .map_err(upload_error)
            }
            UploadCommand::Status { handle } => {
                validate_handle(&handle)?;
                self.store(now_ms)?
                    .status(owner, &handle, now_ms)
                    .map_err(upload_error)
            }
            UploadCommand::Abort { handle } => {
                validate_handle(&handle)?;
                self.store(now_ms)?
                    .abort(owner, &handle, now_ms)
                    .map_err(upload_error)
            }
        }
    }

    fn store(&self, now_ms: u64) -> Result<PackageUploadStore, UploadControlError> {
        let mut store = self.inner.store.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => UploadControlError::Busy,
            TryLockError::Poisoned(_) => UploadControlError::StorageUnavailable,
        })?;
        if let Some(store) = store.as_ref() {
            return Ok(store.clone());
        }
        let opened = PackageUploadStore::open_or_create(
            &self.inner.database_path,
            UploadLimits::default(),
            now_ms,
        )
        .map_err(|error| match error {
            UploadError::Busy => UploadControlError::Busy,
            // Root/configuration/recovery failures are never blamed on a caller.
            _ => UploadControlError::StorageUnavailable,
        })?;
        *store = Some(opened.clone());
        Ok(opened)
    }
}

async fn admitted_blocking<T: Send + 'static>(
    permit: OwnedSemaphorePermit,
    operation: impl FnOnce() -> Result<T, UploadControlError> + Send + 'static,
) -> Result<T, UploadControlError> {
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation()
    })
    .await
    .map_err(|_| UploadControlError::StorageUnavailable)?
}

fn valid_identity(value: &str) -> bool {
    value.len() <= 128 && !value.trim().is_empty() && !value.chars().any(char::is_control)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn validate_handle(value: &str) -> Result<(), UploadControlError> {
    if value.len() == 71
        && value.starts_with("upload_")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        Ok(())
    } else {
        Err(UploadControlError::NotFound)
    }
}

fn decode_chunk(encoded: &str) -> Result<Vec<u8>, UploadControlError> {
    if encoded.len() > MAX_ENCODED_UPLOAD_CHUNK_BYTES {
        return Err(UploadControlError::LimitExceeded);
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| UploadControlError::InvalidRequest)?;
    if bytes.is_empty() {
        return Err(UploadControlError::InvalidRequest);
    }
    if bytes.len() > MAX_UPLOAD_CHUNK_BYTES {
        return Err(UploadControlError::LimitExceeded);
    }
    Ok(bytes)
}

fn upload_error(error: UploadError) -> UploadControlError {
    match error {
        UploadError::Invalid(_) => UploadControlError::InvalidRequest,
        UploadError::NotFound => UploadControlError::NotFound,
        UploadError::Busy => UploadControlError::Busy,
        UploadError::Limit => UploadControlError::LimitExceeded,
        UploadError::Conflict | UploadError::Incomplete => UploadControlError::Conflict,
        UploadError::DigestMismatch => UploadControlError::DigestMismatch,
        UploadError::Unavailable
        | UploadError::CorruptState
        | UploadError::UnsafeEntry
        | UploadError::Io(_) => UploadControlError::StorageUnavailable,
    }
}
